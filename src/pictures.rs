//! the maker of the browser's pictures: a thread with a graphics device of its own and no window, and
//! one clip open at a time, read through the same lanes and shaders as the window's clips (`player::View`).

use crate::player::View;
use crate::sys::Gpu;
use crate::media::clip::{Clip, Lane};
use crate::event::Event;
use crate::preview::make::{self, Pictures, Reader};
use crate::preview::spare::{self, Spare};
use crate::preview::Cache;
use std::cell::OnceCell;
use std::mem::ManuallyDrop;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// how long a frame is waited for before the picture is left out: a long group of 4K frames decodes within it
const WAIT: Duration = Duration::from_secs(5);

/// starts the maker. it makes nothing while `playing` is set. None when its thread will not start.
pub fn start(playing: Arc<AtomicBool>) -> Option<Pictures> {
    let (pictures, wants, got) = Pictures::new();
    let work = move || {
        // the device is made when the first picture is wanted, not at launch: a player that never shows its browser
        // never pays for it. with no device there are no pictures, and the cards stay plain
        let gpu: OnceCell<Option<Gpu>> = OnceCell::new();
        make::serve(wants, got, Cache::open(), &playing, |path| Open::new(gpu.get_or_init(|| Gpu::offscreen().ok()).as_ref()?, path));
    };
    std::thread::Builder::new().name("pictures".into()).spawn(work).ok().map(|_| pictures)
}

/// starts the spare reader of the clip on screen, which wakes the window when it has an answer, and makes no timebar
/// strip while `playing` is set. its device too is made when first needed. None when its thread will not start.
pub fn start_spare(wake: Sender<Event>, playing: Arc<AtomicBool>) -> Option<Spare> {
    let (spare, asks, answers) = Spare::new();
    let work = move || {
        let gpu: OnceCell<Option<Gpu>> = OnceCell::new();
        let told = || {
            let _ = wake.send(Event::Wake);
        };
        spare::serve(asks, answers, &playing, told, |path| Open::new(gpu.get_or_init(|| Gpu::offscreen().ok()).as_ref()?, path).map(Open::keys_only));
    };
    std::thread::Builder::new().name("spare reader".into()).spawn(work).ok().map(|_| spare)
}

struct Open<'a> {
    gpu: &'a Gpu,
    view: ManuallyDrop<View>,
}

impl Drop for Open<'_> {
    /// the clip goes first, then the device is flushed: it has no window and never presents, and without a flush
    /// what the clip's decoder held on it stayed taken
    fn drop(&mut self) {
        // safe: the view is not touched again
        unsafe { ManuallyDrop::drop(&mut self.view) };
        self.gpu.flush();
    }
}

impl<'a> Open<'a> {
    fn new(gpu: &'a Gpu, path: &Path) -> Option<Open<'a>> {
        // a file that would have to be read through to count its frames gets no picture for that
        let mut clip = Clip::open_known(path.to_str()?).ok()?;
        // one picture at a time and never a step back
        clip.keep_nothing();
        // the rate of the display is nothing to a clip that never plays
        let view = View::new(gpu, clip, 60.0).ok()?;
        // one frame at a time, and none kept: 24 frames of a 4K float sequence would be 3 GB
        match &view.clip.lane {
            Lane::Hap(hap) => hap.source.keep_only_wanted(),
            Lane::Video(video) => video.source.keep_only_wanted(),
            Lane::Stills(stills) => stills.source.keep_only_wanted(),
            Lane::Waiting(_) => {}
        }
        Some(Open { gpu, view: ManuallyDrop::new(view) })
    }

    /// for the timebar's reader, which asks a clip with long groups for its key frames alone
    /// (`VideoLane::keys_only`).
    fn keys_only(self) -> Self {
        if let Lane::Video(video) = &self.view.clip.lane {
            video.keys_only();
        }
        self
    }
}

impl Reader for Open<'_> {
    fn frames(&self) -> usize {
        self.view.clip.timeline.frames()
    }

    fn size(&self) -> (u32, u32) {
        let (w, h) = self.view.clip.upright();
        (w as u32, h as u32)
    }

    fn rate(&self) -> f64 {
        self.view.clip.timeline.fps()
    }

    fn keys(&self) -> Option<Vec<usize>> {
        match &self.view.clip.lane {
            Lane::Video(video) if !video.alone => Some(video.keys()),
            Lane::Video(_) | Lane::Hap(_) | Lane::Stills(_) => None,
            Lane::Waiting(_) => Some(Vec::new()),
        }
    }

