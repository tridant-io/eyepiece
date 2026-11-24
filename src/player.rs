//! the player: the render thread, which owns the gpu and the clips, takes events from the window, and draws. while a
//! clip plays, each turn of the loop is one display refresh. while it stands still, the thread sleeps until
//! something happens. nothing here knows the system it runs on: the window, the device, the painter of a clip's
//! frames and of the controls are `crate::sys`, one module for each system with the same things in it.

use crate::sys::window::{self, Handle};
use crate::pictures;
use crate::sys::{Gpu, Paint, UiPainter};
use crate::audio::{Audio, Speaker};
use crate::browse;
use crate::event::{Event, Key, Mods};
use crate::folder;
use crate::markup::{Marks, Placing};
use crate::media::clip::{Clip, Lane};
use crate::media::hap::Tex;
use crate::media::meta::Meta;
use crate::media::watch::{Change, Watch};
use crate::media::still::Samples;
use crate::play::{Show, Transport};
use crate::preview::spare::{Answer, Ask, Seam};
use crate::settings::{self, Settings};
use crate::shot;
use crate::ui::{self, Action, Extras, Facts, FolderList, ListItem, Model, Screen, Shot, Ui};
use crate::view::{Channel, Framing, Guides, Look};
use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// a clip on screen.
pub(crate) struct View {
    pub(crate) clip: Clip,
    pub(crate) transport: Transport,
    /// its shader, and the frame that is in it
    paint: Paint,
    /// the frame that is in the painter now
    shown: Option<usize>,
    /// why the current frame cannot be shown, if it cannot
    pub(crate) fault: Option<String>,
    /// refreshes, while playing, on which no new frame was shown when one was due: the frame was not ready, or the
    /// display missed it. the clip runs that much late; no frame is skipped to make it up
    late: u64,
    /// frames that could not be shown since the clip was opened
    unshowable: u64,
    /// where the picture sits in the window: fitted, or zoomed and panned
    framing: Framing,
    /// how it is turned for the screen
    look: Look,
    /// the last pixel read back for the zoom chip: the frame, the pixel, its red, green and blue
    probe: Option<(usize, (usize, usize), [u32; 3])>,
    /// the clip's sound: None until it has been looked for, then what was found (a clip without is Some(None))
    sound: Option<Option<Audio>>,
    /// what the sound was last told: the transport's run, the speed and whether it goes round with the clip, while
    /// it plays; None while it is silent
    sounding: Option<(u32, f64, bool)>,
    /// times the picture was moved to where the sound is
    resynced: u64,
    /// the spare reader has been asked how the clip's last frame and its first compare, and what it said
    seam_asked: bool,
    seam_change: Option<Seam>,
}

impl View {
    pub(crate) fn new(gpu: &Gpu, mut clip: Clip, hz: f64) -> Result<View, String> {
        // a clip ffmpeg decodes starts here, now that there is a device for a hardware decoder to write into
        clip.start(Some(gpu.hardware()))?;
        let paint = Paint::new(gpu, &clip)?;
        let mut transport = Transport::new(clip.timeline.frames(), clip.timeline.fps(), hz).with_starts(clip.timeline.starts());
        // a sequence opened by one of its files opens on that file's frame
        transport.go(clip.first_frame as i64);
        let framing = Framing::new((clip.width, clip.height));
        Ok(View { clip, transport, paint, shown: None, fault: None, late: 0, unshowable: 0, framing, look: Look::default(), probe: None, sound: None, sounding: None, resynced: 0, seam_asked: false, seam_change: None })
    }

    /// looks for the clip's sound, once. stills have none. returns why, when there is sound that cannot be played.
    fn open_sound(&mut self, path: &str) -> Option<String> {
        self.sound = Some(None);
        if matches!(self.clip.lane, Lane::Stills(_)) {
            return None;
        }
        match Audio::open(path) {
            Ok(Some(audio)) => {
                self.sound = Some(Some(audio));
                None
            }
            Ok(None) => None,
            Err(why) => Some(why.strip_prefix(&format!("{path}: ")).unwrap_or(&why).to_string()),
        }
    }

    /// keeps the clip's sound with its transport: it plays from the transport's frame whenever a run of forward play
    /// starts, and is silent the rest of the time and whenever the clip is not `heard` (another tab is up, or the
    /// browser is over it). while both run, play keeps to the sound. true while the picture should wait for the
    /// sound to start.
    fn sync_sound(&mut self, heard: bool, speaker: &mut Speaker) -> bool {
        let Some(Some(audio)) = &mut self.sound else { return false };
        let t = &mut self.transport;
        // L pressed once is forward at 1×, whatever the speed is set to
        let speed = match t.shuttle_rate() {
            0 => Some(t.speed()),
            1 => Some(1.0),
            _ => None,
        };
        // a clip that loops takes its sound round with it, with no new start at the seam
        let round = t.loop_mode() == crate::play::Loop::One;
        let wanted = speed.filter(|_| heard && t.playing() && t.seam().is_none()).map(|speed| (t.run(), speed, round));
        if wanted != self.sounding {
            match wanted {
                Some((_, speed, round)) => audio.play(speaker, t.seconds(), speed, round.then(|| t.length())),
                None => audio.pause(),
            }
            self.sounding = wanted;
        }
        if wanted.is_none() {
            return false;
        }
        if audio.starting() {
            return true;
        }
        if let Some(seconds) = audio.position() {
            self.resynced += t.follow(t.frame_at(seconds)) as u64;
        }
        false
    }

    /// lets the clip go on a thread of its own: its readers finish the frames they are on first, which took up to
    /// half a second for a 4K float sequence, and the picture must not wait for that.
    fn retire(self) {
        let View { clip, .. } = self;
        std::thread::spawn(move || drop(clip));
    }

    /// puts the transport's frame into the painter, waiting up to `patience` for it, with the frames play or a
    /// step would show next read ahead. true when the picture changed.
    fn load(&mut self, gpu: &Gpu, patience: Duration) -> bool {
        let want = self.transport.ahead();
        self.load_wanting(gpu, patience, &want)
    }

    /// `load`, with the reader told to keep just `want` ready: the transport's frame first.
    pub(crate) fn load_wanting(&mut self, gpu: &Gpu, patience: Duration, want: &[usize]) -> bool {
        let frame = self.transport.frame();
        match &self.clip.lane {
            Lane::Hap(hap) => hap.source.want(want),
            Lane::Video(video) => video.source.want(want),
            Lane::Stills(stills) => stills.source.want(want),
            Lane::Waiting(_) => {}
        }
        if self.shown == Some(frame) {
            return false;
        }
        // the frame, into the painter. Ok(false) while it is still on its way
        let loaded = self.paint.put(gpu, &self.clip, frame, patience);
        match loaded {
            Ok(false) => false,
            result => {
                self.fault = result.err();
                self.unshowable += self.fault.is_some() as u64;
                self.shown = Some(frame);
                true
            }
        }
    }

    /// true while the frame the transport is on has not reached the painter.
    pub(crate) fn waiting(&self) -> bool {
        self.shown != Some(self.transport.frame())
    }

    /// true when there is a frame to draw.
    fn drawable(&self) -> bool {
        self.fault.is_none() && self.paint.ready()
    }

    fn size(&self) -> (usize, usize) {
        (self.clip.width, self.clip.height)
    }

    /// the picture, turned as the look has it, where the framing puts it. nothing during the pause on black.
    fn draw(&self, gpu: &Gpu) {
        let (true, Show::Frame(_)) = (self.drawable(), self.transport.show()) else { return };
        let (to_source, to_source_add) = self.look.to_source(self.size());
        let channel = match self.look.channel {
            Channel::Rgb => 0.0,
            Channel::Red => 1.0,
            Channel::Green => 2.0,
            Channel::Blue => 3.0,
            Channel::Alpha => 4.0,
        };
        let shown = self.look.shown(self.size());
        self.paint.draw(gpu, self.framing.rect((gpu.w as f32, gpu.h as f32)), (shown.0 as f32, shown.1 as f32), Some((to_source, to_source_add, channel)));
    }

    /// the frame in the painter, drawn `w` x `h` into a picture of our own and read back (`Gpu::read_pixels`): the
    /// file's own frame as it is stored, whatever the window shows.
    pub(crate) fn read(&self, gpu: &Gpu, w: u32, h: u32) -> Option<Vec<u16>> {
        self.picture(gpu, (w, h), Some((0.0, 0.0, w as f32, h as f32)), |_| {})
    }

    /// the frame in the painter drawn into a picture of our own `size` big and read back, with `over` painted after
    /// it: the file's own frame as it is stored, at `at`; or with no place, the window's picture as it is shown (zoom,
    /// pan and look) without the controls.
    fn picture(&self, gpu: &Gpu, size: (u32, u32), at: Option<(f32, f32, f32, f32)>, over: impl FnOnce(&Gpu)) -> Option<Vec<u16>> {
        if !self.drawable() {
            return None;
        }
        let file = (self.clip.width as f32, self.clip.height as f32);
        let _lock = gpu.locked();
        gpu.read_pixels(size.0, size.1, |gpu| {
            match at {
                Some(rect) => self.paint.draw(gpu, rect, file, None),
                None => self.draw(gpu),
            }
            over(gpu);
        })
        .ok()
    }

    /// whether the frame in the painter holds the file's alpha: hap alpha and hap r do, stills (whose alpha is 1 when
    /// the file has none), and y'cbcr pictures with a plane of alpha (prores 4444). other y'cbcr frames are drawn
    /// opaque, and so is hap q alpha; hap's bc1 can decode a block to clear black, and hap's alpha alone is drawn as
    /// gray: neither is the file's alpha.
    fn carries_alpha(&self) -> bool {
        match &self.clip.lane {
            Lane::Hap(hap) => matches!(hap.tex, Tex::Bc3 | Tex::Bc7),
            Lane::Stills(_) => true,
            Lane::Video(video) => video.layout.alpha,
            Lane::Waiting(_) => false,
        }
    }

    /// how chroma planes smaller than the picture are brought up to it, as the settings have it.
    fn set_chroma(&mut self, chroma: settings::Chroma) {
        self.paint.set_bilinear(chroma == settings::Chroma::Bilinear);
    }

    /// that, in the metadata panel's words, and whether where the samples sit is the usual place and not the file's
    /// word. none for a picture whose chroma is as big as its luma, or that has none.
    fn chroma(&self, settings: &Settings) -> Option<(String, bool)> {
        let Lane::Video(video) = &self.clip.lane else { return None };
        let layout = video.layout;
        if layout.chroma_shift == (0, 0) {
            return None;
        }
        if settings.chroma == settings::Chroma::Nearest {
            return Some(("the nearest sample".to_string(), false));
        }
        let across = if layout.siting.0 == 0.0 { "left" } else { "center" };
        let down = match layout.siting.1 {
            d if layout.chroma_shift.1 == 0 || d == 0.5 => "",
            d if d == 0.0 => "top ",
            _ => "bottom ",
        };
        Some((format!("blended, sited {down}{across}"), layout.siting_assumed))
    }

    /// how the clip is decoded, in the metadata panel's words.
    fn decoded(&self) -> &'static str {
        match &self.clip.lane {
            Lane::Hap(_) => "blocks to the gpu as stored",
            Lane::Video(video) if video.layout.hardware => "hardware, drawn from the decoder's own surface",
            Lane::Video(_) | Lane::Stills(_) | Lane::Waiting(_) => "software",
        }
    }

    /// bits a sample: the top of the scale pixel values are given on.
    fn depth(&self) -> u32 {
        match &self.clip.lane {
            Lane::Hap(_) | Lane::Waiting(_) => 8,
            // exr's linear light is given on a scale of 16 bits
            Lane::Stills(stills) if stills.samples == Samples::F32 => 16,
            Lane::Stills(stills) => stills.bits,
            Lane::Video(video) => video.layout.stored.bits,
        }
    }

    /// reads the pixel back from the frame in the painter, unless that is the one already read: the picture is drawn
    /// at its own size into one pixel of our own, never read from what the window shows. the numbers are the red,
    /// green and blue the screen is sent, on the scale of the file's bit depth. true when there is a new value.
    fn probe(&mut self, gpu: &Gpu, pixel: Option<(usize, usize)>) -> bool {
        let wanted = self.shown.zip(pixel);
        if wanted == self.probe.map(|(frame, pixel, _)| (frame, pixel)) {
            return false;
        }
        self.probe = None;
        let (Some((frame, pixel)), true) = (wanted, self.drawable()) else { return false };
        let _lock = gpu.locked();
        let size = (self.clip.width as f32, self.clip.height as f32);
        let read = gpu.read_pixels(1, 1, |gpu| self.paint.draw(gpu, (-(pixel.0 as f32), -(pixel.1 as f32), size.0, size.1), size, None));
        let Ok(read) = read else { return false };
        let top = ((1u32 << self.depth()) - 1) as f32;
        self.probe = Some((frame, pixel, [0, 1, 2].map(|i| (read[i] as f32 / 65535.0 * top).round() as u32)));
        true
    }

    /// turns the picture another way. it then starts fitted again (prototype `orient`); one channel alone leaves
    /// the zoom as it is.
    fn set_look(&mut self, look: Look) {
        if (look.mirror, look.flip, look.turns) != (self.look.mirror, self.look.flip, self.look.turns) {
            self.framing = Framing::new(look.shown(self.size()));
        }
        self.look = look;
    }

    /// a key, for the clip. the place is the pointer's when it is over the picture. true when the key changed the view.
    fn key(&mut self, key: Key, mods: Mods, at: Option<(f32, f32)>, window: (f32, f32)) -> bool {
        let arrow = matches!(key, Key::Left | Key::Right | Key::Up | Key::Down);
        // control and alt make another command of a key; the ones built so far are alt with the arrows
        if mods.ctrl || (mods.alt && !arrow) {
            return false;
        }
        let last = self.clip.timeline.frames() as i64 - 1;
        match (key, mods.shift) {
            // an eighth of the window a press
            _ if mods.alt => {
                let (x, y) = match key {
                    Key::Left => (1.0, 0.0),
                    Key::Right => (-1.0, 0.0),
                    Key::Up => (0.0, 1.0),
                    _ => (0.0, -1.0),
                };
                self.framing.pan((x * window.0 / 8.0, y * window.1 / 8.0), window);
                return true;
            }
            (Key::Space, _) => self.transport.set_playing(!self.transport.playing()),
            (Key::F, false) => self.transport.step(-1),
            (Key::G, false) => self.transport.step(1),
            (Key::Left, false) => self.transport.step(-1),
            (Key::Right, false) => self.transport.step(1),
            (Key::Left, true) => self.transport.step(-10),
            (Key::Right, true) => self.transport.step(10),
            (Key::F, true) | (Key::Home, _) => self.transport.go(0),
            (Key::G, true) | (Key::End, _) => self.transport.go(last),
            (Key::J, _) => self.transport.shuttle(-1),
            (Key::K, _) => self.transport.shuttle_stop(),
            (Key::L, _) => self.transport.shuttle(1),
            // - and = zoom out and in one stop, about the pointer (the owner's keys); with shift they step the speed
            (Key::Minus | Key::Equals, false) => {
                self.framing.step(if key == Key::Equals { 1 } else { -1 }, at, window);
                return true;
            }
            (Key::Minus, true) => self.transport.speed_step(-1),
            (Key::Equals, true) => self.transport.speed_step(1),
            (Key::Backspace, _) => self.transport.speed_reset(),
            (Key::H, _) => self.set_look(Look { mirror: !self.look.mirror, ..self.look }),
            (Key::V, _) => self.set_look(Look { flip: !self.look.flip, ..self.look }),
            (Key::Q, _) => self.set_look(Look { turns: (self.look.turns + 1) % 4, ..self.look }),
            (Key::Digit0 | Key::Digit1 | Key::Digit2, _) => {
                let zoom = match key {
                    Key::Digit0 => None,
                    Key::Digit1 => Some(1.0),
                    _ => Some(2.0),
                };
                self.framing.set_zoom(zoom, at, window);
                return true;
            }
            _ => {}
        }
        false
    }

    /// what the controls show. the pixel is the file's pixel under the pointer, when the zoom chip is to name it.
    fn model<'a>(&'a self, window: (f32, f32), pixel: Option<(usize, usize)>, settings: &Settings, facts: Facts<'a>, shots_to: &'a str, (looping, folder): (settings::Loop, bool)) -> Model<'a> {
        Model {
            facts,
            decoded: self.decoded(),
            chroma: self.chroma(settings),
            shots_to,
            sound: matches!(self.sound, Some(Some(_))),
            volume: settings.volume,
            muted: settings.mute,
            frame: self.transport.frame(),
            frames: self.clip.timeline.frames(),
            playing: self.transport.playing(),
            rate: self.clip.timeline.rate(),
            clock: self.clip.clock,
            first: settings.first_frame,
            fault: self.fault.as_deref(),
            gaps: &self.clip.missing,
            linear: matches!(&self.clip.lane, Lane::Stills(stills) if stills.samples == Samples::F32),
            looping,
            folder,
            seam_change: self.seam_change,
            seam: self.transport.seam_place(),
            seam_seconds: settings.loop_analysis_seconds as f64,
            speed: self.transport.speed(),
            shuttle: self.transport.shuttle_rate(),
            zoom_label: self.framing.label(window),
            zoom: self.framing.scale(window),
            zoomed: self.framing.zoomed(),
            exact: self.framing.exact(window),
            overflows: self.framing.overflows(window),
            pixel,
            value: self.probe.filter(|(_, at, _)| Some(*at) == pixel).map(|(.., value)| value),
            picture: self.framing.rect(window),
            shown_size: self.look.shown(self.size()),
            look: self.look,
            guides: guides(settings),
            extras: Extras { caption: settings.screenshot_caption, guides: settings.screenshot_guides, markup: settings.screenshot_markup },
            // the tab's, not the clip's: `play` fills them in
            marks: None,
        }
    }

    /// does what the controls asked of the clip. the tabs' and the window's own are not for it.
    fn act(&mut self, action: &Action, window: (f32, f32), settings: &mut Settings) {
        match action {
            Action::TogglePlay => self.transport.set_playing(!self.transport.playing()),
            Action::Step(by) => self.transport.step(*by),
            Action::Seek(frame) => self.transport.go(*frame as i64),
            Action::SeekPlace(place) => self.transport.go_place(*place),
            Action::Seam(seconds) => {
                self.transport.set_seam(*seconds);
                // the transport holds the seconds to what it takes: keep what it kept
                if let Some(kept) = self.transport.seam() {
                    settings.loop_analysis_seconds = kept as f32;
                }
            }
            Action::Speed(speed) => self.transport.set_speed(*speed),
            Action::Zoom(zoom) => self.framing.set_zoom(*zoom, None, window),
            Action::Pan(by) => self.framing.pan(*by, window),
            Action::Look(look) => self.set_look(*look),
            Action::Key(..) | Action::GoClip(_) | Action::NewTab | Action::SelectTab(_) | Action::CloseTab(_) | Action::Open(_) | Action::OpenTab(_) | Action::Reveal(_) | Action::PlayFolder(_) | Action::Loop(_) | Action::Browse(_) | Action::Shot(_) | Action::Copy(_) | Action::Volume(_) | Action::Mute | Action::Guides(_) | Action::Fullscreen | Action::MoveWindow | Action::Minimize | Action::ToggleMaximize | Action::Close => {}
            Action::Extras(_) | Action::Mark(..) | Action::ClearMarks => {}
        }
    }

    /// what the window's title says: the taskbar and the tests read it.
    /// `tab` is what the tab has to say: its loop mode when that is not as it opened, and where it is in a folder.
    fn title(&self, window: (f32, f32), settings: &Settings, tab: (&str, &str)) -> String {
        let c = &self.clip;
        // the panel will have to admit whether the gpu or the cpu decodes; until then the title does
        let decoded = match &c.lane {
            Lane::Video(video) if video.layout.hardware => ", hardware",
            Lane::Video(_) => ", software",
            Lane::Hap(_) | Lane::Stills(_) | Lane::Waiting(_) => "",
        };
        // what went wrong stays in the title: late refreshes, frames that could not be shown, and the one on screen
        let mut state = String::new();
        if self.late > 0 {
            state += &format!(" · {} late", self.late);
        }
        if self.resynced > 0 {
            state += &format!(" · {} resynced", self.resynced);
        }
        if self.unshowable > 0 {
            state += &format!(" · {} could not be shown", self.unshowable);
        }
        if let Some(why) = &self.fault {
            state += &format!(" · this frame cannot be shown: {why}");
        }
        // the number is where the transport is; until that frame is in the painter the picture is still the old one
        if self.waiting() {
            state += " · reading";
        }
        // what is not as a clip opens: loop, speed, the look, the guides, the zoom
        if self.transport.seam().is_some() {
            state += " · loop analysis";
        } else {
            state += tab.0;
        }
        if self.transport.speed() != 1.0 {
            state += &format!(" · {}×", self.transport.speed());
        }
        if let Some(look) = self.look.label() {
            state += &format!(" · {look}");
        }
        for (on, name) in [(settings.grid, "grid"), (settings.center, "center")] {
            if on {
                state += &format!(" · {name}");
            }
        }
        if self.framing.zoomed() {
            state += &format!(" · {}", self.framing.label(window));
        }
        if let Some((_, (x, y), [r, g, b])) = self.probe {
            state += &format!(" · x {x} y {y} · {r} {g} {b}");
        }
        state += tab.1;
        format!("{} · {} / {} · {} × {} · {}{decoded} · {}{state}", c.name, self.transport.frame() + settings.first_frame, c.timeline.frames(), c.width, c.height, c.codec, c.rate_text())
    }
}

/// one tab of the window.
struct Tab {
    /// the file it was asked to show, as given
    path: Option<String>,
    /// its clip, when the file opened
    view: Option<View>,
    /// why it did not
    error: Option<String>,
    /// the browser is laid over what the tab holds
    browsing: bool,
    /// the file's facts for the metadata panel, read in the background: on their way, here, or why not
    facts: Option<Loaded<Meta>>,
    /// what play does at the end: off or one for a clip, and all as well for a folder. it is the tab's
    mode: settings::Loop,
    /// the folder it plays, when it plays one, and the clip on its way to take the place of the one on screen
    played: Option<Played>,
    coming: Option<Coming>,
    /// the folder's clips before and after the one on screen, kept opened with their first frame read
    parked: Vec<Coming>,
    /// refreshes the last frame of a clip was held while play waited for the folder's next clip
    held: u64,
    /// what the strip calls a tab that plays a folder
    label: String,
    /// the watch on its file, for a render that writes over it
    watch: Option<Watching>,
    /// the marks drawn on each of its clips, by file: they stay while the tab is open, through cuts, steps and
    /// another tab being up, and go with it
    marks: HashMap<String, Marks>,
}

/// a watch on a tab's file, kept from a thread of its own (asking after a file on a share that sleeps can take
/// seconds): it tells the window when the file is a different one and has stopped changing, or has gone. it ends
/// when this is dropped.
struct Watching(Arc<AtomicBool>);

impl Watching {
    fn start(path: &str, wake: &Sender<Event>) -> Watching {
        let stop = Arc::new(AtomicBool::new(false));
        let (stopped, file, wake) = (stop.clone(), path.to_string(), wake.clone());
        std::thread::spawn(move || {
            let network = browse::volume_of(Path::new(&file)) == browse::Volume::Network;
            // a few looks a second at a local file. on a network volume a look is the folder's listing asked for the
            // one name, every two seconds: the file itself is never opened or asked about there (adr 0004)
            let rest = if network { Duration::from_secs(2) } else { Duration::from_millis(250) };
            let mut watch = Watch::new(Path::new(&file), network);
            while !stopped.load(Ordering::Relaxed) {
                std::thread::sleep(rest);
                let seen = match watch.poll() {
                    Change::None => continue,
                    Change::Changed => Event::Changed(file.clone()),
                    Change::Gone => Event::Gone(file.clone()),
                };
                if stopped.load(Ordering::Relaxed) || wake.send(seen).is_err() {
                    return;
                }
            }
        });
        Watching(stop)
    }
}