    fn read(&mut self, frame: usize, w: u32, h: u32) -> Option<Vec<u16>> {
        self.view.transport.go(frame as i64);
        // this frame and no other: nothing is read ahead for a picture
        let until = Instant::now() + WAIT;
        while self.view.waiting() && Instant::now() < until {
            self.view.load_wanting(self.gpu, Duration::from_millis(50), &[frame]);
        }
        if self.view.waiting() || self.view.fault.is_some() {
            return None;
        }
        self.view.read(self.gpu, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::spare::{Answer, Ask};
    use std::path::PathBuf;

    fn media(name: &str) -> PathBuf {
        PathBuf::from(format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR")))
    }

    /// the timebar's reader of a clip, on a thread of its own as the window's is, and standing still
    fn timebar_reader() -> Spare {
        let (spare, asks, answers) = Spare::new();
        std::thread::spawn(move || {
            let gpu = Gpu::offscreen().expect("a graphics device");
            spare::serve(asks, answers, &AtomicBool::new(false), || {}, |path| Open::new(&gpu, path).map(Open::keys_only));
        });
        spare
    }

    /// the next answer, to the millisecond or so, or None when there is none for so long: what the window has already
    /// is not read again
    fn answered(spare: &Spare, wait: Duration) -> Option<Answer> {
        let until = Instant::now() + wait;
        while Instant::now() < until {
            if let Some(answer) = spare.take() {
                return Some(answer);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        None
    }

    /// the timebar's pictures of real clips, timed at the reader, standing still, as the window asks for them: ten
    /// frames under the pointer spread over the clip, each asked for alone and timed to its picture (the frame, or
    /// the key frame nearest it), before there is a strip; then the strip (how many pictures, the room they take,
    /// and how long to the first and to the whole); then the ten again, of which those the strip has are not read.
    /// the clips are the files in EYEPIECE_HOVER, separated by `;`:
    /// `cargo test --release timebar_pictures_are_timed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn the_timebar_pictures_are_timed() {
        let gpu = Gpu::offscreen().expect("a graphics device");
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        for path in std::env::var("EYEPIECE_HOVER").expect("EYEPIECE_HOVER names the clips").split(';').map(PathBuf::from) {
            let frames = Open::new(&gpu, &path).expect("it opens").frames();
            let spare = timebar_reader();
            let hovers = |when: &str| {
                let (mut times, mut had) = (Vec::new(), 0);
                for i in 0..10 {
                    let asked = Instant::now();
                    spare.ask(Ask::Frame { path: path.clone(), frame: (2 * i + 1) * frames / 20, playing: false, exact: false });
                    match answered(&spare, Duration::from_secs(2)) {
                        Some(Answer::Frame { frame, .. }) => times.push(format!("{:.0} ({frame})", ms(asked.elapsed()))),
                        _ => had += 1,
                    }
                }
                println!("  under the pointer, {when}: ms (frame) {}; {had} had already", times.join(" "));
            };
            println!("{}: {frames} frames", path.display());
            hovers("with no strip");
            let asked = Instant::now();
            spare.ask(Ask::Strip(path.clone()));
            let (mut made, mut bytes, mut first, mut whole) = (0, 0, None, Duration::ZERO);
            while let Some(Answer::Frame { pixels, .. }) = answered(&spare, Duration::from_secs(5)) {
                whole = asked.elapsed();
                first.get_or_insert(whole);
                (made, bytes) = (made + 1, bytes + pixels.len());
            }
            println!("  strip of {made} in {:.1} MB, the first in {:.0} ms, all in {:.0} ms", bytes as f64 / 1e6, ms(first.unwrap_or_default()), ms(whole));
            hovers("with the strip");
        }
    }

    /// what a marker of the trim costs at the reader, on the same clips: the very frame asked for, each timed to its
    /// picture. the last frame of the clip's longest group asked from the clip's start, which costs the most; thirty
    /// frames back from it a frame at a time, as a slow drag asks for them, and the thirty forward again; then a fast
    /// drag, 120 frames asked 4 ms apart, which is faster than they are read: how many were read, and how long after
    /// the last was asked for its picture came.
    /// `cargo test --release a_markers_frames_are_timed -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn a_markers_frames_are_timed() {
        let gpu = Gpu::offscreen().expect("a graphics device");
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        for path in std::env::var("EYEPIECE_HOVER").expect("EYEPIECE_HOVER names the clips").split(';').map(PathBuf::from) {
            let (frames, keys) = {
                let open = Open::new(&gpu, &path).expect("it opens");
                (open.frames(), open.keys().unwrap_or_default())
            };
            let groups = keys.iter().zip(keys.iter().skip(1).chain([&frames])).map(|(key, next)| (next - key, next - 1));
            let (longest, deep) = groups.max().unwrap_or((1, frames / 2));
            let spare = timebar_reader();
            let ask = |frame: usize| spare.ask(Ask::Frame { path: path.clone(), frame, playing: false, exact: true });
            let read = |frame: usize| {
                let asked = Instant::now();
                ask(frame);
                match answered(&spare, Duration::from_secs(6)) {
                    Some(Answer::Frame { frame: at, .. }) if at == frame => ms(asked.elapsed()),
                    _ => panic!("no picture of frame {frame}"),
                }
            };
            let spread = |mut times: Vec<f64>| {
                times.sort_by(f64::total_cmp);
                format!("median {:.0} ms, most {:.0} ms", times[times.len() / 2], times[times.len() - 1])
            };
            println!("{}: {frames} frames, its longest group {longest}", path.display());
            println!("  frame 0, which opens the clip: {:.0} ms", read(0));
            println!("  frame {deep}, the last of that group: {:.0} ms", read(deep));
            let far = deep.saturating_sub(30);
            println!("  back a frame at a time to {far}: {}", spread((far..deep).rev().map(read).collect()));
            println!("  and forward again: {}", spread((far + 1..=deep).map(read).collect()));
            // (to the frame before it: the reader does not read again the picture it answered last)
            let (from, to) = (deep.saturating_sub(121), deep - 1);
            for frame in from..to {
                ask(frame);
                std::thread::sleep(Duration::from_millis(4));
            }
            let asked = Instant::now();
            ask(to);
            let mut pictures = 0;
            while let Some(Answer::Frame { frame, .. }) = answered(&spare, Duration::from_secs(6)) {
                pictures += 1;
                if frame == to {
                    break;
                }
            }
            println!("  a fast drag from {from} to {to}: {pictures} pictures read, the last {:.0} ms after it was asked for", ms(asked.elapsed()));
        }
    }

    /// clips with long groups: h.264 with a key frame every 8, shown where the file places them, and hevc from x265
    /// with one every 24, whose open groups store after each key frame but the first a picture that is shown before
    /// it and needs the group before (key frames at places 23 and 47 are frames 24 and 48). the strip (a frame every
    /// two seconds: the key frames nearest 0 and 60) and the pictures for frames under the pointer are each the very
    /// frame they say, and none costs a group: in h.264 the reader decodes on from frame 1 to 16 (fewer than 16 to go)
    /// and seeks back to 8; in hevc it seeks to the key frame at place 47, whose first picture is frame 48, and takes
    /// that rather than decode the group before for frame 47, and so on to 24. a picture had already is not read again
    #[test]
    fn the_timebar_gets_key_frames_alone_of_a_clip_with_long_groups() {
        let gpu = Gpu::offscreen().expect("a graphics device");
        let clips = [("h264_320.mp4", vec![15, 5, 7, 9], vec![(0, true), (16, false), (8, false)]), ("hevc_open_320.mp4", vec![20, 26, 60, 5], vec![(48, true), (0, true), (24, false)])];
        for (name, hovers, wanted) in clips {
            let spare = timebar_reader();
            spare.ask(Ask::Strip(media(name)));
            let mut got = Vec::new();
            let mut take = |wait| {
                while let Some(Answer::Frame { frame, pixels, strip, .. }) = answered(&spare, wait) {
                    let exact = make::small(&mut Open::new(&gpu, &media(name)).expect("it opens"), frame).expect("the frame").1;
                    assert!(pixels == exact, "{name}: the picture given as frame {frame} is another frame");
                    got.push((frame, strip));
                }
            };
            take(Duration::from_secs(3));
            for frame in hovers {
                spare.ask(Ask::Frame { path: media(name), frame, playing: false, exact: false });
                take(Duration::from_secs(1));
            }
            assert_eq!(got, wanted, "{name}");
        }
    }

    /// a marker of the trim shows the frame it names: asked for exactly, the reader of key frames gives the frame
    /// itself, decoded from the key frame before it, in the same clips. going on, going back, a key frame, the last
    /// frame. in the hevc, places 23 and 47 hold key frames that are shown as frames 24 and 48, and frames 23 and 47
    /// are the group before's. each is that frame when it is the first thing asked of its place (47), when the pointer
    /// was just near it and got the key frame for it (22, then 23), and once the reader knows (23 again)
    #[test]
    fn the_timebar_gets_the_very_frame_asked_for_exactly_of_a_clip_with_long_groups() {
        let gpu = Gpu::offscreen().expect("a graphics device");
        let exactly = |frames: &[usize]| frames.iter().map(|&frame| (frame, true, frame)).collect::<Vec<_>>();
        let hevc = [exactly(&[47]), vec![(22, false, 24)], exactly(&[23, 20, 26, 24, 23, 71, 48])].concat();
        for (name, asks) in [("h264_320.mp4", exactly(&[15, 5, 8, 7, 23, 0])), ("hevc_open_320.mp4", hevc)] {
            let spare = timebar_reader();
            // a reader of every frame, as a card's is, to tell each picture by
            let mut every = Open::new(&gpu, &media(name)).expect("it opens");
            for (frame, exact, wanted) in asks {
                spare.ask(Ask::Frame { path: media(name), frame, playing: false, exact });
                let Some(Answer::Frame { frame: at, pixels, strip, .. }) = answered(&spare, Duration::from_secs(10)) else { panic!("{name}: no picture of frame {frame}") };
                assert_eq!((at, strip), (wanted, false), "{name}: asked for frame {frame}");
                assert!(pixels == make::small(&mut every, at).expect("the frame").1, "{name}: the picture given as frame {at} is another frame");
            }
        }
    }

    /// a real clip's seam: its last frame against its first differs, and a frame against itself does not.
    #[test]
    fn a_clips_last_frame_is_compared_with_its_first() {
        let gpu = Gpu::offscreen().expect("a graphics device");
        let mut open = Open::new(&gpu, Path::new(&format!("{}/tests/media/hapq_64.mov", env!("CARGO_MANIFEST_DIR")))).expect("it opens");
        let (last, first) = (open.read(open.frames() - 1, 64, 64).expect("the last frame"), open.read(0, 64, 64).expect("the first"));
        let seam = spare::compare(&last, &first);
        assert!(!seam.identical && seam.change > 0.0, "{seam:?}");
        assert!(spare::compare(&first, &open.read(0, 64, 64).expect("the first again")).identical);
    }

    /// a clip whose file asks to be turned is read upright, for its card and the timebar: its size turned, and each
    /// pixel the one the same clip untagged has there once turned a quarter clockwise.
    #[test]
    fn a_clip_the_file_turns_is_read_upright() {
        let gpu = Gpu::offscreen().expect("a graphics device");
        let mut plain = Open::new(&gpu, &media("h264_320.mp4")).expect("it opens");
        let mut turned = Open::new(&gpu, &media("h264_320_turn90.mp4")).expect("it opens");
        assert_eq!((plain.size(), turned.size()), ((320, 180), (180, 320)));
        let (plain, turned) = (plain.read(5, 320, 180).expect("the frame"), turned.read(5, 180, 320).expect("the frame"));
        for (x, y) in (0..180).flat_map(|x| (0..320).map(move |y| (x, y))) {
            assert_eq!(turned[(y * 180 + x) * 4..][..4], plain[((179 - x) * 320 + y) * 4..][..4], "pixel {x},{y}");
        }
    }

    /// a picture of a real clip, through a device with no window: the frame asked for, at the size asked for.
    #[test]
    fn a_frame_is_read_with_no_window() {
        let gpu = Gpu::offscreen().expect("a graphics device");
        for name in ["hapq_64.mov", "h264_64.mp4", "prores_64.mov", "still_8bit.png"] {
            let mut open = Open::new(&gpu, Path::new(&format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR")))).unwrap_or_else(|| panic!("{name} opens"));
            let last = open.frames() - 1;
            let picture = open.read(last, 32, 32).unwrap_or_else(|| panic!("{name}: its last frame is read"));
            assert_eq!(picture.len(), 32 * 32 * 4, "{name}");
            // a picture, not a blank: some pixel differs from the first
            assert!(picture.chunks_exact(4).any(|p| p[..3] != picture[..3]), "{name} is one flat color");
        }
    }
}