impl Drop for Watching {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// the folder a tab plays: its clips in name order, and which of them is up
struct Played {
    title: String,
    items: Vec<std::path::PathBuf>,
    at: usize,
    /// on a volume where files may be read ahead of their turn: their headers for the list, and the clips before
    /// and after the one on screen kept ready. not on a network volume, where a clip is opened only when it plays
    local: bool,
    /// each clip's name, and what its header says, as far as the headers have been read; the reader, while it reads
    names: Vec<String>,
    facts: Vec<Option<Result<crate::media::clip::Facts, String>>>,
    reading: Option<Receiver<(usize, Result<crate::media::clip::Facts, String>)>>,
}

impl Played {
    /// a folder's clips, with their headers being read in the background: no reader is started for them and no
    /// frame read, so a folder of hundreds costs its headers.
    fn new(title: String, items: Vec<std::path::PathBuf>, wake: &Sender<Event>, local: bool) -> Played {
        let names: Vec<String> = items.iter().map(|file| file.file_name().map_or_else(|| file.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())).collect();
        if !local {
            return Played { title, at: 0, local, names, facts: items.iter().map(|_| None).collect(), items, reading: None };
        }
        let (read, reading) = mpsc::channel();
        let (files, wake) = (items.clone(), wake.clone());
        std::thread::spawn(move || {
            for (i, file) in files.iter().enumerate() {
                if read.send((i, crate::media::clip::facts(&file.to_string_lossy()))).is_err() {
                    return;
                }
                // the list fills in as it is read, a few clips a look
                if i % 16 == 15 {
                    let _ = wake.send(Event::Wake);
                }
            }
            drop(read);
            let _ = wake.send(Event::Wake);
        });
        Played { title, at: 0, local, names, facts: items.iter().map(|_| None).collect(), items, reading: Some(reading) }
    }
}

/// why a clip is on its way to a tab
#[derive(Clone, Copy, PartialEq)]
enum Why {
    /// a file was opened in the tab: it starts as a clip does, and the window grows to it
    Opened,
    /// a step by hand to another of the folder's clips
    Stepped,
    /// play ran off the end of the folder's clip before: it goes on in this one, from its first frame
    Ran,
    /// the file on screen changed on disk: it comes up on the same frame
    Changed,
}

/// a clip on its way to a tab: opened off the drawing thread, and put up once its first frame is in its painter.
/// what the tab shows stays up until then, so the picture is never blank at a cut and the window never waits on
/// a file.
struct Coming {
    /// which of the folder's clips, in a tab that plays one
    item: usize,
    path: String,
    why: Why,
    /// it plays as soon as it is up
    play: bool,
    /// the folder the tab plays from then on (play folder), and the frame, zoom, pan and look it comes up on (a
    /// closed tab brought back)
    played: Option<Played>,
    place: Option<(usize, Framing, Look)>,
    /// the clip, until it has arrived, and the thread that opens it; for a file that lists no frames, how many
    /// have been counted, and the word that ends the count
    opening: Option<Receiver<Result<(Clip, Result<Option<Audio>, String>), String>>>,
    worker: Option<std::thread::JoinHandle<()>>,
    counted: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    view: Option<View>,
    /// how many clips before it play ran onto in a row that did not open
    tried: usize,
    /// why it did not open, once that is known; and why its sound cannot be played, when it has sound that cannot
    failed: Option<String>,
    no_sound: Option<String>,
    /// the tab has handed on to it what carries over (the speed, the look, a frame to come up on)
    seeded: bool,
    /// for a clip kept ready: its file's size and time when it was opened
    stamp: Option<(u64, std::time::SystemTime)>,
    /// when it was asked for, and when the window last said how far it has got
    since: Instant,
    said: Instant,
}

/// how long a clip may be on its way before the window says so, and how often it then says how far the count is
const SLOW: Duration = Duration::from_millis(250);
const SAY_EVERY: Duration = Duration::from_millis(100);

impl Coming {
    fn start(item: usize, path: String, why: Why, hardware: crate::media::dec::Hardware, wake: Sender<Event>) -> Coming {
        let (opened, opening) = mpsc::channel();
        let (counted, stop) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
        let (file, count, stopped) = (path.clone(), counted.clone(), stop.clone());
        let work = move || {
            // decoding starts here too: making a hardware decoder took 80 to 250 ms for a 4K clip. and the clip's
            // sound is looked for here, where the window does not wait for it (stills have none)
            let clip = Clip::open_counting(&file, &count, &stopped).and_then(|mut clip| clip.start(Some(hardware)).map(|()| clip));
            let _ = opened.send(clip.map(|clip| {
                let sound = if matches!(clip.lane, Lane::Stills(_)) { Ok(None) } else { Audio::open(&file) };
                (clip, sound)
            }));
            let _ = wake.send(Event::Wake);
        };
        let worker = std::thread::Builder::new().name("opening".into()).spawn(work).ok();
        let now = Instant::now();
        Coming { item, path, why, play: why == Why::Ran, played: None, place: None, opening: Some(opening), worker, counted, stop, view: None, tried: 0, failed: None, no_sound: None, seeded: false, stamp: None, since: now, said: now }
    }

    /// takes the clip once it is opened, makes its view, and reads the frame its transport is on, waiting up to
    /// `patience` for each. true when that frame is in its painter. Err, every time, for a clip that did not open.
    fn arrive(&mut self, gpu: &Gpu, hz: f64, patience: Duration) -> Result<bool, String> {
        if let Some(why) = &self.failed {
            return Err(why.clone());
        }
        if let Some(opening) = &self.opening {
            let clip = match opening.recv_timeout(patience) {
                Err(RecvTimeoutError::Timeout) => return Ok(false),
                Err(RecvTimeoutError::Disconnected) => Err("its reader did not start".to_string()),
                Ok(clip) => clip,
            };
            self.opening = None;
            match clip.and_then(|(clip, sound)| View::new(gpu, clip, hz).map(|view| (view, sound))) {
                Ok((mut view, sound)) => {
                    // sound that cannot be played is said when the clip comes up
                    (view.sound, self.no_sound) = match sound {
                        Ok(sound) => (Some(sound), None),
                        Err(why) => (Some(None), Some(why.strip_prefix(&format!("{}: ", self.path)).unwrap_or(&why).to_string())),
                    };
                    self.view = Some(view);
                }
                Err(why) => {
                    self.failed = Some(why.clone());
                    return Err(why);
                }
            }
        }
        let Some(view) = &mut self.view else { return Ok(false) };
        // its one frame, and nothing read ahead until it is up
        let frame = view.transport.frame();
        view.load_wanting(gpu, patience, &[frame]);
        Ok(!view.waiting())
    }

    /// what there is to say of a clip that is slow to come: that it is being opened, and for a file that lists no
    /// frames how many have been counted. None for the first moment, in which most clips are up.
    fn words(&self) -> Option<String> {
        if self.since.elapsed() < SLOW {
            return None;
        }
        let name = Path::new(&self.path).file_name().map_or(self.path.as_str().into(), |n| n.to_string_lossy());
        Some(match self.counted.load(Ordering::Relaxed) {
            0 => format!("opening {name}"),
            frames => format!("opening {name} · {frames} frames counted"),
        })
    }
}

/// what a tab hands on to the clip coming to it. a file opened in the tab starts as a clip does (a closed tab
/// brought back, on its frame, zoom and pan). a step by hand between clips of one size and length keeps the frame,
/// zoom and pan, so two versions flip on the spot, and so does a file that changed on disk, as far as the new file
/// has them; the speed, the look and loop analysis are the tab's and stay through a folder's cuts.
fn seed(view: &mut View, why: Why, place: Option<(usize, Framing, Look)>, old: Option<&View>) {
    match (why, old) {
        (Why::Opened, _) => {
            if let Some((frame, framing, look)) = place {
                view.transport.go(frame as i64);
                (view.framing, view.look) = (framing, look);
            }
        }
        (why, Some(old)) => {
            view.transport.set_speed(old.transport.speed());
            view.set_look(old.look);
            let same = (view.size() == old.size(), view.clip.timeline.frames() == old.clip.timeline.frames());
            if why == Why::Changed || (why == Why::Stepped && same == (true, true)) {
                view.transport.go(old.transport.frame() as i64);
            }
            if why != Why::Ran && same.0 && (why == Why::Changed || same.1) {
                view.framing = old.framing.clone();
            }
            if let Some(seconds) = old.transport.seam() {
                view.transport.set_seam(Some(seconds));
            }
        }
        (_, None) => {}
    }
}

/// lets a clip that was on its way, or kept ready, go without waiting for it: a count of its frames is stopped, its
/// view is closed off the drawing thread, and its opening thread is left to finish by itself. that thread has the
/// graphics device in hand, so it is waited for when the window closes (`stragglers`).
fn let_go(mut coming: Coming, stragglers: &mut Vec<std::thread::JoinHandle<()>>) {
    coming.stop.store(true, Ordering::Relaxed);
    stragglers.retain(|worker| !worker.is_finished());
    stragglers.extend(coming.worker.take());
    if let Some(view) = coming.view.take() {
        view.retire();
    }
}

/// a file's size and time, as its folder has them now.
fn stamp(path: &str) -> Option<(u64, std::time::SystemTime)> {
    let file = std::fs::metadata(path).ok()?;
    Some((file.len(), file.modified().ok()?))
}

impl Drop for Coming {
    /// the opening thread is handed the graphics device: it must be done with it before the device can go. a count
    /// of the file's frames is stopped first, so this never waits for a whole file to be read.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// a file's facts for the metadata panel, read apart from its clip: they are there for a file that does not play too.
fn read_facts(path: &str, wake: &Sender<Event>) -> Loaded<Meta> {
    let (reader, arriving) = mpsc::channel();
    let (file, wake) = (path.to_string(), wake.clone());
    std::thread::spawn(move || {
        let _ = reader.send(Meta::read(&file));
        let _ = wake.send(Event::Wake);
    });
    Loaded::Reading(arriving)
}

/// something read in the background: on its way, here, or not to be had
enum Loaded<T> {
    Reading(Receiver<Result<T, String>>),
    Ready(T),
    Failed(String),
}

impl<T> Loaded<T> {
    /// takes what has arrived, if anything has.
    fn poll(&mut self) {
        if let Loaded::Reading(arriving) = self {
            match arriving.try_recv() {
                Ok(Ok(it)) => *self = Loaded::Ready(it),
                Ok(Err(why)) => *self = Loaded::Failed(why),
                Err(TryRecvError::Disconnected) => *self = Loaded::Failed("the reader stopped".into()),
                Err(TryRecvError::Empty) => {}
            }
        }
    }
}

/// the loop mode a tab opens with: the folder's for a played folder, else off or one as it was left for a clip.
fn opening_mode(folder: bool, settings: &Settings) -> settings::Loop {
    match (folder, settings.loop_file) {
        (true, _) => settings.loop_folder,
        (false, settings::Loop::Off) => settings::Loop::Off,
        (false, _) => settings::Loop::One,
    }
}

/// the speaker as loud as the settings say.
fn loudness(speaker: &mut Speaker, settings: &Settings) {
    speaker.set_volume(settings.volume);
    speaker.set_muted(settings.mute);
}

/// a line for the window to say, and until when
fn say(words: impl Into<String>) -> Option<(String, Instant)> {
    Some((words.into(), Instant::now() + Duration::from_millis(2600)))
}

/// a tab that was closed: its file, the frame it was on and how the picture sat, for bringing it back
type Closed = (String, usize, Framing, Look);

impl Tab {
    fn empty() -> Tab {
        Tab { path: None, view: None, error: None, browsing: false, facts: None, mode: settings::Loop::One, played: None, coming: None, parked: Vec::new(), held: 0, label: String::new(), watch: None, marks: HashMap::new() }
    }

    /// a tab for a file: its clip, or why there is none.
    fn open(gpu: &Gpu, path: String, clip: Result<Clip, String>, hz: f64, settings: &Settings, wake: &Sender<Event>) -> Tab {
        let (facts, watch) = (Some(read_facts(&path, wake)), Some(Watching::start(&path, wake)));
        let (view, error) = match clip.and_then(|clip| View::new(gpu, clip, hz)) {
            Ok(view) => (Some(view), None),
            Err(why) => (None, Some(why)),
        };
        let mode = if settings.loop_file == settings::Loop::Off { settings::Loop::Off } else { settings::Loop::One };
        Tab { path: Some(path), view, error, browsing: false, facts, mode, played: None, coming: None, parked: Vec::new(), held: 0, label: String::new(), watch, marks: HashMap::new() }
    }

    /// the marks of the clip it shows, when it shows one.
    fn marks(&self) -> Option<&Marks> {
        self.marks.get(self.path.as_ref()?)
    }

    /// play or pause, asked for while play waits on a clip's last frame for the folder's next, or while a file is
    /// being opened: it is kept for the clip on its way. false when no clip is on its way that way.
    fn toggle_cut(&mut self) -> bool {
        match &mut self.coming {
            Some(coming) if matches!(coming.why, Why::Ran | Why::Opened) => {
                coming.play = !coming.play;
                true
            }
            _ => false,
        }
    }

    /// true while play waits on a clip's last frame for the folder's next.
    fn cutting(&self) -> bool {
        self.coming.as_ref().is_some_and(|c| c.why == Why::Ran && c.play)
    }

    /// sets the loop mode, which ends loop analysis, and keeps it for the next tab of its kind.
    fn set_mode(&mut self, mode: settings::Loop, settings: &mut Settings) {
        self.mode = mode;
        if self.played.is_some() {
            settings.loop_folder = mode;
        } else {
            settings.loop_file = mode;
        }
        if let Some(v) = self.view.as_mut().filter(|v| v.transport.seam().is_some()) {
            v.transport.set_seam(None);
        }
    }

    /// the strip's name for a tab that plays a folder: the folder, where it is in it, and the clip.
    fn relabel(&mut self) {
        if let Some(played) = &self.played {
            self.label = format!("{} / {} of {} · {}", played.title, played.at + 1, played.items.len(), self.clip_name());
        }
    }

    /// what the title says of the tab: its loop mode when that is not what its kind opens with (one for a clip, all
    /// for a folder), and for a folder which clip is up and the refreshes held at its cuts.
    fn words(&self) -> (&'static str, String) {
        let mode = match (self.mode, self.played.is_some()) {
            (settings::Loop::Off, _) => " · loop off",
            (settings::Loop::One, true) => " · loop one",
            (settings::Loop::All, false) => " · loop all",
            _ => "",
        };
        let mut folder = self.played.as_ref().map_or_else(String::new, |p| format!(" · clip {} of {}", p.at + 1, p.items.len()));
        if self.held > 0 {
            folder += &format!(" · {} held at cuts", self.held);
        }
        (mode, folder)
    }

    /// what the strip calls it.
    fn name(&self) -> &str {
        if self.played.is_some() {
            return &self.label;
        }
        self.clip_name()
    }

    fn clip_name(&self) -> &str {
        match (&self.view, &self.path) {
            (Some(view), _) => &view.clip.name,
            (None, Some(path)) => Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path),
            (None, None) => "new tab",
        }
    }

    /// nothing has been opened in it.
    fn is_empty(&self) -> bool {
        self.view.is_none() && self.error.is_none()
    }

    /// the browser is what this tab shows: asked for over its clip, or because there is nothing else to show and
    /// nothing on its way.
    fn browsing(&self) -> bool {
        self.browsing || (self.is_empty() && self.coming.is_none())
    }
}

/// the tabs of the window, of which one is up. there is always at least one.
struct Tabs {
    all: Vec<Tab>,
    up: usize,
    closed: Vec<Closed>,
    /// the display's refresh rate, which every clip opened is paced to
    hz: f64,
    /// opening threads of clips that were let go before they were done (`let_go`)
    stragglers: Vec<std::thread::JoinHandle<()>>,
}

impl Drop for Tabs {
    /// an opening thread has the graphics device in hand: each is done before the device goes.
    fn drop(&mut self) {
        for worker in self.stragglers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Tabs {
    fn tab(&mut self) -> &mut Tab {
        &mut self.all[self.up]
    }


    /// the clip on screen, when a clip is what is on screen.
    fn view(&mut self) -> Option<&mut View> {
        let tab = self.tab();
        if tab.browsing() {
            None
        } else {
            tab.view.as_mut()
        }
    }

    /// the clip on screen with its marks, when a clip is what is on screen.
    fn marked(&self) -> Option<(&View, Option<&Marks>)> {
        let tab = &self.all[self.up];
        Some((tab.view.as_ref().filter(|_| !tab.browsing())?, tab.marks()))
    }

    /// the marks of the clip on screen, to draw on or take off, and the frame it is on.
    fn marking(&mut self) -> Option<(&mut Marks, usize)> {
        let tab = &mut self.all[self.up];
        let frame = tab.view.as_ref().filter(|_| !tab.browsing())?.transport.frame();
        Some((tab.marks.entry(tab.path.clone()?).or_default(), frame))
    }

    /// puts another tab up. the one left behind stops where it is. two clips of one size and length change places on
    /// the same frame, zoom and pan, which makes two tabs a flip comparison.
    fn select(&mut self, to: usize) {
        if to == self.up || to >= self.all.len() {
            return;
        }
        // the tab left keeps nothing ready: each clip kept holds a decoder
        let Tabs { all, up, stragglers, .. } = self;
        for gone in all[*up].parked.drain(..) {
            let_go(gone, stragglers);
        }
        let carried = self.tab().view.as_mut().map(|v| {
            v.transport.set_playing(false);
            (v.size(), v.clip.timeline.frames(), v.transport.frame(), v.framing.clone(), v.look)
        });
        self.up = to;
        if let (Some((size, frames, frame, framing, look)), Some(v)) = (carried, self.tab().view.as_mut()) {
            if (v.size(), v.clip.timeline.frames()) == (size, frames) {
                v.transport.go(frame as i64);
                (v.framing, v.look) = (framing, look);
            }
        }
    }

    /// a new tab with nothing in it: the browser.
    fn new_tab(&mut self) {
        self.all.push(Tab::empty());
        self.select(self.all.len() - 1);
    }

    /// opens a file, off the drawing thread. from the browser it takes the place of what the tab that is up holds;
    /// dropped on the window or handed over by another launch it gets a tab of its own, and never replaces what is
    /// being looked at. what the tab shows stays up until the clip's first frame is in; the window then grows to a
    /// bigger clip and never shrinks. returns the clip on its way, for what else is to be said of it.
    fn open(&mut self, gpu: &Gpu, path: String, wake: &Sender<Event>, replace: bool) -> &mut Coming {
        if !(replace || self.tab().is_empty()) {
            self.all.push(Tab::empty());
            self.select(self.all.len() - 1);
        }
        let Tabs { all, up, stragglers, .. } = self;
        let tab = &mut all[*up];
        tab.browsing = false;
        if let Some(before) = tab.coming.take() {
            let_go(before, stragglers);
        }
        tab.coming.insert(Coming::start(0, path, Why::Opened, gpu.hardware(), wake.clone()))
    }

    /// plays a folder in the tab that is up: its clips one after another in name order, from the first, with the
    /// folder's loop mode. on a network volume it first asks, once, with the count of clips it will open (`asked`
    /// says that has just been done); there each clip is opened only when it is played. returns what there is to
    /// say about it.
    fn play_folder(&mut self, gpu: &Gpu, folder: &Path, wake: &Sender<Event>, asked: bool) -> Option<String> {
        let listed = browse::list(folder).map_err(|e| e.to_string().to_lowercase());
        let (items, left_out) = match listed {
            Ok(entries) => folder::items(&entries),
            Err(why) => return Some(why),
        };
        let Some(first) = items.first().map(|first| first.to_string_lossy().into_owned()) else { return Some("no video in this folder".into()) };
        let network = browse::volume_of(folder) == browse::Volume::Network;
        if let Some(ask) = folder::ask(browse::volume_of(folder), items.len(), asked) {
            return Some(ask);
        }
        let title = folder.file_name().map_or_else(|| folder.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
        let coming = self.open(gpu, first, wake, true);
        (coming.played, coming.play) = (Some(Played::new(title, items, wake, !network)), true);
        (left_out > 0).then(|| format!("{left_out} file{} not on this disk left out", if left_out == 1 { " is" } else { "s are" }))
    }

    /// takes in what has been read of a played folder's clips. once all are read the window is sized to the biggest
    /// of them by its count of pixels (the first in name order of two that tie): once, and only ever larger, so
    /// nothing jumps when the folder plays on (the design's fig. 14). true when something new came.
    fn settle(&mut self, window: Handle) -> bool {
        let Some(played) = &mut self.tab().played else { return false };
        let Some(reading) = &played.reading else { return false };
        let mut new = false;
        let done = loop {
            match reading.try_recv() {
                Ok((i, facts)) => (played.facts[i], new) = (Some(facts), true),
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        if done {
            played.reading = None;
            // the last of the biggest, from the end: the first of them in name order
            if let Some(size) = played.facts.iter().flatten().flatten().map(|f| (f.width, f.height)).rev().max_by_key(|(w, h)| w * h) {
                window::fit(window, window::client_for(size));
            }
        }
        new || done
    }

    /// starts bringing in another of the folder's clips.
    fn go_item(&mut self, to: usize, why: Why, gpu: &Gpu, wake: &Sender<Event>) {
        let Tabs { all, up, stragglers, .. } = self;
        let tab = &mut all[*up];
        let Some(path) = tab.played.as_ref().and_then(|p| p.items.get(to)) else { return };
        let path = path.to_string_lossy().into_owned();
        // a step by hand while play waits at a cut: play goes on in the clip stepped to
        let playing_on = tab.cutting();
        if let Some(before) = tab.coming.take() {
            let_go(before, stragglers);
        }
        // a clip kept ready is taken as it is, unless its file has changed since it was opened
        let kept = tab.parked.iter().position(|c| c.item == to && c.path == path).map(|at| tab.parked.swap_remove(at));
        let mut coming = match kept {
            Some(mut kept) if kept.stamp == stamp(&path) => {
                (kept.why, kept.play, kept.since) = (why, why == Why::Ran, Instant::now());
                kept
            }
            stale => {
                if let Some(stale) = stale {
                    let_go(stale, stragglers);
                }
                Coming::start(to, path, why, gpu.hardware(), wake.clone())
            }
        };
        coming.play |= playing_on;
        tab.coming = Some(coming);
    }

    /// the file at `path` changed on disk: every tab that has it open opens it again, and shows the new file in
    /// place of the old when the tab is up.
    fn reload(&mut self, path: &str, gpu: &Gpu, wake: &Sender<Event>) {
        let Tabs { all, stragglers, .. } = self;
        for tab in all.iter_mut().filter(|tab| tab.path.as_deref() == Some(path)) {
            let item = tab.played.as_ref().map_or(0, |p| p.at);
            if let Some(before) = tab.coming.take() {
                let_go(before, stragglers);
            }
            tab.coming = Some(Coming::start(item, path.to_string(), Why::Changed, gpu.hardware(), wake.clone()));
        }
    }

    /// keeps the clips of the folder before and after the one on screen opened, each with its first frame read, so
    /// a cut or a page key finds its clip ready and only swaps (the design's neighbor pre-open). those that are no
    /// longer next to the clip on screen are let go. nothing changes while another clip is on its way.
    fn park(&mut self, gpu: &Gpu, wake: &Sender<Event>) {
        let hz = self.hz;
        let Tabs { all, up, stragglers, .. } = self;
        let tab = &mut all[*up];
        let near: Vec<(usize, String)> = match &tab.played {
            Some(_) if tab.coming.is_some() || tab.view.is_none() => return,
            Some(played) if !played.local => Vec::new(),
            Some(played) => {
                let mut near: Vec<usize> = [1, -1].into_iter().filter_map(|by| folder::step(played.items.len(), played.at, by)).collect();
                near.dedup();
                near.into_iter().map(|item| (item, played.items[item].to_string_lossy().into_owned())).collect()
            }
            None => Vec::new(),
        };
        let mut at = 0;
        while at < tab.parked.len() {
            if near.iter().any(|(item, path)| (*item, path) == (tab.parked[at].item, &tab.parked[at].path)) {
                at += 1;
            } else {
                let_go(tab.parked.swap_remove(at), stragglers);
            }
        }
        for (item, path) in near {
            if !tab.parked.iter().any(|c| c.item == item) {
                let mut kept = Coming::start(item, path, Why::Stepped, gpu.hardware(), wake.clone());
                kept.stamp = stamp(&kept.path);
                tab.parked.push(kept);
            }
        }
        for kept in &mut tab.parked {
            // one that does not open says so when it is stepped to
            let _ = kept.arrive(gpu, hz, Duration::ZERO);
        }
    }

    /// page up and page down: the folder's clip before or after, round its ends, counted from the one on its way
    /// when there is one, so presses in a row each go one further.
    fn step_item(&mut self, by: i64, gpu: &Gpu, wake: &Sender<Event>) {
        let tab = self.tab();
        let Some(played) = &tab.played else { return };
        let from = tab.coming.as_ref().map_or(played.at, |c| c.item);
        if let Some(to) = folder::step(played.items.len(), from, by) {
            self.go_item(to, Why::Stepped, gpu, wake);
        }
    }

    /// play ran off the end of the folder's clip: the next takes over, when the loop mode has one.
    fn advance(&mut self, gpu: &Gpu, wake: &Sender<Event>) {
        let tab = self.tab();
        let Some(played) = &tab.played else { return };
        if let Some(next) = folder::after(tab.mode, played.items.len(), played.at) {
            self.go_item(next, Why::Ran, gpu, wake);
        }
    }

    /// brings the clip that is on its way nearer the screen: makes its view once it is opened, reads its first
    /// frame, and puts it up in place of what the tab shows when that frame is in its painter. it waits up to
    /// `patience` for what it needs next. what the tab hands on to it is `seed`'s; the window grows to a file opened
    /// in the tab.
    /// Some when the tab shows something new, with what there is to say about it; Err says why a clip of a folder
    /// did not open, and the clip on screen then stays.
    fn bring_in(&mut self, gpu: &Gpu, window: Handle, settings: &Settings, wake: &Sender<Event>, patience: Duration) -> Result<Option<String>, String> {
        let hz = self.hz;
        let tab = self.tab();
        let Some(coming) = &mut tab.coming else { return Ok(None) };
        let mut ready = match coming.arrive(gpu, hz, patience) {
            Ok(ready) => ready,
            Err(why) => {
                {
                    let Some(mut coming) = tab.coming.take() else { return Ok(None) };
                    return match coming.why {
                        // a file opened in the tab, or stepped to in its folder, that does not open is what the tab
                        // then says, with its facts as far as they can be read: the folder is still there to step
                        // on through. it stays watched: a render that finishes it brings it in
                        Why::Opened | Why::Stepped => {
                            if let Some(old) = tab.view.take() {
                                old.retire();
                            }
                            if coming.why == Why::Opened {
                                tab.played = coming.played.take();
                                tab.mode = opening_mode(tab.played.is_some(), settings);
                            } else if let Some(played) = &mut tab.played {
                                played.at = coming.item;
                            }
                            (tab.facts, tab.watch) = (Some(read_facts(&coming.path, wake)), Some(Watching::start(&coming.path, wake)));
                            (tab.path, tab.error) = (Some(std::mem::take(&mut coming.path)), Some(why));
                            tab.relabel();
                            Ok(Some(String::new()))
                        }
                        // a file caught while it is still being written does not open: its next change brings it in
                        Why::Changed => Ok(None),
                        // play goes on to the next clip that opens, and says which it passed over: once round
                        // the folder at most, and no further than the loop mode goes
                        Why::Ran => {
                            let tried = coming.tried + 1;
                            let next = tab.played.as_ref().filter(|played| tried < played.items.len()).and_then(|played| {
                                let next = folder::after(tab.mode, played.items.len(), coming.item)?;
                                Some((next, played.items[next].to_string_lossy().into_owned()))
                            });
                            let Some((item, path)) = next else { return Err(why) };
                            let mut on = Coming::start(item, path, Why::Ran, gpu.hardware(), wake.clone());
                            (on.tried, on.play) = (tried, coming.play);
                            tab.coming = Some(on);
                            Ok(Some(format!("passed over · {why}")))
                        }
                    };
                }
            }
        };
        // what the tab hands on to it, once, when its view is there: the frame it comes up on may then be another
        if let (false, Some(view)) = (coming.seeded, &mut coming.view) {
            seed(view, coming.why, coming.place.take(), tab.view.as_ref());
            coming.seeded = true;
            let frame = view.transport.frame();
            view.load_wanting(gpu, Duration::ZERO, &[frame]);
            ready = !view.waiting();
        }
        if !ready {
            return Ok(None);
        }
        let Some(mut coming) = tab.coming.take() else { return Ok(None) };
        let Some(mut view) = coming.view.take() else { return Ok(None) };
        // a file that changed on disk says so, and whether it is on the frame it was on (the design's fig. 22)
        let said = match (&tab.view, coming.why) {
            (Some(old), Why::Changed) => {
                let place = if view.transport.frame() == old.transport.frame() { "same frame" } else { "its last frame" };
                format!("{} changed on disk at {} · {place}", view.clip.name, window::clock())
            }
            _ => coming.no_sound.take().map_or_else(String::new, |why| format!("no sound: {why}")),
        };
        let goes_on = matches!(coming.why, Why::Stepped | Why::Changed) && tab.view.as_ref().is_some_and(|old| old.transport.playing());
        if coming.play || goes_on {
            view.transport.set_playing(true);
        }
        if coming.why == Why::Opened {
            tab.played = coming.played.take();
            // a folder's window is sized when its headers are read; where they are not read, to its first clip
            if tab.played.as_ref().is_none_or(|played| !played.local) {
                window::fit(window, window::client_for((view.clip.width, view.clip.height)));
            }
            tab.mode = opening_mode(tab.played.is_some(), settings);
            tab.held = 0;
        } else if let Some(played) = &mut tab.played {
            played.at = coming.item;
        }
        (tab.facts, tab.watch) = (Some(read_facts(&coming.path, wake)), Some(Watching::start(&coming.path, wake)));
        (tab.path, tab.error) = (Some(std::mem::take(&mut coming.path)), None);
        if let Some(old) = tab.view.replace(view) {
            old.retire();
        }
        tab.relabel();
        Ok(Some(said))
    }

    /// closes a tab, remembering it. closing the last one leaves an empty tab: the window stays.
    fn close(&mut self, at: usize) {
        if at >= self.all.len() {
            return;
        }
        let mut tab = self.all.remove(at);
        for gone in tab.parked.drain(..).chain(tab.coming.take()) {
            let_go(gone, &mut self.stragglers);
        }
        if let (Some(path), Some(view)) = (tab.path.take(), tab.view.take()) {
            self.closed.push((path, view.transport.frame(), view.framing.clone(), view.look));
            view.retire();
        }
        if self.all.is_empty() {
            self.all.push(Tab::empty());
        }
        if at < self.up || self.up >= self.all.len() {
            self.up = self.up.saturating_sub(1);
        }
    }

    /// brings back the tab closed last, on its frame, zoom and pan.
    fn reopen(&mut self, gpu: &Gpu, wake: &Sender<Event>) {
        let Some((path, frame, framing, look)) = self.closed.pop() else { return };
        if !self.tab().is_empty() {
            self.new_tab();
        }
        self.open(gpu, path, wake, true).place = Some((frame, framing, look));
    }
}

/// takes a screenshot of the clip on screen: the file's frame at full size, the view as shown, or the frame to the
/// clipboard. files are written in the background, and a toast says what was written.
fn shoot(v: &View, marks: Option<&Marks>, gpu: &Gpu, what: Shot, settings: &Settings, scale: f32, wake: &Sender<Event>) {
    let Some((size, pixels, with)) = shot_pixels(v, marks, gpu, what, settings, scale) else {
        let _ = wake.send(Event::Toast("there is no picture to save".into()));
        return;
    };
    if what == Shot::Copy {
        let said = if window::copy_picture(size.0, size.1, &pixels) { format!("frame copied · {} × {}, 8 bits a channel", size.0, size.1) } else { "the clipboard could not be had".to_string() };
        let _ = wake.send(Event::Toast(said));
        return;
    }
    // a saved frame keeps the file's alpha when its frame has one; the view is saved as it is seen
    let alpha = what == Shot::Frame && v.carries_alpha() && shot::transparent(&pixels);
    let stem = shot::stem(&v.clip.name, v.transport.frame() + settings.first_frame) + if what == Shot::View { ".view" } else { "" };
    let (deep, folder, wake) = (v.depth() > 8, shots_folder(settings), wake.clone());
    let mut detail = if what == Shot::View { "as shown".to_string() } else { format!("{} × {}", v.clip.width, v.clip.height) };
    if !with.is_empty() {
        detail += &format!(" · with {}", with.join(", "));
    }
    std::thread::spawn(move || {
        let said = match shot::save(&pixels, size.0, size.1, deep, alpha, &folder, &stem) {
            Ok(file) => format!("saved to {} · {} · {detail}", shots_to(&folder), file.file_name().map_or_else(|| file.display().to_string(), |n| n.to_string_lossy().into_owned())),
            Err(why) => format!("not saved · {why}"),
        };
        let _ = wake.send(Event::Toast(said));
    });
}

/// what a screenshot is made of: its size, its pixels as `shot::save` takes them, and what the screenshot menu's
/// switches put in it besides the picture. the frame is the file's own at its own size, with the guides and the marks
/// drawn as fig. 20 draws them on a frame 1200 wide, scaled to its width (never smaller than at 100% on screen); the
/// view is the window's picture, with them as the window shows them. a caption goes under either. the copy is the
/// frame alone.
fn shot_pixels(v: &View, marks: Option<&Marks>, gpu: &Gpu, what: Shot, settings: &Settings, scale: f32) -> Option<((u32, u32), Vec<u16>, Vec<&'static str>)> {
    let frame = v.transport.frame();
    let (w, h) = (v.clip.width as u32, v.clip.height as u32);
    let (size, at, place, unit) = match what {
        Shot::View => ((gpu.w, gpu.h), None, Placing { picture: v.framing.rect((gpu.w as f32, gpu.h as f32)), look: v.look, size: v.size() }, scale),
        Shot::Frame | Shot::Copy => {
            let whole = (0.0, 0.0, w as f32, h as f32);
            ((w, h), Some(whole), Placing { picture: whole, look: Look::default(), size: v.size() }, (2.1 * w as f32 / 1200.0).max(1.0))
        }
    };
    let extras = what != Shot::Copy;
    let guides = Some(guides(settings)).filter(|g| extras && settings.screenshot_guides && (g.grid || g.center));
    let marks = if extras && settings.screenshot_markup { marks.map_or(&[][..], |m| m.on(frame)) } else { &[] };
    let caption = (extras && settings.screenshot_caption).then(|| ui::caption(&v.clip.name, frame, settings.first_frame, v.clip.clock, v.clip.timeline.rate(), v.size()));
    let with: Vec<&'static str> = [(caption.is_some(), "caption"), (guides.is_some(), "guides"), (!marks.is_empty(), "markup")].into_iter().filter(|(on, _)| *on).map(|(_, name)| name).collect();
    if with.is_empty() {
        return Some((size, v.picture(gpu, size, at, |_| {})?, with));
    }
    // a pass of egui of their own, painted after the picture by a painter of its own (the window's holds the
    // controls' textures, under the same names)
    let mut over = ui::saved(size, unit, &place, guides, marks, caption.as_deref());
    let size = (size.0, over.height);
    let pixels = UiPainter::new(gpu).ok().and_then(|mut painter| {
        {
            let _lock = gpu.locked();
            painter.set_textures(gpu, &over.textures).ok()?;
        }
        v.picture(gpu, size, at, |gpu| {
            let _ = painter.paint(gpu, &over.primitives, over.scale, size);
        })
    });
    // egui checks that no change to its textures is dropped unseen
    over.textures.clear();
    Some((size, pixels?, with))
}

/// where screenshots go: the folder the settings name, or pictures / eyepiece.
fn shots_folder(settings: &Settings) -> std::path::PathBuf {
    settings.screenshot_folder.clone().unwrap_or_else(shot::folder)
}

/// a folder as the controls name it: its last two parts, "pictures / eyepiece".
fn shots_to(folder: &Path) -> String {
    let parts: Vec<String> = folder.components().rev().take(2).map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join(" / ")
}

/// the guides as the settings have them.
fn guides(settings: &Settings) -> Guides {
    Guides { grid: settings.grid, center: settings.center, tile: settings.tile, color: settings.guide_color }
}

/// the pointer, when it is over the picture: between the strip and the bar, as in the design's prototype.
fn over_picture(pointer: Option<(f32, f32)>, window: (f32, f32), scale: f32) -> Option<(f32, f32)> {
    pointer.filter(|p| p.1 > 34.0 * scale && p.1 < window.1 - 52.0 * scale)
}

/// runs the player until its window closes. what is kept between runs is read first and written back after.
/// `first` is the file given at the start, with its clip already opened or the reason it did not.
pub fn run(window: Handle, events: Receiver<Event>, wake: Sender<Event>, first: Option<(String, Result<Clip, String>)>) {
    let mut settings = Settings::load();
    play(window, events, wake, first, &mut settings);
    if let Err(e) = settings.save() {
        eprintln!("eyepiece: the settings were not saved: {e}");
    }
}

fn play(window: Handle, events: Receiver<Event>, wake: Sender<Event>, first: Option<(String, Result<Clip, String>)>, settings: &mut Settings) {
    let (w, h) = window::client_size(window);
    // declared before the tabs on purpose: a clip, and with it any decoder writing to this device, goes first
    let mut gpu = match Gpu::new(window, w, h) {
        Ok(gpu) => gpu,
        Err(e) => return window::set_title(window, &format!("eyepiece · no graphics device: {e}")),
    };
    let hz = window::refresh_rate();
    // the maker of the browser's pictures stands back while a clip plays
    let playing_now = Arc::new(AtomicBool::new(false));
    let mut ui = Ui::new((w, h), window::scale(window), settings.timecode, settings.browser_folder.clone(), pictures::start(playing_now.clone()));
    ui.set_browser_recent(settings.browser_recent.clone());
    ui.set_buttons_left(cfg!(target_os = "macos"));
    // the player's one way out to the speakers, as loud as it was left
    let mut speaker = Speaker::new();
    loudness(&mut speaker, settings);
    let mut speaker_open = false;
    // the network folder play was last asked about, and until when asking for it again plays it
    let mut asked_folder: Option<(std::path::PathBuf, Instant)> = None;
    // a second reader of the clip on screen, for loop analysis
    let spare = pictures::start_spare(wake.clone(), playing_now.clone());
    // the clip the spare reader has in hand for the timebar, and the frame last asked of it
    let (mut time_of, mut time_asked): (Option<String>, Option<(String, usize)>) = (None, None);
    let mut ui_painter = match UiPainter::new(&gpu) {
        Ok(p) => p,
        Err(e) => return window::set_title(window, &format!("eyepiece · the controls could not be set up: {e}")),
    };
    // when the controls want another look with nothing new from the window: they are due to fade
    let mut ui_due: Option<Instant> = None;
    // the file given at launch is open already, unless its frames have to be counted: that is done from here
    let (first, to_count) = match first {
        Some((path, Err(why))) if why.ends_with(crate::media::clip::UNCOUNTED) => (Tab::empty(), Some(path)),
        Some((path, clip)) => (Tab::open(&gpu, path, clip, hz, settings, &wake), None),
        None => (Tab::empty(), None),
    };
    // what just happened, said in the window until then; and where screenshots go, as the menu names it
    let mut toast: Option<(String, Instant)> = None;
    let shots_label = shots_to(&shots_folder(settings));
    let mut tabs = Tabs { all: vec![first], up: 0, closed: Vec::new(), hz, stragglers: Vec::new() };
    if let Some(path) = to_count {
        tabs.open(&gpu, path, &wake, true);
    }
    // where the pointer is, in physical pixels, while it is in the window; and physical pixels for one of the design's
    let (mut pointer, mut scale) = (None, window::scale(window));
    // the picture area fills a display; the pointer is shown (it hides in fullscreen while the controls are away)
    let (mut fullscreen, mut pointer_shown) = (false, window::Pointer::Arrow);
    let (mut redraw, mut titled) = (true, None);
    // the last key was a command, not typing: the character windows makes of the same press is then not typed
    // anywhere (N opens a new tab; its "n" must not pick a file in that tab's browser)
    let mut command = false;
    loop {
        let _pool = crate::sys::Pool::new();
        // a window's first frames may not reach the display: it is drawn again until one has
        redraw |= !gpu.reached();
        // with nothing moving and nothing owed, sleep until the window says something or the controls are due
        let playing = tabs.view().is_some_and(|v| v.transport.playing());
        let waiting = tabs.view().is_some_and(|v| v.waiting());
        let mut next = if playing || waiting || redraw || tabs.tab().coming.is_some() {
            match events.try_recv() {
                Ok(e) => Some(e),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else if let Some(due) = ui_due {
            match events.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(e) => Some(e),
                Err(RecvTimeoutError::Timeout) => {
                    redraw = true;
                    None
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match events.recv() {
                Ok(e) => Some(e),
                Err(_) => return,
            }
        };
        while let Some(event) = next {
            let size = (gpu.w as f32, gpu.h as f32);
            let browsing = tabs.tab().browsing();
            if let Event::Key { key, mods } = &event {
                let for_browser = browsing && !mods.ctrl && !mods.alt && !matches!(key, Key::BracketLeft | Key::BracketRight);
                command = !(ui.typing() || for_browser);
            }
            // any key takes the table of keys away and then does what it does; esc and ? only take it away
            let table_up = matches!(event, Event::Key { .. }) && ui.hide_keys();
            match event {
                Event::Close => return,
                Event::Resize { w, h } => {
                    if let Err(e) = gpu.resize(w, h) {
                        return window::set_title(window, &format!("eyepiece · the graphics device stopped: {e}"));
                    }
                    ui.resize(w, h);
                }
                Event::Scale(to) => {
                    scale = to;
                    ui.set_scale(to);
                }
                Event::PointerMove { x, y } => {
                    pointer = Some((x, y));
                    ui.pointer_move(x, y);
                }
                Event::PointerButton { button, down, x, y } => ui.pointer_button(button, down, x, y),
                Event::PointerLeave => {
                    pointer = None;
                    ui.pointer_leave();
                }
                Event::Open(path) => {
                    tabs.open(&gpu, path, &wake, false);
                }
                Event::Changed(path) => tabs.reload(&path, &gpu, &wake),
                Event::Gone(path) => toast = say(format!("{} is no longer where it was on disk", Path::new(&path).file_name().map_or(path.as_str().into(), |n| n.to_string_lossy()))),
                Event::Toast(said) => toast = say(said),
                // what was read in the background is taken up below, with everything else
                Event::Wake => {}
                Event::Key { key: Key::Escape | Key::Slash, .. } if table_up => {}
                // a field being typed into takes the keys; escape closes it and its menu
                Event::Key { key, mods } if ui.typing() => {
                    ui.key(key, mods);
                    if key == Key::Escape {
                        ui.escape();
                    }
                }
                // tabs. the brackets go round them; control + W closes the one that is up and control + shift + T
                // brings the last one closed back
                Event::Key { key: key @ (Key::BracketLeft | Key::BracketRight), mods } if !mods.ctrl && !mods.alt => {
                    let count = tabs.all.len();
                    tabs.select((tabs.up + if key == Key::BracketLeft { count - 1 } else { 1 }) % count);
                }
                Event::Key { key: Key::W, mods } if mods.ctrl => tabs.close(tabs.up),
                Event::Key { key: Key::T, mods } if mods.ctrl && mods.shift => tabs.reopen(&gpu, &wake),
                // control + C copies the frame
                Event::Key { key: Key::C, mods } if mods.ctrl && !browsing => {
                    if let Some((v, marks)) = tabs.marked() {
                        shoot(v, marks, &gpu, Shot::Copy, settings, scale, &wake);
                    }
                }
                // in markup mode control + Z takes back the last mark drawn on the frame on screen
                Event::Key { key: Key::Z, mods } if mods.ctrl && !browsing && ui.markup() => {
                    if let Some((marks, frame)) = tabs.marking() {
                        marks.undo(frame);
                    }
                }
                // escape leaves fullscreen first; then it is the browser's, which goes back to the clip, or closes a menu
                Event::Key { key: Key::Escape, .. } => {
                    if fullscreen {
                        fullscreen = false;
                        window::set_fullscreen(window, false, false);
                    } else if browsing {
                        ui.browser_key(Key::Escape, Mods::default());
                    } else {
                        ui.escape();
                    }
                }
                // in the browser the keys move about it and letters select by name
                Event::Key { key, mods } if browsing && !mods.ctrl && !mods.alt => ui.browser_key(key, mods),
                // the character a command's own press types is not typed. a field being typed into takes every
                // character: one that comes without a key of its own, after a command chosen with the pointer (undo
                // from the row of tools), is no command's
                Event::Char(_) if command && !ui.typing() => command = false,
                Event::Char(c) if browsing => ui.browser_char(c),
                // ? (or the slash alone) shows the table of keys over the clip
                Event::Key { key: Key::Slash, mods } if !mods.ctrl && !mods.alt => ui.show_keys(),
                // the keys that are not one clip's: a new tab, the browser over this one, fullscreen (with shift, on
                // the next display), the readout, the guides, the loop mode and loop analysis, markup mode
                Event::Key { key, mods } if !mods.ctrl && !mods.alt && matches!(key, Key::N | Key::O | Key::P | Key::T | Key::D | Key::C | Key::R | Key::A | Key::S | Key::I | Key::M | Key::E | Key::Up | Key::Down | Key::PageUp | Key::PageDown) => match key {
                    Key::N => tabs.new_tab(),
                    Key::PageUp => tabs.step_item(-1, &gpu, &wake),
                    Key::PageDown => tabs.step_item(1, &gpu, &wake),
                    Key::M => {
                        settings.mute = !settings.mute;
                        loudness(&mut speaker, settings);
                        ui.flash(if settings.mute { "muted" } else { "sound on" });
                    }
                    // a tenth a press; turning it up ends a mute
                    Key::Up | Key::Down => {
                        let by = if matches!(key, Key::Up) { 10.0 } else { -10.0 };
                        settings.volume = ((settings.volume * 100.0).round() + by).clamp(0.0, 100.0) / 100.0;
                        settings.mute &= !matches!(key, Key::Up);
                        loudness(&mut speaker, settings);
                        ui.flash(&format!("volume {}%", (settings.volume * 100.0).round()));
                    }
                    // a screenshot: the file's frame at full size; with shift, the view as shown
                    Key::S => {
                        if let Some((v, marks)) = tabs.marked() {
                            shoot(v, marks, &gpu, if mods.shift { Shot::View } else { Shot::Frame }, settings, scale, &wake);
                        }
                    }
                    // markup mode, over a clip: play stops for it
                    Key::E => {
                        if let Some(v) = tabs.view() {
                            if !ui.markup() {
                                v.transport.set_playing(false);
                            }
                            ui.toggle_markup();
                        }
                    }
                    Key::I => ui.toggle_facts(),
                    Key::O => {
                        let tab = tabs.tab();
                        tab.browsing = true;
                        if let Some(v) = &mut tab.view {
                            v.transport.set_playing(false);
                        }
                    }
                    Key::P => {
                        fullscreen = !fullscreen || mods.shift;
                        window::set_fullscreen(window, fullscreen, mods.shift);
                    }
                    Key::T => ui.toggle_timecode(),
                    Key::D => settings.grid = !settings.grid,
                    Key::C => settings.center = !settings.center,
                    Key::R => {
                        // off, one, for a folder all, and round again; from loop analysis, back to plain looping
                        let tab = tabs.tab();
                        let (folder, analysis) = (tab.played.is_some(), tab.view.as_ref().is_some_and(|v| v.transport.seam().is_some()));
                        let mode = match tab.mode {
                            _ if analysis && folder => settings::Loop::All,
                            _ if analysis => settings::Loop::One,
                            settings::Loop::Off => settings::Loop::One,
                            settings::Loop::One if folder => settings::Loop::All,
                            _ => settings::Loop::Off,
                        };
                        tab.set_mode(mode, settings);
                    }
                    _ => {
                        if let Some(v) = tabs.view() {
                            v.transport.set_seam(v.transport.seam().is_none().then_some(settings.loop_analysis_seconds as f64));
                        }
                    }
                },
                Event::Key { key: Key::Space, mods } if !mods.ctrl && !mods.alt && tabs.tab().toggle_cut() => {}
                Event::Key { key, mods } => {
                    if tabs.view().is_some_and(|v| v.key(key, mods, over_picture(pointer, size, scale), size)) {
                        ui.view_changed();
                    }
                }
                Event::Char(c) => {
                    if ui.typing() {
                        ui.char(c);
                    }
                }
                Event::Wheel { notches, .. } if browsing => ui.browser_wheel(notches),
                // over the metadata panel the wheel turns its list
                Event::Wheel { notches, .. } if ui.wheel(notches) => {}
                // with shift, one stop of zoom a notch, about the pointer: away from the user is in
                Event::Wheel { notches, mods } if mods.shift => {
                    if let Some(v) = tabs.view() {
                        (0..notches.abs()).for_each(|_| v.framing.step(notches.signum(), pointer, size));
                        ui.view_changed();
                    }
                }
                // one frame a notch. toward the user is forward, as a page scrolls
                Event::Wheel { notches, .. } => {
                    if let Some(v) = tabs.view() {
                        v.transport.step(-notches as i64);
                    }
                }
            }
            redraw = true;
            next = events.try_recv().ok();
        }

        // what the spare reader has read since the last turn, taken before the controls are laid out: an answer wakes
        // the window, and its picture is drawn in the turn it wakes (a picture of the timebar strip wakes nothing,
        // and is there when the timebar is next looked at)
        while let Some(answer) = spare.as_ref().and_then(|spare| spare.take()) {
            match answer {
                Answer::Seam(path, seam) => {
                    for v in tabs.all.iter_mut().filter(|tab| tab.path.as_deref().map(Path::new) == Some(path.as_path())).filter_map(|tab| tab.view.as_mut()) {
                        v.seam_change = seam;
                    }
                }
                Answer::Frame { path, frame, width, height, pixels, strip } => {
                    if time_of.as_deref().map(Path::new) == Some(path.as_path()) {
                        ui.timebar_picture(frame, (width, height), pixels, strip);
                    }
                }
            }
        }
        // the controls: laid out against where the clip is now. what they ask for is done before the frame is
        // fetched, so a click and its picture are one refresh apart
        let size = (gpu.w as f32, gpu.h as f32);
        // the pixel the zoom chip names: the file's pixel under the pointer, while a zoom is set
        let pixel_of = |v: &View| over_picture(pointer, size, scale).filter(|_| v.framing.zoomed()).and_then(|p| v.framing.pixel_at(p, size)).map(|shown| v.look.source_pixel(shown, v.size()));
        if let Some(facts) = &mut tabs.tab().facts {
            facts.poll();
        }
        if toast.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) {
            toast = None;
        }
        let out = {
            let tab = &tabs.all[tabs.up];
            // a clip on its way: what to say of it once it is slow, and that one is coming at all
            let opening = tab.coming.as_ref().map(|c| c.words().unwrap_or_default());
            let facts = match &tab.facts {
                Some(Loaded::Ready(meta)) => Facts::Ready(meta),
                Some(Loaded::Failed(why)) => Facts::Failed(why),
                Some(Loaded::Reading(_)) | None => Facts::Reading,
            };
            let screen = Screen {
                tabs: tabs.all.iter().map(Tab::name).collect(),
                places: tabs.all.iter().map(|t| t.played.as_ref().map(|p| ui::Place { folder: &p.title, at: p.at, count: p.items.len(), clip: t.clip_name() })).collect(),
                current: tabs.up,
                clip: tab.view.as_ref().map(|v| Model { marks: tab.marks(), ..v.model(size, pixel_of(v), settings, facts, &shots_label, (tab.mode, tab.played.is_some())) }),
                error: tab.error.as_deref(),
                opening: opening.as_deref().filter(|_| tab.view.is_none() && tab.error.is_none()),
                list: tab.played.as_ref().map(|played| FolderList {
                    title: &played.title,
                    at: played.at,
                    items: played
                        .names
                        .iter()
                        .zip(&played.facts)
                        .map(|(name, facts)| {
                            let first = played.facts[0].as_ref().and_then(|f| f.as_ref().ok());
                            let facts = facts.as_ref().and_then(|f| f.as_ref().ok());
                            ListItem {
                                name,
                                size: facts.map(|f| (f.width, f.height)),
                                rate: facts.and_then(|f| f.rate.as_deref()),
                                frames: facts.and_then(|f| f.frames),
                                odd_size: facts.zip(first).is_some_and(|(f, first)| f.other_size(first)),
                                odd_rate: facts.zip(first).is_some_and(|(f, first)| f.other_rate(first)),
                            }
                        })
                        .collect(),
                }),
                browsing: tab.browsing(),
                fullscreen,
                toast: opening.as_deref().filter(|_| tab.view.is_some() || tab.error.is_some()).or(toast.as_ref().map(|(said, _)| said.as_str())),
            };
            ui.run(&screen)
        };
        settings.timecode = ui.timecode();
        if settings.browser_folder.as_deref() != Some(ui.browser_folder()) {
            settings.browser_folder = Some(ui.browser_folder().to_path_buf());
        }
        if settings.browser_recent != ui.browser_recent() {
            settings.browser_recent = ui.browser_recent().to_vec();
        }
        for action in &out.actions {
            match action {
                Action::NewTab => tabs.new_tab(),
                Action::SelectTab(to) => tabs.select(*to),
                Action::GoClip(to) => tabs.go_item(*to, Why::Stepped, &gpu, &wake),
                Action::CloseTab(at) => tabs.close(*at),
                Action::Open(path) => {
                    tabs.open(&gpu, path.to_string_lossy().into_owned(), &wake, true);
                }
                Action::OpenTab(path) => {
                    tabs.new_tab();
                    tabs.open(&gpu, path.to_string_lossy().into_owned(), &wake, true);
                }
                Action::Reveal(file) => window::reveal(file, wake.clone()),
                Action::Shot(what) => {
                    if let Some((v, marks)) = tabs.marked() {
                        shoot(v, marks, &gpu, *what, settings, scale, &wake);
                    }
                }
                Action::Extras(to) => (settings.screenshot_caption, settings.screenshot_guides, settings.screenshot_markup) = (to.caption, to.guides, to.markup),
                Action::Mark(frame, mark) => {
                    if let Some((marks, _)) = tabs.marking() {
                        marks.add(*frame, mark.clone());
                    }
                }
                Action::ClearMarks => {
                    if let Some((marks, frame)) = tabs.marking() {
                        marks.clear(frame);
                    }
                }
                Action::Copy(text) => {
                    let said = if window::copy_text(text) { "metadata copied" } else { "the clipboard could not be had" };
                    toast = say(said);
                }
                Action::Volume(to) => {
                    settings.volume = (to * 100.0).round() / 100.0;
                    settings.mute = settings.volume <= 0.0;
                    loudness(&mut speaker, settings);
                }
                Action::Mute => {
                    settings.mute = !settings.mute;
                    loudness(&mut speaker, settings);
                }
                Action::Browse(on) => tabs.tab().browsing = *on,
                Action::PlayFolder(folder) => {
                    // a folder that had to be asked about is played when it is asked for again within a few seconds
                    let asked = asked_folder.take().is_some_and(|(of, until): (std::path::PathBuf, Instant)| &of == folder && Instant::now() < until);
                    if let Some(said) = tabs.play_folder(&gpu, folder, &wake, asked) {
                        toast = say(said);
                        asked_folder = (!asked).then(|| (folder.clone(), Instant::now() + Duration::from_secs(8)));
                    }
                }
                Action::Loop(mode) => tabs.tab().set_mode(*mode, settings),
                Action::TogglePlay if tabs.tab().toggle_cut() => {}
                Action::Guides(to) => (settings.grid, settings.center, settings.tile) = (to.grid, to.center, to.tile),
                Action::Fullscreen => {
                    fullscreen = !fullscreen;
                    window::set_fullscreen(window, fullscreen, false);
                }
                Action::MoveWindow => window::begin_move(window),
                Action::Minimize => window::minimize(window),
                Action::ToggleMaximize => window::toggle_maximize(window),
                Action::Close => window::close(window),
                // a command chosen in the right-click menu goes the way its key does
                Action::Key(key, mods) => {
                    let _ = wake.send(Event::Key { key: *key, mods: *mods });
                }
                // the rest are for the clip on screen
                clip => {
                    if let Some(v) = tabs.view() {
                        v.act(clip, size, settings);
                    }
                }
            }
        }
        // the controls are drawn again when they ask to be: at once while something of theirs is moving, later when
        // a fade is due, and once more after a click so they show what it did
        let mut again = !out.actions.is_empty();
        ui_due = None;
        let set = {
            let _lock = gpu.locked();
            ui_painter.set_textures(&gpu, &out.textures)
        };
        if let Err(e) = set {
            return window::set_title(window, &format!("eyepiece · the graphics device stopped: {e}"));
        }
        match out.again {
            Some(wait) if wait.is_zero() => again = true,
            Some(wait) => ui_due = Some(Instant::now() + wait),
            None => {}
        }
        // a toast is taken away when its time is up, even if nothing else happens
        if let Some((_, until)) = &toast {
            ui_due = Some(ui_due.map_or(*until, |due| due.min(*until)));
        }
        // fullscreen with the controls away is the frame and nothing else: no pointer either. where a drag would
        // draw a mark the pointer is a cross, and stays
        let pointer_now = match (out.cross, fullscreen && !out.shown) {
            (true, _) => window::Pointer::Cross,
            (false, true) => window::Pointer::Hidden,
            (false, false) => window::Pointer::Arrow,
        };
        if pointer_shown != pointer_now {
            pointer_shown = pointer_now;
            window::show_pointer(window, pointer_now);
        }

        // loop analysis asks, once for a clip, how much changes across its seam; the answer comes when it is read
        if let Some(spare) = &spare {
            // a clip that is another one now (a new tab up, a file opened or changed on disk) is let go first, and
            // the timebar's pictures with it: nothing is answered from the clip that was. a clip counts once its first
            // frame is up, and then its timebar strip is made: behind everything else, and not of a clip of one frame
            let up = tabs.tab().path.clone().filter(|_| tabs.tab().view.as_ref().is_some_and(|v| v.shown.is_some()));
            let asking = up.clone().zip(out.hovered);
            if up != time_of {
                spare.ask(Ask::Forget);
                if let Some(path) = up.as_ref().filter(|_| tabs.view().is_some_and(|v| v.clip.timeline.frames() > 1)) {
                    spare.ask(Ask::Strip(path.into()));
                }
                ui.forget_timebar_picture();
                (time_of, time_asked) = (up, None);
            }
            let tab = tabs.tab();
            if let (Some(path), Some(v)) = (&tab.path, &mut tab.view) {
                if v.transport.seam().is_some() && !v.seam_asked {
                    v.seam_asked = true;
                    spare.ask(Ask::Seam(path.into()));
                }
            }
            // the timebar asks for the frame under the pointer, each time that is another frame
            if let Some((path, frame)) = asking.clone().filter(|_| asking != time_asked) {
                spare.ask(Ask::Frame { path: path.into(), frame, playing: tabs.view().is_some_and(|v| v.transport.playing()) });
            }
            time_asked = asking;
        }
        // a folder's next clip comes nearer; and what play does at the clip's own ends follows the tab's loop mode
        // (standing still nothing else paces the loop, so the wait for it is taken here, a few milliseconds a look)
        let paced = playing || tabs.tab().cutting();
        match tabs.bring_in(&gpu, window, settings, &wake, if paced { Duration::ZERO } else { Duration::from_millis(4) }) {
            Ok(Some(said)) => {
                (redraw, again) = (true, true);
                // another file is up, or the same one changed: what the spare reader holds is the old one
                time_of = None;
                if !said.is_empty() {
                    toast = say(said);
                }
            }
            Ok(None) => {
                if let Some(coming) = tabs.tab().coming.as_mut().filter(|c| c.since.elapsed() >= SLOW && c.said.elapsed() >= SAY_EVERY) {
                    (coming.said, redraw) = (Instant::now(), true);
                }
            }
            Err(why) => toast = say(why),
        }
        tabs.park(&gpu, &wake);
        again |= tabs.settle(window);
        let tab = tabs.tab();
        if let Some(v) = &mut tab.view {
            let ends = folder::clip_loop(tab.mode, tab.played.as_ref().map_or(1, |p| p.items.len()), v.transport.shuttle_rate() < 0);
            if v.transport.seam().is_none() && v.transport.loop_mode() != ends {
                v.transport.set_loop(ends);
            }
        }
        // a clip's sound is looked for once its first frame is up: opening the file again, and the output device,
        // must not hold the picture back
        let tab = tabs.tab();
        if let (Some(path), Some(v)) = (&tab.path, &mut tab.view) {
            if v.sound.is_none() && v.shown.is_some() {
                if let Some(why) = v.open_sound(path) {
                    toast = say(format!("no sound: {why}"));
                }
            }
        }
        // the output device is opened once, while the first clip with sound stands still, so that the first play
        // does not wait for it (about 40 ms)
        if !speaker_open && !playing && tabs.tab().view.as_ref().is_some_and(|v| matches!(v.sound, Some(Some(_)))) {
            speaker_open = speaker.line().is_some();
        }
        // every clip's sound does what its transport does. only the tab that is up is heard, and not under the browser
        let up = tabs.up;
        let mut starting = false;
        for (i, tab) in tabs.all.iter_mut().enumerate() {
            let heard = i == up && !tab.browsing;
            if let Some(v) = &mut tab.view {
                starting |= v.sync_sound(heard, &mut speaker) && heard;
            }
        }

        // a frame that is nearly there is worth a short wait: while playing, half a refresh, since showing the old
        // frame again skips this one for good. standing still, a few milliseconds between looks at the window
        let playing = tabs.view().is_some_and(|v| v.transport.playing());
        // play that ran off a clip's end waits on its last frame for the folder's next clip, and counts the wait
        let cutting = tabs.tab().cutting();
        playing_now.store(playing || cutting, Ordering::Relaxed);
        let patience = if playing { Duration::from_secs_f64(0.5 / hz) } else { Duration::from_millis(4) };
        let changed = tabs.view().is_some_and(|v| {
            v.set_chroma(settings.chroma);
            v.load(&gpu, patience)
        });
        // the chip was laid out before this frame arrived: a new value under the pointer is one more look
        again |= tabs.view().is_some_and(|v| {
            let pixel = pixel_of(v);
            v.probe(&gpu, pixel)
        });
        if redraw || changed || playing || cutting {
            {
                // the decoder shares this device: its calls stay out of the middle of ours. the lock is let go before
                // the present, which waits for the display
                let _lock = gpu.locked();
                gpu.clear();
                if let Some(v) = tabs.view() {
                    v.draw(&gpu);
                }
                if let Err(e) = ui_painter.paint(&gpu, &out.primitives, out.scale, (gpu.w, gpu.h)) {
                    return window::set_title(window, &format!("eyepiece · the graphics device stopped: {e}"));
                }
            }
            let missed = gpu.present(playing, Duration::from_secs_f64(1.0 / hz));
            tabs.tab().held += cutting as u64;
            if let Some(v) = tabs.view() {
                // the pause on black between rounds of loop analysis shows no frame: nothing to be late with
                let black = v.transport.show() == Show::Black;
                // nor is a refresh late while the picture waits for its sound to start
                if playing && !black && !starting {
                    v.late += missed as u64 + v.waiting() as u64;
                }
                // only a refresh that showed what it was due counts toward the next one
                if black || !(v.waiting() || starting) {
                    v.transport.tick();
                }
                // play that ended by itself (looping off, at the clip's end) is one more look, for the controls and
                // the title
                again |= playing && !v.transport.playing();
            }
            // a folder's clip that played to its end hands over to the next
            if tabs.view().is_some_and(|v| playing && !v.transport.playing() && v.transport.frame() + 1 == v.clip.timeline.frames()) {
                tabs.advance(&gpu, &wake);
            }
            redraw = false;
            // the title follows the frame while the clip stands still. it is left alone during play: redrawing a
            // title bar makes the compositor miss a refresh now and then
            if !playing {
                let tab = &tabs.all[tabs.up];
                let title = match (&tab.view, &tab.error) {
                    (Some(v), _) => {
                        let (mode, folder) = tab.words();
                        // the marks on the frame on screen, and markup mode
                        let marks = match tab.marks().map_or(0, |m| m.on(v.transport.frame()).len()) {
                            0 => String::new(),
                            1 => " · 1 mark".to_string(),
                            n => format!(" · {n} marks"),
                        } + if ui.markup() { " · markup" } else { "" };
                        v.title(size, settings, (mode, &folder)) + &marks + if tab.browsing { " · browser" } else { "" }
                    }
                    (None, Some(why)) => format!("eyepiece · {why}{}", tab.words().1),
                    (None, None) => "eyepiece".to_string(),
                };
                if titled.as_ref() != Some(&title) {
                    window::set_title(window, &title);
                    titled = Some(title);
                }
            }
        }
        ui_painter.free_textures(&out.textures);
        // both halves are done with: egui checks that no change is dropped unseen
        let mut textures = out.textures;
        textures.clear();
        redraw |= again;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::color;
    use crate::media::source::Frame;

    /// chroma smaller than the picture is brought up to it by blending the four samples round each pixel, placed
    /// where the file sites them; on request the nearest sample is taken instead. every pixel is checked against the
    /// same arithmetic done here on the decoder's own planes: 4:2:0 in 8 bits, and 4:2:2 in 10 from two decoders.
    #[test]
    fn chroma_is_blended_at_the_files_siting_or_taken_from_the_nearest_sample() {
        let mut gpu = Gpu::offscreen().expect("a graphics device");
        (gpu.w, gpu.h) = (64, 64);
        for (name, shift, siting) in [("h264_64.mp4", (1, 1), (0.0, 0.5)), ("h264_422_64.mp4", (1, 0), (0.0, 0.0)), ("prores_64.mov", (1, 0), (0.0, 0.0))] {
            // decoded in software, so that the planes can be read here too
            let mut clip = Clip::open(&format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))).expect("open");
            clip.start(None).expect("start");
            let mut view = View::new(&gpu, clip, 60.0).expect("view");
            assert!(view.load(&gpu, Duration::from_secs(20)), "{name}: the first frame arrived");
            let Lane::Video(video) = &view.clip.lane else { panic!("a clip ffmpeg decodes") };
            let layout = video.layout;
            assert_eq!((layout.chroma_shift, layout.siting), (shift, siting), "{name}");
            let Frame::Ready(picture) = video.source.get(0, Duration::from_secs(20)) else { panic!("{name}: the first frame") };
            // a sample as the shader reads it: its texel as a part of the texel's range
            let wide = layout.stored.texel_bits > 8;
            let sample = |plane: usize, x: usize, y: usize| {
                let (data, stride, _) = picture.plane(plane, layout.chroma_shift);
                if wide { u16::from_le_bytes([data[y * stride + x * 2], data[y * stride + x * 2 + 1]]) as f32 / 65535.0 } else { data[y * stride + x] as f32 / 255.0 }
            };
            let (across, down) = (1usize << shift.0, 1usize << shift.1);
            let last = ((64 / across - 1) as f32, (64 / down - 1) as f32);
            let c = color::conversion(layout.stored, layout.matrix);
            let wanted = |x: usize, y: usize, bilinear: bool| -> [f32; 3] {
                let chroma = |plane: usize| {
                    if !bilinear {
                        return sample(plane, x / across, y / down);
                    }
                    let (ax, ay) = ((x as f32 - siting.0) / across as f32, (y as f32 - siting.1) / down as f32);
                    let at = |dx: f32, dy: f32| sample(plane, (ax.floor() + dx).clamp(0.0, last.0) as usize, (ay.floor() + dy).clamp(0.0, last.1) as usize);
                    let (wx, wy) = (ax - ax.floor(), ay - ay.floor());
                    let (top, bottom) = (at(0.0, 0.0) + (at(1.0, 0.0) - at(0.0, 0.0)) * wx, at(0.0, 1.0) + (at(1.0, 1.0) - at(0.0, 1.0)) * wx);
                    top + (bottom - top) * wy
                };
                let (luma, cb, cr) = (sample(0, x, y) * c.y_scale + c.y_offset, chroma(1) * c.c_scale + c.c_offset, chroma(2) * c.c_scale + c.c_offset);
                [luma + c.cr_to_r * cr, luma + c.cb_to_g * cb + c.cr_to_g * cr, luma + c.cb_to_b * cb].map(|v| v.clamp(0.0, 1.0))
            };
            let mut reads = Vec::new();
            for bilinear in [true, false] {
                view.paint.set_bilinear(bilinear);
                let read = view.read(&gpu, 64, 64).expect("the frame");
                let worst = (0..64 * 64).flat_map(|i| (0..3).map(move |channel| (i, channel))).map(|(i, channel)| (read[i * 4 + channel] as f32 / 65535.0 - wanted(i % 64, i / 64, bilinear)[channel]).abs()).fold(0.0, f32::max);
                assert!(worst < 1.0 / 255.0, "{name}, blended {bilinear}: the worst sample is {:.2} code values of 8 bits off", worst * 255.0);
                reads.push(read);
            }
            assert!(reads[0] != reads[1], "{name} has edges of color: the two ways differ");
            // the panel says which
            let nearest = Settings { chroma: settings::Chroma::Nearest, ..Settings::default() };
            assert_eq!((view.chroma(&Settings::default()).map(|said| said.0), view.chroma(&nearest)), (Some("blended, sited left".to_string()), Some(("the nearest sample".to_string(), false))), "{name}");
        }
    }

    /// a test clip with its first frame in the painter, with no window. needs a gpu.
    fn view(name: &str) -> (Gpu, View) {
        open(&format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR")))
    }

    /// `view`, for a file anywhere
    fn open(path: &str) -> (Gpu, View) {
        // a device that draws only into pictures of its own, told the size a window would have
        let mut gpu = Gpu::offscreen().expect("a graphics device");
        (gpu.w, gpu.h) = (64, 64);
        let clip = Clip::open(path).expect("open");
        let mut view = View::new(&gpu, clip, 60.0).expect("view");
        assert!(view.load(&gpu, Duration::from_secs(20)), "the first frame arrived");
        (gpu, view)
    }

    /// the pixel readout gives the file's pixel, whatever the window shows: each one read back matches ffmpeg's
    /// decode of the same hap frame, within the few code values by which two bc1 decoders round.
    #[test]
    fn a_pixel_read_back_is_the_files_pixel() {
        let (gpu, mut view) = view("hap_64.mov");
        // zoomed and pushed aside: the reading must not care
        view.framing.set_zoom(Some(8.0), Some((3.0, 60.0)), (64.0, 64.0));
        let referee = std::fs::read(format!("{}/tests/media/hap_64.frame0.rgb", env!("CARGO_MANIFEST_DIR"))).expect("referee");
        // every seventh pixel each way: the four corners are among them
        for (x, y) in (0..64).step_by(7).flat_map(|y| (0..64).step_by(7).map(move |x| (x, y))) {
            assert!(view.probe(&gpu, Some((x, y))), "pixel {x},{y} was read");
            let (_, at, value) = view.probe.expect("a value");
            assert_eq!(at, (x, y));
            let wanted = &referee[(y * 64 + x) * 3..][..3];
            let worst = (0..3).map(|c| value[c].abs_diff(wanted[c] as u32)).max().unwrap();
            assert!(worst <= 3, "pixel {x},{y}: read {value:?}, ffmpeg decodes {wanted:?}");
        }
        // the same pixel of the same frame is not read twice; with no pixel asked for, nothing is kept
        assert!(!view.probe(&gpu, Some((63, 63))));
        assert!(!view.probe(&gpu, None) && view.probe.is_none());
    }

    /// a saved frame carries the marks drawn on it, over the very pixels of the file they were drawn on, and the
    /// caption in a band under the picture, which covers none of it. the copy is the frame alone.
    #[test]
    fn a_saved_frame_carries_its_marks_and_a_caption_under_the_picture() {
        use crate::markup::{Mark, Shape, INKS};
        let (gpu, view) = view("hap_64.mov");
        let plain = view.read(&gpu, 64, 64).expect("the frame");
        let mut marks = Marks::default();
        marks.add(0, Mark { shape: Shape::Rectangle((10.0, 10.0), (40.0, 40.0)), ink: INKS[0] });
        let settings = Settings { screenshot_caption: true, screenshot_markup: true, ..Settings::default() };
        let ((w, h), pixels, with) = shot_pixels(&view, Some(&marks), &gpu, Shot::Frame, &settings, 1.0).expect("a picture");
        assert_eq!((w, with), (64, vec!["caption", "markup"]));
        // a 64-wide frame takes the window's own size of ink and type: the caption's line wraps, under the picture
        assert!(h > 64 + 29, "{h} high");
        let at = |pixels: &[u16], x: usize, y: usize| -> [i32; 4] { std::array::from_fn(|c| pixels[(y * w as usize + x) * 4 + c] as i32) };
        let close = |a: [i32; 4], b: [i32; 4]| (0..3).all(|c| (a[c] - b[c]).abs() < 700);
        // the rectangle's left side runs down x = 10 of the file, 2.5 wide: pixels 9 and 10 are its ink
        let red = [65535, 0x3b * 257, 0x30 * 257, 65535];
        for y in [15, 25, 35] {
            assert!(close(at(&pixels, 9, y), red) && close(at(&pixels, 10, y), red), "at y {y}: {:?} {:?}", at(&pixels, 9, y), at(&pixels, 10, y));
            // and the file's own pixels on each side of it
            assert_eq!(at(&pixels, 7, y), at(&plain, 7, y));
            assert_eq!(at(&pixels, 12, y), at(&plain, 12, y));
        }
        // under the picture, the caption's band
        assert!(close(at(&pixels, 1, 66), [0x11 * 257; 4]), "{:?}", at(&pixels, 1, 66));
        let (_, copy, with) = shot_pixels(&view, Some(&marks), &gpu, Shot::Copy, &settings, 1.0).expect("a copy");
        assert!(copy == plain && with.is_empty());
    }

    /// the view saved as shown carries the marks where the window shows them, mirrored with the picture, and the
    /// guides: here the center line, down the middle.
    #[test]
    fn a_saved_view_carries_the_marks_and_the_guides_as_the_window_shows_them() {
        use crate::markup::{Mark, Shape, INKS};
        let (gpu, mut view) = view("hap_64.mov");
        view.set_look(Look { mirror: true, ..Look::default() });
        let mut marks = Marks::default();
        marks.add(0, Mark { shape: Shape::Rectangle((10.0, 10.0), (40.0, 40.0)), ink: INKS[0] });
        let settings = Settings { center: true, screenshot_guides: true, screenshot_markup: true, ..Settings::default() };
        let ((w, h), pixels, with) = shot_pixels(&view, Some(&marks), &gpu, Shot::View, &settings, 1.0).expect("a picture");
        assert_eq!((w, h, with), (64, 64, vec!["guides", "markup"]));
        let at = |x: usize, y: usize| -> [i32; 3] { std::array::from_fn(|c| pixels[(y * 64 + x) * 4 + c] as i32) };
        let close = |a: [i32; 3], b: [i32; 3]| (0..3).all(|c| (a[c] - b[c]).abs() < 700);
        // the rectangle's left side, x = 10 of the file, is shown mirrored at x = 54 of the window
        let side = Placing { picture: view.framing.rect((64.0, 64.0)), look: view.look, size: view.size() }.to_window((10.0, 25.0));
        assert_eq!(side, (54.0, 25.0));
        let red = [65535, 0x3b * 257, 0x30 * 257];
        assert!(close(at(53, 25), red) && close(at(54, 25), red), "{:?} {:?}", at(53, 25), at(54, 25));
        assert!(!close(at(9, 25), red) && !close(at(10, 25), red), "nothing where the file has it unmirrored");
        // the center line, one pixel wide at 100%, in the guides' color
        assert!(close(at(32, 5), [0x3d * 257, 0xf0 * 257, 0xff * 257]), "{:?}", at(32, 5));
    }

    /// what a frame read back holds of the file's alpha, by lane: a still's own (here a png with clear pixels), which a
    /// saved frame keeps, hap alpha's (the test clip's is opaque all over), and prores 4444's, from its own plane;
    /// none for hap q and the other y'cbcr lanes.
    #[test]
    fn a_frame_read_back_holds_the_files_alpha_where_the_lane_draws_it() {
        // 16 x 16, its top row clear, the next half clear, the rest opaque
        let dir = std::env::temp_dir().join(format!("eyepiece-alpha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clear.png");
        let stored: Vec<u8> = (0..16 * 16).flat_map(|i| [200, 100, 50, if i < 16 { 0 } else if i < 32 { 128 } else { 255 }]).collect();
        let mut encoder = png::Encoder::new(std::fs::File::create(&file).unwrap(), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&stored).unwrap();
        writer.finish().unwrap();
        {
            let (gpu, view) = open(&file.to_string_lossy());
            assert!(view.carries_alpha());
            let (_, read, _) = shot_pixels(&view, None, &gpu, Shot::Frame, &Settings::default(), 1.0).expect("the frame");
            assert_eq!([read[3], read[16 * 4 + 3], read[32 * 4 + 3]], [0, 128 * 257, 65535]);
            assert!(shot::transparent(&read));
        }
        // a failed test leaves the file for a look
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&dir).unwrap();
        assert!(view("hapalpha_64.mov").1.carries_alpha());
        for name in ["hapq_64.mov", "prores_64.mov", "h264_64.mp4"] {
            let (gpu, opaque) = view(name);
            assert!(!opaque.carries_alpha(), "{name}");
            assert!(!shot::transparent(&opaque.read(&gpu, 64, 64).expect("the frame")), "{name}: an opaque lane read back clear pixels");
        }
        // prores 4444 keeps alpha in a plane of its own. this clip's is opaque all over
        let (gpu, full) = view("prores4444_64.mov");
        assert!(full.carries_alpha() && !shot::transparent(&full.read(&gpu, 64, 64).expect("the frame")));
        // and this one's runs from clear at the left by 16 of 1023 a pixel
        let (gpu, ramp) = view("prores4444a_64.mov");
        let read = ramp.read(&gpu, 64, 64).expect("the frame");
        for x in [0usize, 1, 20, 63] {
            let (got, stored) = (read[(10 * 64 + x) * 4 + 3] as f32 / 65535.0, x as f32 * 16.0 / 1023.0);
            assert!((got - stored).abs() < 1.0 / 255.0, "alpha at x {x} reads {got}, and the file has {stored}");
        }
        assert!(ramp.carries_alpha() && shot::transparent(&read));
    }
}
