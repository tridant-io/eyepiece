//! the controls: drawn by us, to the colors, type and measurements of the approved design (docs/design/plan.html).
//! platform-neutral: the window reports the pointer and the keys, this returns triangles for the renderer and what
//! was asked for. one unit here is one of the design's pixels; the display's scale turns it into physical pixels.

mod brand;
mod browser;
mod icons;
mod keys;
mod markup;
mod menus;
mod panel;
mod update;
mod widgets;

pub use markup::{caption, saved};
pub use update::{Applying, Updates};

use crate::event::{Key, Mods};
use crate::markup::{Mark, Marks};
use crate::media::meta::Meta;
use crate::settings::{Loop, Settings};
use crate::timecode::Timecode;
use crate::preview::make::Pictures;
use crate::preview::spare::Seam;
use crate::view::{Guides, Look};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontFamily, FontId, Pos2, Rect, Sense, Vec2};
use icons::Icon;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---- the design's tokens for inside the app window: neutral on purpose (no accent on the strip or the bar)
const BAR: Color32 = Color32::from_rgba_premultiplied(16, 16, 16, 230); // rgba(18,18,18,.9)
const SOLID: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
const LINE: Color32 = Color32::from_rgba_premultiplied(33, 33, 33, 33); // rgba(255,255,255,.13)
const FG: Color32 = Color32::from_rgb(0xed, 0xed, 0xed);
const DIM: Color32 = Color32::from_rgb(0x9c, 0x9c, 0x9c);
/// the accent: only on sliders, switches and the focused field inside menus
const ACCENT: Color32 = Color32::from_rgb(0xff, 0x5a, 0x36);
const AMBER: Color32 = Color32::from_rgb(0xff, 0xc4, 0x6b);
const HOVER: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26); // rgba(255,255,255,.1)
const PRESSED: Color32 = Color32::from_rgba_premultiplied(51, 51, 51, 51); // rgba(255,255,255,.2)
const TRACK: Color32 = Color32::from_rgba_premultiplied(51, 51, 51, 51); // rgba(255,255,255,.2)
const FILL: Color32 = Color32::from_rgba_premultiplied(140, 140, 140, 140); // rgba(255,255,255,.55)
const BANNER: Color32 = Color32::from_rgba_premultiplied(19, 19, 19, 242); // rgba(20,20,20,.95)
const CHIP: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 184); // rgba(0,0,0,.72)
const TAB_ON: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x2e);
const TAB_OFF: Color32 = Color32::from_rgb(0x7d, 0x7d, 0x7d);
/// the strip over the browser, which is not see-through
const STRIP_SOLID: Color32 = Color32::from_rgb(0x14, 0x14, 0x14);
const MESSAGE: Color32 = Color32::from_rgb(0x0b, 0x0b, 0x0b);

// ---- the design's measurements
const STRIP: f32 = 32.0;
const WINDOW_BUTTON: f32 = 40.0;
const TAB_MAX: f32 = 310.0;
/// a tab's width without its name: 12 before, 8 to the close mark, 12 of mark, 8 after
const TAB_PAD: f32 = 40.0;
/// tabs narrow to this before they run under the window's buttons
const TAB_MIN: f32 = 64.0;
/// room each side of the arrows on a folder's tab
const NAV_PAD: f32 = 5.0;
const PLUS: f32 = 32.0;
const ROW: f32 = 36.0;
const TIME: f32 = 14.0;
const TRACK_THIN: f32 = 4.0;
const TRACK_THICK: f32 = 8.0;
const ICON_BUTTON: Vec2 = Vec2::new(30.0, 28.0);
const TEXT_BUTTON: f32 = 28.0;
const GAP: f32 = 2.0;
const PAD: f32 = 8.0;
/// under this width the bar's labels are shortened
const NARROW: f32 = 760.0;
/// the controls fade in and out over this long (and go away as long after the pointer stops as the settings say)
const FADE: f32 = 0.14;
/// the zoom chip goes this long after the pointer rests
const CHIP_IDLE: f64 = 2.0;
/// the longer side of the navigator
const NAVIGATOR: f32 = 112.0;
/// the metadata panel comes up this long after the pointer comes onto its button, and goes this long after it has
/// left both the button and the panel
const FACTS_IN: f64 = 0.35;
const FACTS_OUT: f64 = 0.18;
/// how long a word about what a key did stays at the lower left
const FLASH: f64 = 1.1;
/// a button's tooltip comes this long after the pointer comes onto it
const TIP_IN: f64 = 0.5;

const MONO: &str = "mono";
const MONO_MEDIUM: &str = "mono-medium";
const SANS: &str = "sans";

fn font(family: &str, size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(family.into()))
}

/// the design's type: ibm plex mono and archivo, each weight under its own name.
fn fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::empty();
    let files: [(&str, &'static [u8]); 4] = [
        (MONO, include_bytes!("../../assets/fonts/ibm-plex-mono-400.ttf")),
        (MONO_MEDIUM, include_bytes!("../../assets/fonts/ibm-plex-mono-500.ttf")),
        (SANS, include_bytes!("../../assets/fonts/archivo-400.ttf")),
        ("sans-semibold", include_bytes!("../../assets/fonts/archivo-600.ttf")),
    ];
    for (name, bytes) in files {
        fonts.font_data.insert(name.to_string(), Arc::new(egui::FontData::from_static(bytes)));
        fonts.families.insert(FontFamily::Name(name.into()), vec![name.to_string()]);
    }
    // the two families every text falls back to
    fonts.families.insert(FontFamily::Proportional, vec![SANS.to_string()]);
    fonts.families.insert(FontFamily::Monospace, vec![MONO.to_string()]);
    fonts
}

/// our type, and after it the system's own for the letters ours lacks: the two faces hold western european
/// letters, and a folder named in polish came out with an empty box. the system's ui face (segoe ui on windows, san
/// francisco on macos) is read from the system's own folder when the controls are made; it adds the rest of
/// latin, greek and cyrillic. a name in a script it lacks too is still drawn with boxes.
fn fonts_with_system() -> egui::FontDefinitions {
    let mut fonts = fonts();
    let file = if cfg!(windows) {
        std::env::var_os("WINDIR").map(|windows| std::path::Path::new(&windows).join("Fonts").join("segoeui.ttf"))
    } else {
        Some(PathBuf::from("/System/Library/Fonts/SFNS.ttf"))
    };
    if let Some(bytes) = file.and_then(|file| std::fs::read(file).ok()) {
        fonts.font_data.insert("system".to_string(), Arc::new(egui::FontData::from_owned(bytes)));
        for family in fonts.families.values_mut() {
            family.push("system".to_string());
        }
    }
    fonts
}

/// where the top of a line of this type goes for the line to look centered on `middle`: by eye, which takes the
/// middle of the figures and capitals for the middle of the text. the line's own box has more room above the letters
/// than below them, so centering the box leaves the text low.
fn text_top(painter: &egui::Painter, font_id: FontId, middle: f32) -> f32 {
    let figure = painter.layout_no_wrap("0".to_string(), font_id, Color32::WHITE);
    let glyph = &figure.rows[0].row.glyphs[0];
    middle - (glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y / 2.0)
}

/// what is on screen, read from the player each frame: the tabs, and the one that is up.
pub struct Screen<'a> {
    /// every tab's name, and which one is up
    pub tabs: Vec<&'a str>,
    pub current: usize,
    /// for each tab that plays a folder, by the same place: where it is in the folder
    pub places: Vec<Option<Place<'a>>>,
    /// its clip, when it has one
    pub clip: Option<Model<'a>>,
    /// why its file did not open, when it did not
    pub error: Option<&'a str>,
    /// a file is on its way to a tab that shows nothing else: what to say meanwhile, which is nothing at first
    pub opening: Option<&'a str>,
    /// the folder the tab that is up plays, when it plays one
    pub list: Option<FolderList<'a>>,
    /// the browser is up: in a tab with nothing in it, or laid over the clip
    pub browsing: bool,
    pub fullscreen: bool,
    /// what just happened, said low in the middle of the window for a moment
    pub toast: Option<&'a str>,
    /// what is kept between runs, which the settings panel shows and changes
    pub settings: &'a Settings,
}

/// the facts for the metadata panel, which are read in the background.
pub enum Facts<'a> {
    Reading,
    Ready(&'a Meta),
    Failed(&'a str),
}

/// which picture a screenshot takes, or the frame to the clipboard
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shot {
    /// the file's own frame, at full size
    Frame,
    /// the picture as the window shows it
    View,
    Copy,
}

/// what a saved screenshot carries besides the picture: the screenshot menu's three switches
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extras {
    /// the clip's name, the frame, the timecode, the size and the rate, under the picture
    pub caption: bool,
    /// the guides, when they are showing
    pub guides: bool,
    pub markup: bool,
}

/// a picture for the timebar: the frame it is of, its width and height, and its red, green and blue, a byte each
type TimePicture = (usize, [usize; 2], Vec<u8>);

/// where a tab that plays a folder is in it: the folder's name, the clip that is up, how many there are, and the
/// clip's name
#[derive(Clone, Copy)]
pub struct Place<'a> {
    pub folder: &'a str,
    pub at: usize,
    pub count: usize,
    pub clip: &'a str,
}

/// a played folder, for its list: its name, its clips in order, and which of them is up
pub struct FolderList<'a> {
    pub title: &'a str,
    pub at: usize,
    pub items: Vec<ListItem<'a>>,
}

/// one clip of a played folder: its name, and what its header says, as far as that has been read
pub struct ListItem<'a> {
    pub name: &'a str,
    pub size: Option<(usize, usize)>,
    pub rate: Option<&'a str>,
    pub frames: Option<usize>,
    /// its size, or its rate, is not the folder's first clip's
    pub odd_size: bool,
    pub odd_rate: bool,
}

/// the clip on screen, as the controls show it.
pub struct Model<'a> {
    pub frame: usize,
    pub frames: usize,
    pub playing: bool,
    /// frames a second as the file's own ratio, when it has one rate
    pub rate: Option<(i64, i64)>,
    /// what the frames are called by the clock: the file's own timecode, or counted from 00:00:00:00
    pub clock: Option<Timecode>,
    /// the number the first frame gets, 0 or 1
    pub first: usize,
    /// why the frame on screen cannot be shown, when it cannot
    pub fault: Option<&'a str>,
    /// a sequence's frames that have no file
    pub gaps: &'a [usize],
    /// the file holds linear light (exr), shown through the srgb curve
    pub linear: bool,
    pub looping: Loop,
    /// the tab plays a folder: "all" is then one of its loop modes
    pub folder: bool,
    /// how the clip's last frame and its first compare, once that has been measured for loop analysis
    pub seam_change: Option<Seam>,
    /// loop analysis, while it is on: the place in its round and the round's length, in frames
    pub seam: Option<(usize, usize)>,
    /// the seconds each side of the seam loop analysis uses
    pub seam_seconds: f64,
    pub speed: f64,
    /// negative in reverse, 0 when not shuttling
    pub shuttle: i32,
    /// the zoom as the bar says it ("fit 43.2%", "200%"), as a number (display pixels for one video pixel), whether
    /// one is set rather than fit, and whether it is a whole multiple
    pub zoom_label: String,
    pub zoom: f32,
    pub zoomed: bool,
    pub exact: bool,
    /// the picture is larger than the window: a drag on it moves it, not the window
    pub overflows: bool,
    /// the file's pixel under the pointer, and the red, green and blue the screen is sent for it, on the scale of
    /// the file's bit depth
    pub pixel: Option<(usize, usize)>,
    pub value: Option<[u32; 3]>,
    /// where the picture is in the window, in physical pixels, and its size in video pixels as shown
    pub picture: (f32, f32, f32, f32),
    pub shown_size: (usize, usize),
    pub look: Look,
    pub guides: Guides,
    pub facts: Facts<'a>,
    /// how the clip is decoded, in the panel's words
    pub decoded: &'a str,
    /// how chroma smaller than the picture is brought up to it, in the panel's words, and whether where it sits
    /// is assumed
    pub chroma: Option<(String, bool)>,
    /// the clip has sound; how loud the player is set, 0 to 1; and whether it is muted
    pub sound: bool,
    pub volume: f32,
    pub muted: bool,
    /// where screenshots go, as the menu names it
    pub shots_to: &'a str,
    pub extras: Extras,
    /// the marks drawn on the clip, by frame
    pub marks: Option<&'a Marks>,
}

/// what the user asked for through the controls.
#[derive(Debug, PartialEq)]
pub enum Action {
    TogglePlay,
    /// the second click of a double click on the picture: play and the frame go back to what the first click, which
    /// played or paused as any click does, found
    PlayAsBefore,
    Step(i64),
    Seek(usize),
    /// loop analysis: to this place in its round
    SeekPlace(usize),
    Loop(Loop),
    /// loop analysis on, with this many seconds each side of the seam, or off
    Seam(Option<f64>),
    Speed(f64),
    /// fit (None), or this many display pixels for one video pixel, about the middle of the window
    Zoom(Option<f32>),
    /// the picture, dragged by this many physical pixels
    Pan((f32, f32)),
    Look(Look),
    Guides(Guides),
    /// tabs: a new one, this one up, this one closed
    NewTab,
    SelectTab(usize),
    CloseTab(usize),
    /// go to this clip of the played folder
    GoClip(usize),
    /// open this file in the tab that is up
    Open(PathBuf),
    /// show this file in the system's own file browser
    Reveal(PathBuf),
    /// play this folder: asked for in the browser
    PlayFolder(PathBuf),
    /// play these files one after another: picked together in the browser
    PlayFiles(Vec<PathBuf>),
    /// lay the browser over the clip, or take it away
    Browse(bool),
    Shot(Shot),
    Extras(Extras),
    /// a mark drawn on this frame of the clip; every mark taken off the frame on screen
    Mark(usize, Mark),
    ClearMarks,
    /// this text to the clipboard
    Copy(String),
    /// how loud, 0 to 1; and mute, or sound again
    Volume(f32),
    Mute,
    Fullscreen,
    /// the window: move it with the pointer, whose button is down; minimize; maximize or put back; close
    MoveWindow,
    Minimize,
    ToggleMaximize,
    Close,
    /// a key, as if it were pressed: a command chosen in the right-click menu
    Key(Key, Mods),
    /// the settings as a control of the settings panel left them: they hold at once
    Settings(Box<Settings>),
    /// this page in the system's browser, or this file in the program the system opens it with
    OpenUrl(String),
    /// look for a newer release now; put the one found in place; stop that while it downloads
    CheckUpdates,
    StartUpdate,
    StopUpdate,
}

/// what one pass over the controls produced.
pub struct Output {
    pub actions: Vec<Action>,
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures: egui::TexturesDelta,
    /// physical pixels for one unit
    pub scale: f32,
    /// when the controls want drawing again with nothing new from the window: an animation is running, or the
    /// controls are due to fade. None when they are at rest.
    pub again: Option<Duration>,
    /// text copied from a field, for the clipboard
    pub copied: Option<String>,
    /// the strip and the bar are up
    pub shown: bool,
    /// the frame under the pointer on the timebar: its picture is wanted
    pub hovered: Option<usize>,
    /// the pointer is where a drag would draw a mark: it is shown as a cross
    pub cross: bool,
    /// the pointer is on an edge of the browser that resizes a part: it is shown as a two-way arrow
    pub resize: bool,
}

/// the menus that open over the bar
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    /// a played folder's clips, under its tab
    List,
    Loop,
    Speed,
    Zoom,
    View,
    Shot,
}

pub struct Ui {
    ctx: egui::Context,
    started: Instant,
    events: Vec<egui::Event>,
    /// the picture area in physical pixels, and physical pixels for one unit
    size: (f32, f32),
    scale: f32,
    /// the pointer moved since the last pass
    moved: bool,
    inside: bool,
    cross: bool,
    /// the controls are wanted on screen, and until when with the pointer at rest
    shown: bool,
    awake_until: f64,
    /// the controls are pinned: up wherever the pointer goes or rests. kept between runs
    pinned: bool,
    /// the window filled the display at the last pass, and until when the pointer's moves do not bring the controls
    /// up: the moment after it began to
    was_full: bool,
    still_until: f64,
    /// the zoom chip shows until then; the view changed since the last pass, which brings it back
    chip_until: f64,
    view_changed: bool,
    /// the readout counts in timecode instead of frames
    timecode: bool,
    /// the menu that is open, where it was drawn last, and the button that opened it
    menu: Option<Menu>,
    menu_rect: Rect,
    menu_button: Rect,
    /// the metadata panel: pinned open; or up while the pointer is on its button or on it. when the pointer came
    /// onto the button, when it left button and panel, and where the two were drawn
    facts_pinned: bool,
    facts_up: bool,
    facts_since: Option<f64>,
    facts_left: Option<f64>,
    facts_rect: Rect,
    facts_button: Rect,
    /// where the mute button and its slider were drawn: the slider is out while the pointer is on either
    volume_rect: Rect,
    /// what a key just did, said at the lower left until then (the design's fig. 09); and what was up at the last
    /// pass, by which a change is known: the tab, its name, the frame and the loop mode
    flash: Option<(Vec<(String, Color32)>, f64)>,
    flash_seen: Option<(usize, String, usize, Loop)>,
    /// how far the strip's tabs are scrolled, when there are more than fit; the tab that was up, how many there
    /// were and the room they had when it was last drawn; and where the tabs are, while they overflow
    tab_scroll: f32,
    tab_seen: (usize, usize, i32),
    tabs_over: Option<Rect>,
    /// room kept at the strip's left for the system's own window buttons, where a system draws them itself (a mac)
    buttons_room: Option<f32>,
    /// the keys held with the last press of a pointer button
    held: egui::Modifiers,
    /// the frame under the pointer on the timebar; the pictures had for the timebar, each with the frame it is of and
    /// its size: the clip's timebar strip, in order, and the one read last; and the one shown
    time_hover: Option<usize>,
    time_strip: Vec<TimePicture>,
    time_read: Option<TimePicture>,
    time_picture: Option<(usize, egui::TextureHandle)>,
    /// the right-click menu is open, from where it was asked for, and was drawn there last
    commands: Option<Pos2>,
    commands_rect: Rect,
    /// what has been typed into the right-click menu's field, to find a command
    commands_find: String,
    /// whether each of the last two presses went to the right-click menu, a menu or a panel, the earlier first
    taken: [bool; 2],
    /// the settings or help panel, while one is up
    panel: Option<panel::Panel>,
    /// the strip's tridant mark: when the pointer came onto it, and when it left the mark and its card; since when the
    /// card is up, with the seed its star's holds are drawn from; and where the card was drawn
    mark_since: Option<f64>,
    mark_left: Option<f64>,
    card: Option<(f64, u64)>,
    card_rect: Rect,
    /// what the player knows of updates, and whether the update's modal is up
    updates: Updates,
    update_open: bool,
    /// the brand's drawings and the program's icon, as textures
    pictures: brand::Pictures,
    /// the system asks for less motion: nothing moves by itself
    still: bool,
    /// the button the pointer is on in this pass, where it is and what its tooltip says; and the one it came onto,
    /// and when
    hinted: Option<(egui::Id, Rect, (String, String))>,
    tip: Option<(egui::Id, f64)>,
    browser: browser::Browser,
    markup: markup::Markup,
}

impl Ui {
    /// `folder` is where the browser starts: the one it was last in.
    pub fn new(size: (u32, u32), scale: f32, timecode: bool, folder: Option<PathBuf>, pictures: Option<Pictures>) -> Ui {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts_with_system());
        let mut visuals = egui::Visuals::dark();
        // the one text field style: the design's
        visuals.selection.bg_fill = ACCENT.gamma_multiply(0.5);
        visuals.text_cursor.stroke = egui::Stroke::new(1.0, FG);
        ctx.set_visuals(visuals);
        Ui {
            ctx,
            started: Instant::now(),
            events: Vec::new(),
            size: (size.0 as f32, size.1 as f32),
            scale,
            moved: false,
            inside: false,
            cross: false,
            shown: false,
            awake_until: 0.0,
            pinned: false,
            was_full: false,
            still_until: 0.0,
            chip_until: 0.0,
            view_changed: false,
            timecode,
            menu: None,
            menu_rect: Rect::NOTHING,
            menu_button: Rect::NOTHING,
            facts_pinned: false,
            facts_up: false,
            facts_since: None,
            facts_left: None,
            facts_rect: Rect::NOTHING,
            facts_button: Rect::NOTHING,
            volume_rect: Rect::NOTHING,
            flash: None,
            flash_seen: None,
            tab_scroll: 0.0,
            tab_seen: (0, 0, 0),
            tabs_over: None,
            buttons_room: None,
            held: egui::Modifiers::NONE,
            time_hover: None,
            time_strip: Vec::new(),
            time_read: None,
            time_picture: None,
            commands: None,
            commands_rect: Rect::NOTHING,
            commands_find: String::new(),
            taken: [false; 2],
            panel: None,
            mark_since: None,
            mark_left: None,
            card: None,
            card_rect: Rect::NOTHING,
            updates: Updates::default(),
            update_open: false,
            pictures: brand::Pictures::default(),
            still: false,
            hinted: None,
            tip: None,
            browser: browser::Browser::new(folder, pictures),
            markup: markup::Markup::default(),
        }
    }

    /// a key, a typed character and the wheel for the browser, while it is up.
    pub fn browser_key(&mut self, key: Key, mods: Mods) {
        self.browser.key(key, mods);
    }

    pub fn browser_address(&mut self) {
        self.browser.edit_address();
    }

    pub fn browser_retrace(&mut self, by: isize) {
        self.browser.retrace(by);
    }

    pub fn browser_refresh(&mut self) {
        self.browser.refresh();
    }

    pub fn browser_char(&mut self, c: char) {
        self.browser.char(c);
    }

    pub fn browser_wheel(&mut self, notches: i32) {
        self.browser.wheel(notches);
    }

    /// the folder the browser is in, and the ones it last opened files from: kept between runs.
    pub fn browser_folder(&self) -> &Path {
        self.browser.folder()
    }

    pub fn browser_recent(&self) -> &[PathBuf] {
        self.browser.recent()
    }

    /// where the system draws the window's buttons itself, at the strip's left (a mac): the strip keeps `room` for
    /// them there, draws none of its own, and the tabs start after it.
    pub fn set_buttons_room(&mut self, room: Option<f32>) {
        self.buttons_room = room;
    }

    /// whether the controls are pinned up (the pin at the bar's far left), which is kept between runs.
    pub fn pinned(&self) -> bool {
        self.pinned
    }

    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    pub fn set_browser_recent(&mut self, recent: Vec<PathBuf>) {
        self.browser.set_recent(recent);
    }

    pub fn browser_widths(&self) -> crate::browse::Widths {
        self.browser.widths()
    }

    pub fn set_browser_widths(&mut self, widths: crate::browse::Widths) {
        self.browser.set_widths(widths);
    }

    pub fn browser_view(&self) -> crate::browse::View {
        self.browser.view()
    }

    pub fn set_browser_view(&mut self, view: crate::browse::View) {
        self.browser.set_view(view);
    }

    fn point(&self, x: f32, y: f32) -> Pos2 {
        Pos2::new(x / self.scale, y / self.scale)
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.size = (w as f32, h as f32);
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.events.push(egui::Event::PointerMoved(self.point(x, y)));
        (self.moved, self.inside) = (true, true);
    }

    /// the keys held with the pointer press that follows: a press with shift or control picks several in the browser.
    pub fn set_held(&mut self, mods: Mods) {
        self.held = egui::Modifiers { alt: mods.alt, ctrl: mods.ctrl, shift: mods.shift, mac_cmd: false, command: mods.ctrl };
        self.browser.set_held(mods.shift, mods.ctrl);
    }

    pub fn pointer_button(&mut self, button: crate::event::Button, down: bool, x: f32, y: f32) {
        let button = match button {
            crate::event::Button::Left => egui::PointerButton::Primary,
            crate::event::Button::Right => egui::PointerButton::Secondary,
            crate::event::Button::Middle => egui::PointerButton::Middle,
        };
        self.events.push(egui::Event::PointerButton { pos: self.point(x, y), button, pressed: down, modifiers: self.held });
    }

    pub fn pointer_leave(&mut self) {
        self.events.push(egui::Event::PointerGone);
        // the mark's card goes with the pointer, and waits for it to rest on the mark again
        (self.inside, self.card, self.mark_since) = (false, None, None);
    }

    /// true while a field in a menu is being typed into: keys then go there, not to the player.
    pub fn typing(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// what the clipboard holds is typed into the field that has the keyboard.
    pub fn paste(&mut self, text: String) {
        self.events.push(egui::Event::Paste(text));
    }

    /// a key for the field being typed into. control + C copies what is selected in it (`Output::copied`).
    pub fn key(&mut self, key: Key, mods: Mods) {
        if key == Key::C && mods.ctrl {
            return self.events.push(egui::Event::Copy);
        }
        let key = match key {
            Key::Backspace => egui::Key::Backspace,
            Key::Delete => egui::Key::Delete,
            Key::Enter => egui::Key::Enter,
            Key::Escape => egui::Key::Escape,
            Key::Tab => egui::Key::Tab,
            Key::Left => egui::Key::ArrowLeft,
            Key::Right => egui::Key::ArrowRight,
            Key::Home => egui::Key::Home,
            Key::End => egui::Key::End,
            Key::A if mods.ctrl => egui::Key::A,
            _ => return,
        };
        let modifiers = egui::Modifiers { alt: mods.alt, ctrl: mods.ctrl, shift: mods.shift, mac_cmd: false, command: mods.ctrl };
        for pressed in [true, false] {
            self.events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
        }
    }

    pub fn char(&mut self, c: char) {
        self.events.push(egui::Event::Text(c.to_string()));
    }

    /// the wheel, when the pointer is over the metadata panel or the right-click menu: it turns their list. false when
    /// the pointer is elsewhere and the wheel is the player's.
    pub fn wheel(&mut self, notches: i32) -> bool {
        // over a strip with more tabs than fit, the wheel moves the tabs: a tab's least width a notch
        if self.panel.is_none() && self.ctx.pointer_latest_pos().is_some_and(|p| self.tabs_over.is_some_and(|tabs| tabs.contains(p))) {
            self.tab_scroll -= notches as f32 * TAB_MIN;
            return true;
        }
        // a panel takes the wheel wherever the pointer is: its page scrolls
        let over = self.panel.is_some() || self.ctx.pointer_latest_pos().is_some_and(|p| (self.menu == Some(Menu::List) && self.menu_rect.contains(p)) || ((self.facts_pinned || self.facts_up) && self.facts_rect.contains(p)) || (self.commands.is_some() && self.commands_rect.contains(p)));
        if over {
            self.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: Vec2::new(0.0, notches as f32),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            });
        }
        over
    }

    /// escape: drops the markup text being typed, or else closes the open menu, or else ends markup mode. false when
    /// there was nothing of ours to close.
    pub fn escape(&mut self) -> bool {
        if self.markup.drop_text() || self.menu.take().is_some() | self.commands.take().is_some() {
            return true;
        }
        let was = self.markup.on;
        if was {
            self.markup.toggle();
        }
        was
    }

    /// markup mode on or off (E). it closes the menus, as the screenshot menu's "markup mode" asks.
    pub fn toggle_markup(&mut self) {
        self.markup.toggle();
        (self.menu, self.commands) = (None, None);
    }

    pub fn markup(&self) -> bool {
        self.markup.on
    }

    /// help comes up on its keys page (`?`), in place of any menu.
    pub fn show_help(&mut self) {
        self.open_panel(panel::Which::Help, 0);
    }

    /// settings comes up on its first page, or goes when it is up (control + ,).
    pub fn toggle_settings(&mut self) {
        self.toggle_panel(panel::Which::Settings);
    }

    /// true while the settings or help panel is up.
    pub fn panel_up(&self) -> bool {
        self.panel.is_some()
    }

    /// a key while a panel is up. esc closes either. help keeps the rule of the table of keys it took the place of
    /// (fig. 39): ? closes it too, and any other key closes it and then does what it does, so a key read there can be
    /// pressed at once. in settings (fig. 33) up and down go through the pages, control + , closes it, and the
    /// player's other keys wait. true when the key was the panel's, false when it goes on to the player.
    pub fn panel_key(&mut self, key: Key, mods: Mods) -> bool {
        let Some(panel) = &mut self.panel else { return false };
        let help = panel.which == panel::Which::Help;
        let closes = key == Key::Escape || (help && key == Key::Slash && !mods.ctrl && !mods.alt) || (!help && key == Key::Comma && mods.ctrl);
        if closes || help {
            self.panel = None;
            return closes;
        }
        if matches!(key, Key::Up | Key::Down) {
            panel.turn(if key == Key::Up { -1 } else { 1 });
        }
        true
    }

    /// the system asks for less motion (`window::reduce_motion`): the mark's star stands still.
    pub fn set_still(&mut self, still: bool) {
        self.still = still;
    }

    /// a panel comes up on one of its pages, over whatever is up, and the menus and the mark's card go.
    fn open_panel(&mut self, which: panel::Which, page: usize) {
        self.panel = Some(panel::Panel::new(which, page));
        (self.menu, self.commands, self.card) = (None, None, None);
    }

    /// the strip's button for a panel: it opens it, and closes it when it is the one up.
    fn toggle_panel(&mut self, which: panel::Which) {
        if self.panel.as_ref().is_some_and(|panel| panel.which == which) {
            self.panel = None;
        } else {
            self.open_panel(which, 0);
        }
    }

    /// the view changed by key or wheel: the zoom chip shows again.
    pub fn view_changed(&mut self) {
        self.view_changed = true;
    }

    /// says a few words at the lower left for a moment: what a key changed that the picture does not show (the
    /// volume, the mute).
    pub fn flash(&mut self, said: &str) {
        let now = self.ctx.input(|i| i.time);
        self.flash = Some((vec![(said.to_string(), FG)], now + FLASH));
    }

    /// a picture of a frame of the clip on screen, for the timebar: red, green and blue, a byte each. one of the clip's
    /// `strip` is kept while the clip is up; any other takes the place of the one read before.
    pub fn timebar_picture(&mut self, frame: usize, (width, height): (u32, u32), pixels: Vec<u8>, strip: bool) {
        let picture = (frame, [width as usize, height as usize], pixels);
        if strip {
            let at = self.time_strip.partition_point(|(of, ..)| *of < frame);
            self.time_strip.insert(at, picture);
        } else {
            self.time_read = Some(picture);
        }
    }

    /// the clip on screen is another one now: the timebar's pictures are not of it.
    pub fn forget_timebar_picture(&mut self) {
        (self.time_strip, self.time_read, self.time_picture) = (Vec::new(), None, None);
    }

    /// pins the metadata panel open, or lets it go.
    pub fn toggle_facts(&mut self) {
        self.facts_pinned = !self.facts_pinned;
    }

    /// the readout counts in timecode, or back in frames.
    pub fn toggle_timecode(&mut self) {
        self.timecode = !self.timecode;
    }

    pub fn timecode(&self) -> bool {
        self.timecode
    }

    /// one pass: takes what the window reported since the last one, lays out and draws the controls.
    pub fn run(&mut self, screen: &Screen) -> Output {
        let now = self.started.elapsed().as_secs_f64();
        let mut raw = egui::RawInput { time: Some(now), focused: true, events: std::mem::take(&mut self.events), ..Default::default() };
        raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(self.size.0 / self.scale, self.size.1 / self.scale)));
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(self.scale);
        let mut actions = Vec::new();
        let ctx = self.ctx.clone();
        let out = ctx.run_ui(raw, |ui| {
            // while a button is held the controls are looked at again every refresh: a drag is told from a click one
            // pass after the press, and that pass must come even if the pointer's moves all arrived with the press
            // (a busy machine hands them over in one lump)
            if ui.input(|i| i.pointer.any_down()) {
                ui.ctx().request_repaint();
            }
            self.draw(ui, screen, now, &mut actions)
        });
        let again = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay).filter(|d| *d < Duration::from_secs(3600));
        let copied = out.platform_output.commands.iter().find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        });
        self.moved = false;
        // under a panel the strip stays up, though the bar steps aside: the pointer shows, and the mac's buttons stay
        let shown = self.shown || self.panel.is_some();
        Output { actions, primitives: ctx.tessellate(out.shapes, out.pixels_per_point), textures: out.textures_delta, scale: out.pixels_per_point, again, copied, shown, hovered: self.time_hover, cross: self.cross , resize: screen.browsing && self.browser.over_edge()}
    }

    fn draw(&mut self, ui: &mut egui::Ui, screen: &Screen, now: f64, actions: &mut Vec<Action>) {
        self.cross = false;
        let full = ui.max_rect();
        let strip = Rect::from_min_size(full.min, Vec2::new(full.width(), STRIP));
        match &screen.clip {
            Some(model) if !screen.browsing => {
                self.browser.away();
                self.clip(ui, screen, model, now, actions)
            }
            // no clip to look at: the strip stays up, and under it the browser, or why the file did not open
            _ => {
                (self.menu, self.shown, self.commands) = (None, true, None);
                let body = Rect::from_min_max(strip.left_bottom(), full.max);
                match (screen.error.filter(|_| !screen.browsing), screen.opening.filter(|_| !screen.browsing)) {
                    (Some(why), _) => {
                        self.browser.away();
                        message(ui, body, screen.tabs[screen.current], why)
                    }
                    // a file on its way to a tab with nothing else in it: the tab waits, and says so once it is slow
                    (None, Some(words)) => {
                        self.browser.away();
                        message(ui, body, words, "")
                    }
                    (None, None) => match self.browser.draw(ui, body) {
                        Some(browser::Chosen::Open(path)) => actions.push(Action::Open(path)),
                        Some(browser::Chosen::Reveal(path)) => actions.push(Action::Reveal(path)),
                        Some(browser::Chosen::PlayFolder(folder)) => actions.push(Action::PlayFolder(folder)),
                        Some(browser::Chosen::PlayFiles(files)) => actions.push(Action::PlayFiles(files)),
                        Some(browser::Chosen::Back) => actions.push(Action::Browse(false)),
                        None => {}
                    },
                }
                self.strip(ui, strip, screen, true, actions);
                // tooltips are the clip's
                (self.hinted, self.tip) = (None, None);
            }
        }
        // the settings or help panel, over whatever is up
        self.panel_over(ui, full, screen, actions);
        self.update_modal(ui, full, actions);
        if let Some(said) = screen.toast {
            toast(ui, full, said);
        }
    }

    /// a clip is up: the picture takes the pointer, and the controls come and go over it.
    fn clip(&mut self, ui: &mut egui::Ui, screen: &Screen, model: &Model, now: f64, actions: &mut Vec<Action>) {
        let full = ui.max_rect();
        // the picture, under everything else. a plain click on it plays or pauses. a drag moves the
        // window while the whole frame is in view, and the picture when it is larger than the window. a double click
        // fills the display with it, or puts the window back (the owner's word, in place of the design's fit and
        // last zoom). in markup mode a drag draws instead, and a click and a double click do nothing of the kind
        let picture = ui.interact(full, ui.scope_id().with("picture"), Sense::click_and_drag());
        if self.markup.on {
            self.markup.pointer(ui, &picture, model, self.scale, actions);
        } else if model.overflows {
            if picture.dragged_by(egui::PointerButton::Primary) {
                let by = picture.drag_delta() * self.scale;
                if by != Vec2::ZERO {
                    actions.push(Action::Pan((by.x, by.y)));
                }
            }
        } else if picture.drag_started_by(egui::PointerButton::Primary) && !screen.fullscreen {
            actions.push(Action::MoveWindow);
        }
        let pointer = ui.ctx().pointer_latest_pos();
        let pressed = ui.input(|i| i.pointer.any_pressed());
        // egui counts a double click by time and place, not by what was hit: one whose first press chose a row of the
        // right-click menu, closed a menu or a panel, or went to a panel, is not one on the picture
        if pressed {
            self.taken = [self.taken[1], self.panel.is_some() || self.commands.is_some() || self.menu.is_some()];
        }
        if picture.double_clicked() && !self.taken[0] && !self.markup.on {
            actions.push(Action::Fullscreen);
        }
        let strip = Rect::from_min_size(full.min, Vec2::new(full.width(), STRIP));
        let bar = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - (1.0 + ROW + TIME)), full.max);
        let over = self.inside && pointer.is_some_and(|p| bar.contains(p) || strip.contains(p) || (self.menu.is_some() && self.menu_rect.contains(p)) || (self.facts_up && self.facts_rect.contains(p)) || (self.card.is_some() && self.card_rect.contains(p)) || (self.markup.on && self.markup.row.is_some_and(|row| row.contains(p))));
        let held = ui.input(|i| i.pointer.any_down());
        // in markup mode the pointer is a cross where a drag would draw: on the picture, clear of everything over it
        self.cross = self.markup.on && self.inside && !over && self.panel.is_none() && pointer.is_some_and(|p| !(self.commands.is_some() && self.commands_rect.contains(p)));

        // a press anywhere but on the open menu or its button closes the menu
        if self.menu.is_some() && pressed && !pointer.is_some_and(|p| self.menu_rect.contains(p) || self.menu_button.contains(p)) {
            self.menu = None;
        }
        // the right-click menu closes the same way, and a right click on the picture opens it there
        if self.commands.is_some() && pressed && !pointer.is_some_and(|p| self.commands_rect.contains(p)) {
            self.commands = None;
        }
        let opened = picture.secondary_clicked();
        if opened {
            self.commands = pointer;
        }

        // the design's rule: moving the pointer brings the controls up; they go when it rests for a while off them,
        // or leaves the window. an open menu keeps them up.
        // marking up, only the top 40 and the bottom 60 bring them up, so the picture is free to draw on: in between
        // they go 0.9 s after the pointer comes there (prototype `wake`)
        let middle = self.markup.on && pointer.is_some_and(|p| p.y > full.top() + 40.0 && p.y < full.bottom() - 60.0);
        // filling the display takes the controls away at once (the owner's word, 2026-11-24): the picture and nothing
        // else, until the pointer next moves. the window says the pointer moved as it changes size, so for half a
        // second its moves do not count
        if screen.fullscreen && !std::mem::replace(&mut self.was_full, screen.fullscreen) {
            (self.shown, self.still_until) = (false, now + 0.5);
        }
        self.was_full = screen.fullscreen;
        let still = now < self.still_until;
        if self.moved && middle {
            self.awake_until = self.awake_until.min(now + 0.9);
        } else if self.moved && !still {
            (self.shown, self.awake_until) = (true, now + screen.settings.controls_after as f64);
        }
        if self.moved || std::mem::take(&mut self.view_changed) {
            self.chip_until = now + CHIP_IDLE;
        }
        // a plain click on the picture plays or pauses: the owner's word (2026-11-24), in place of the design's show
        // and hide. not the click that closed a menu or took the table of keys away, nor the second of two whose
        // first did, nor one on the metadata panel. the second click of a double click takes the first one back, so
        // filling the display leaves play, and the frame of a clip that stood still, as they were
        let on_facts = (self.facts_pinned || self.facts_up) && pointer.is_some_and(|p| self.facts_rect.contains(p));
        if picture.clicked() && !self.markup.on && !self.taken[1] && !(picture.double_clicked() && self.taken[0]) && !on_facts {
            actions.push(if picture.double_clicked() { Action::PlayAsBefore } else { Action::TogglePlay });
        }
        if self.menu.is_some() || self.commands.is_some() || self.pinned {
            self.shown = true;
        } else {
            if !self.inside && !held {
                self.shown = false;
            }
            if self.shown && !over && !held {
                if now >= self.awake_until {
                    self.shown = false;
                } else {
                    ui.ctx().request_repaint_after(Duration::from_secs_f64(self.awake_until - now));
                }
            }
        }
        // a panel takes the window, as the design draws it: the bar steps aside while it is up, and the strip stays
        let panel = self.panel.is_some();
        self.shown &= !panel;
        let alpha = ui.ctx().animate_bool_with_time(ui.scope_id().with("chrome"), self.shown, FADE);

        Self::draw_guides(ui.painter(), model.guides, model.picture, model.shown_size, self.scale);
        self.markup.marks(ui, model, self.scale);
        // what states the view sits under the strip when it is up, as in the design: a fault across the top, then
        // loop analysis, speed and the look from the left, and the zoom at the right
        let mut top = full.top() + 10.0 + STRIP * alpha;
        if let Some(why) = model.fault {
            top = banner(ui, Pos2::new(full.left() + 10.0, top), full.width() - 20.0, &format!("this frame cannot be shown: {why}")).bottom() + 8.0;
        }
        let mut left = full.left() + 10.0;
        let linear = model.linear.then(|| vec![("linear".to_string(), FG), (" · shown through srgb".to_string(), DIM)]);
        for parts in [seam_chip(model), speed_chip(model), model.look.label().map(|look| vec![(look, FG)]), linear].into_iter().flatten() {
            left = chip(ui, Pos2::new(left, top), Align2::LEFT_TOP, &parts, 1.0, MONO_MEDIUM).right() + 8.0;
        }
        self.zoom_chip(ui, Pos2::new(full.right() - 10.0, top), model, now);
        // with the controls away, what a key did is said at the lower left for a moment (the design's fig. 09): the
        // clip another tab or a page key brought up, the loop mode, the frame a step landed on
        let seen = (screen.current, screen.tabs[screen.current].to_string(), model.frame, model.looping);
        if let Some(was) = self.flash_seen.replace(seen.clone()).filter(|was| *was != seen && !self.shown) {
            let rate = model.rate.map_or_else(String::new, |(n, d)| if d == 1 { n.to_string() } else { format!("{n}/{d}") });
            let said = if (was.0, &was.1) != (seen.0, &seen.1) {
                let size = format!(" · {} × {} · ", model.shown_size.0, model.shown_size.1);
                Some((vec![(seen.1.clone(), FG), (size, DIM), (rate, FG), (" fps".to_string(), DIM)], 2.0))
            } else if was.3 != seen.3 {
                let mode = match seen.3 {
                    Loop::Off => "loop off",
                    Loop::One => "loop one",
                    Loop::All => "loop all",
                };
                Some((vec![(mode.to_string(), FG)], FLASH))
            } else if !model.playing {
                let (label, at) = stamp(model.frame, model.first, model.clock, self.timecode);
                Some((vec![(format!("{label} "), DIM), (at, FG)], FLASH))
            } else {
                None
            };
            if let Some((parts, seconds)) = said {
                self.flash = Some((parts, now + seconds));
            }
        }
        // the lower left, 12 up from the bottom and clear of the bar when it is up: the navigator when the picture
        // is zoomed past the window, and over it what a key did
        let low = full.bottom() - 12.0 - 50.0 * alpha;
        let low = self.navigator(ui, full, low, model, now, actions).map_or(low, |navigator| navigator.top() - 8.0);
        match &self.flash {
            Some((parts, until)) if now < *until => {
                chip(ui, Pos2::new(full.left() + 10.0, low), Align2::LEFT_BOTTOM, parts, 1.0, MONO_MEDIUM);
                ui.ctx().request_repaint_after(Duration::from_secs_f64(until - now));
            }
            _ => self.flash = None,
        }
        // markup mode says it is on with its row of tools, at the top middle as the chips stand, whether the controls
        // are up or not
        if self.markup.on {
            self.markup.tools(ui, full.center().x, full.top() + 10.0 + STRIP * alpha, actions);
            self.markup.field(ui, full, actions);
        }
        // the metadata panel, pinned: at the right under the chips, whether the controls are up or not
        if self.facts_pinned {
            let corner = Pos2::new(full.right() - 10.0, top + 34.0);
            self.facts_rect = menus::facts(ui, corner, Align2::RIGHT_TOP, 1.0, full.bottom() - corner.y - 10.0 - (bar.height() + 7.0) * alpha, model, actions);
        }
        // the right-click menu, whether the controls are up or not, 8 in from the window's edges and, while a chip is
        // up, under the chips as the pinned metadata panel stands: a row chosen is its key, pressed
        if let Some(at) = self.commands {
            let chipped = left > full.left() + 10.0 || model.zoomed || model.fault.is_some();
            let room = Rect::from_min_max(Pos2::new(full.left() + 8.0, if chipped { top + 34.0 } else { full.top() + 8.0 }), full.max - Vec2::splat(8.0));
            if opened {
                self.commands_find.clear();
            }
            let (drawn, chosen) = keys::menu(ui, at, room, self.commands_rect, opened, &mut self.commands_find);
            self.commands_rect = drawn;
            if let Some((key, mods)) = chosen {
                actions.push(Action::Key(key, mods));
                self.commands = None;
            }
        }

        if panel {
            ui.set_opacity(1.0);
            self.strip(ui, strip, screen, false, actions);
            // nothing under a panel has a tooltip
            (self.hinted, self.tip) = (None, None);
            return;
        }
        if alpha <= 0.0 {
            return;
        }
        ui.set_opacity(alpha);
        self.strip(ui, strip, screen, false, actions);
        self.bar(ui, bar, model, now, actions);
        if self.facts_up && !self.facts_pinned {
            // as a menu: standing over its button
            let corner = Pos2::new((self.facts_button.right() + 4.0).min(bar.right() - 8.0), bar.top() - 7.0);
            self.facts_rect = menus::facts(ui, corner, Align2::RIGHT_BOTTOM, alpha, corner.y - full.top() - 10.0, model, actions);
        }
        if let Some(menu) = self.menu {
            // the design stands a menu 7 above the bar, its right edge 4 past its button's, and 8 in from the window's
            let corner = Pos2::new((self.menu_button.right() + 4.0).min(bar.right() - 8.0), bar.top() - 7.0);
            self.menu_rect = match (menu, &screen.list) {
                // the folder's list stands under the strip, at the left (the design's fig. 14)
                (Menu::List, Some(list)) => menus::list(ui, Pos2::new(full.left() + 8.0, strip.bottom() + 8.0), alpha, full.height() - 104.0, list, actions),
                (Menu::List, None) => {
                    self.menu = None;
                    Rect::NOTHING
                }
                _ => menus::show(ui, menu, corner, alpha, model, actions),
            };
            // what a menu changes about the view is said by the chip
            self.view_changed |= actions.iter().any(|a| matches!(a, Action::Zoom(_)));
        }
        self.tooltip(ui, full, now);
    }

    /// the lines over the picture: the grid of tiles, the center lines and circle, one display pixel wide at 100%. the
    /// picture is at `(x, y, w, h)` in the target's pixels, `shown` big in its own as shown, and `scale` pixels of the
    /// target make one of the design's: the window's, or a saved picture's (`markup::saved`).
    fn draw_guides(painter: &egui::Painter, guides: Guides, (x, y, w, h): (f32, f32, f32, f32), shown: (usize, usize), scale: f32) {
        if !guides.grid && !guides.center {
            return;
        }
        let (video_w, video_h) = (shown.0 as f32, shown.1 as f32);
        let unit = 1.0 / scale;
        let width = scale.round().max(1.0);
        let color = Color32::from_rgb(guides.color[0], guides.color[1], guides.color[2]);
        let painter = painter.with_clip_rect(Rect::from_min_size(Pos2::new(x * unit, y * unit), Vec2::new(w * unit, h * unit)));
        // a filled rectangle given in physical pixels
        let fill = |x: f32, y: f32, w: f32, h: f32, color: Color32| painter.rect_filled(Rect::from_min_size(Pos2::new(x * unit, y * unit), Vec2::new(w * unit, h * unit)), 0.0, color);
        let (across, down) = (|v: f32| (x + v * w / video_w).round(), |v: f32| (y + v * h / video_h).round());
        if guides.grid {
            let faint = color.gamma_multiply(0.75);
            // every tile's edge, and the picture's own far edge kept inside it
            let (mut at, tile) = (0.0, (guides.tile.0.max(1) as f32, guides.tile.1.max(1) as f32));
            while at <= video_w {
                fill(across(at).min(across(video_w) - width), down(0.0), width, down(video_h) - down(0.0), faint);
                at += tile.0;
            }
            at = 0.0;
            while at <= video_h {
                fill(across(0.0), down(at).min(down(video_h) - width), across(video_w) - across(0.0), width, faint);
                at += tile.1;
            }
        }
        if guides.center {
            let half = (width / 2.0).floor();
            fill(across(video_w / 2.0) - half, down(0.0), width, down(video_h) - down(0.0), color);
            fill(across(0.0), down(video_h / 2.0) - half, across(video_w) - across(0.0), width, color);
            let middle = Pos2::new((x + w / 2.0) * unit, (y + h / 2.0) * unit);
            let radius = video_w.min(video_h) / 2.0 * w / video_w - width / 2.0;
            painter.circle_stroke(middle, radius * unit, (width * unit, color));
        }
    }

    /// the navigator (the owner's ask, 2026-11-24): while the picture is zoomed past the window, a small plan of the
    /// whole of it at the lower left, as a chip is drawn, with the part in view marked in the bar's own white. a
    /// press or a drag in it brings what is under the pointer to the middle of the window. it is plain while the
    /// pointer moves or the view changes, and faint when they rest. `foot` is where its lower edge stands.
    fn navigator(&mut self, ui: &mut egui::Ui, full: Rect, foot: f32, model: &Model, now: f64, actions: &mut Vec<Action>) -> Option<Rect> {
        let (x, y, w, h) = model.picture;
        if !model.overflows || w < 1.0 || h < 1.0 {
            return None;
        }
        let size = Vec2::new(w, h) * (NAVIGATOR / w.max(h));
        let plan = Rect::from_min_size(Pos2::new(full.left() + 10.0, foot - size.y), size);
        let response = ui.interact(plan, ui.scope_id().with("navigator"), Sense::click_and_drag());
        // the window on the picture, as parts of it: no further than its edges, since a picture narrower than the
        // window stands inside it
        let window = full.size() * self.scale;
        let part = |from: f32, length: f32, room: f32| ((-from / length).clamp(0.0, 1.0), ((room - from) / length).clamp(0.0, 1.0));
        let (across, down) = (part(x, w, window.x), part(y, h, window.y));
        let view = Rect::from_min_max(plan.min + Vec2::new(across.0 * size.x, down.0 * size.y), plan.min + Vec2::new(across.1 * size.x, down.1 * size.y));
        if response.is_pointer_button_down_on() {
            if let Some(at) = response.interact_pointer_pos() {
                let to = ((at - plan.min) / size).clamp(Vec2::ZERO, Vec2::splat(1.0));
                let middle = Vec2::new((window.x / 2.0 - x) / w, (window.y / 2.0 - y) / h);
                let by = ((middle.x - to.x) * w, (middle.y - to.y) * h);
                if by.0.abs() >= 0.5 || by.1.abs() >= 0.5 {
                    actions.push(Action::Pan(by));
                }
            }
            self.chip_until = now + CHIP_IDLE;
        }
        let awake = now < self.chip_until || response.hovered();
        let alpha = 0.45 + 0.55 * ui.ctx().animate_bool_with_time(ui.scope_id().with("navigator-awake"), awake, FADE);
        let painter = ui.painter();
        painter.rect_filled(plan, 0.0, CHIP.gamma_multiply(alpha));
        painter.rect_stroke(plan, 0.0, (1.0, LINE.gamma_multiply(alpha)), egui::StrokeKind::Inside);
        painter.rect_filled(view, 0.0, HOVER.gamma_multiply(alpha));
        painter.rect_stroke(view, 0.0, (1.0, FG.gamma_multiply(alpha)), egui::StrokeKind::Inside);
        Some(plan)
    }

    /// the zoom chip, top right: the zoom, whether it is exact, and the pixel under the pointer with its values.
    /// it shows while a zoom is set, and fades when the pointer rests.
    fn zoom_chip(&mut self, ui: &mut egui::Ui, right_top: Pos2, model: &Model, now: f64) {
        let awake = model.zoomed && now < self.chip_until;
        if awake {
            ui.ctx().request_repaint_after(Duration::from_secs_f64((self.chip_until - now).max(0.0)));
        }
        let alpha = ui.ctx().animate_bool_with_time(ui.scope_id().with("zoom-chip"), awake, FADE);
        if alpha <= 0.0 || !model.zoomed {
            return;
        }
        let mut parts = vec![(model.zoom_label.clone(), FG), (if model.exact { " exact" } else { " scaled" }.to_string(), DIM)];
        if let Some((x, y)) = model.pixel {
            parts.push((" · ".to_string(), DIM));
            parts.push((format!("x {x} y {y}"), FG));
            if let Some([r, g, b]) = model.value {
                parts.push((" · ".to_string(), DIM));
                parts.push((format!("{r} {g} {b}"), FG));
            }
        }
        chip(ui, right_top, Align2::RIGHT_TOP, &parts, alpha, MONO_MEDIUM);
    }

    /// the strip along the top: the tabs, and the window's own buttons. it is the title bar: a drag on it moves the
    /// window and a double click maximizes it. over a clip it is see-through; over the browser, `solid`.
    fn strip(&mut self, ui: &mut egui::Ui, strip: Rect, screen: &Screen, solid: bool, actions: &mut Vec<Action>) {
        let painter = ui.painter().clone();
        painter.rect_filled(strip, 0.0, if solid { STRIP_SOLID } else { BAR });
        painter.rect_filled(Rect::from_min_max(Pos2::new(strip.left(), strip.bottom() - 1.0), strip.max), 0.0, LINE);
        let grab = ui.interact(strip, ui.scope_id().with("strip"), Sense::click_and_drag());
        if !screen.fullscreen {
            if grab.double_clicked() {
                actions.push(Action::ToggleMaximize);
            } else if grab.drag_started_by(egui::PointerButton::Primary) {
                actions.push(Action::MoveWindow);
            }
        }

        // the window's buttons, at the right: minimize, maximize, close. 10 x 10 marks in 40-wide buttons. where the
        // system draws its own at the left (`buttons_room`) there are none of ours. before them help, settings and
        // the tridant mark, 32 wide each (fig. 32), the last thing at the right on a mac. the tabs have the rest:
        // `from` to `x`
        let (from, x, ours) = match self.buttons_room {
            Some(room) => (strip.left() + room, strip.right() - 3.0 * PLUS, 0),
            None => (strip.left(), strip.right() - 3.0 * WINDOW_BUTTON - 3.0 * PLUS, 3),
        };
        let open = self.panel.as_ref().map(|panel| panel.which);
        for (i, (name, icon, which, tip)) in [("help", Icon::Help, panel::Which::Help, keys::named(Key::Slash, keys::SHIFT)), ("settings", Icon::Gear, panel::Which::Settings, keys::named(Key::Comma, keys::CTRL))].into_iter().enumerate() {
            let rect = Rect::from_min_size(Pos2::new(x + i as f32 * PLUS, strip.top()), Vec2::new(PLUS, STRIP - 1.0));
            let response = ui.interact(rect, ui.scope_id().with(name), Sense::click());
            // lit while pressed, and while its panel is up: the button closes what it opened
            if response.is_pointer_button_down_on() || open == Some(which) {
                painter.rect_filled(rect, 0.0, HOVER);
            }
            icons::draw(&painter, rect, icon, FG);
            if response.clicked() {
                self.toggle_panel(which);
            }
            self.hint(&response, tip);
        }
        self.mark(ui, &painter, Rect::from_min_size(Pos2::new(x + 2.0 * PLUS, strip.top()), Vec2::new(PLUS, STRIP - 1.0)), actions);
        // a newer release's quiet button, left of help, from the tabs' room
        let x = x - self.update_button(ui, strip, x);
        for (i, (name, action)) in [("close", Action::Close), ("maximize", Action::ToggleMaximize), ("minimize", Action::Minimize)].into_iter().enumerate().take(ours) {
            let rect = Rect::from_min_size(Pos2::new(strip.right() - (i + 1) as f32 * WINDOW_BUTTON, strip.top()), Vec2::new(WINDOW_BUTTON, STRIP - 1.0));
            let response = ui.interact(rect, ui.scope_id().with(name), Sense::click());
            if response.is_pointer_button_down_on() {
                painter.rect_filled(rect, 0.0, PRESSED);
            } else if response.hovered() {
                painter.rect_filled(rect, 0.0, HOVER);
            }
            let mark = Rect::from_min_size(rect.min + Vec2::new(15.0, 11.0), Vec2::splat(10.0));
            let line = egui::Stroke::new(1.0, DIM);
            match name {
                "minimize" => {
                    painter.hline(mark.x_range(), mark.top() + 5.5, line);
                }
                "maximize" => {
                    painter.rect_stroke(mark, 0.0, line, egui::StrokeKind::Inside);
                }
                _ => {
                    painter.line_segment([mark.left_top(), mark.right_bottom()], line);
                    painter.line_segment([mark.right_top(), mark.left_bottom()], line);
                }
            }
            if response.clicked() {
                // in fullscreen the middle button leaves it, which is what putting the window back means there
                actions.push(if screen.fullscreen && action == Action::ToggleMaximize { Action::Fullscreen } else { action });
            }
        }

        // the tabs, from the left: the one that is up is the bright, filled one, and carries the close mark. each is
        // as wide as its name up to a limit; too many for the strip, and they all narrow
        let room = (x - from - PLUS).max(0.0);
        let place = |i: usize| screen.places.get(i).copied().flatten();
        // the one tab there is, with nothing in it but the browser: closing it would leave the same, so it has no close
        // mark, and no room kept for one (the owner's word, 2026-11-24)
        let lone = screen.tabs.len() == 1 && screen.clip.is_none() && screen.error.is_none() && screen.opening.is_none();
        let pad = if lone { 24.0 } else { TAB_PAD };
        let wide = |i: usize, name: &str| {
            let label = match place(i) {
                Some(place) => {
                    let [title, arrow, count, space] = folder_pieces(&painter, place, MONO_MEDIUM);
                    title + 2.0 * arrow + count + space + painter.layout_no_wrap(place.clip.to_string(), font(MONO_MEDIUM, 11.5), FG).size().x
                }
                None => painter.layout_no_wrap(name.to_string(), font(MONO_MEDIUM, 11.5), FG).size().x,
            };
            (label + pad).min(TAB_MAX)
        };
        let mut widths: Vec<f32> = screen.tabs.iter().enumerate().map(|(i, name)| wide(i, name)).collect();
        if widths.iter().sum::<f32>() > room {
            let each = (room / widths.len() as f32).max(TAB_MIN);
            widths.iter_mut().for_each(|w| *w = w.min(each));
        }
        // narrowed as far as they go and still too many, the tabs scroll under the + (the design's rule)
        let total: f32 = widths.iter().sum();
        // (a window made narrower can push the tab that is up out of view as well)
        let seen = (screen.current, widths.len(), room as i32);
        let came_up = std::mem::replace(&mut self.tab_seen, seen) != seen;
        self.tab_scroll = tab_scroll(self.tab_scroll, &widths, screen.current, room, came_up);
        let view = Rect::from_min_max(Pos2::new(from, strip.top()), Pos2::new(if total > room { from + room } else { x }, strip.bottom()));
        self.tabs_over = (total > room).then_some(view);
        let clipped = painter.with_clip_rect(view);
        let mut left = from - self.tab_scroll;
        for (i, (name, width)) in screen.tabs.iter().zip(widths).enumerate() {
            let tab = Rect::from_min_size(Pos2::new(left, strip.top()), Vec2::new(width, STRIP - 1.0));
            left = tab.right();
            // a tab scrolled out of view is neither drawn nor clicked
            if !tab.intersects(view) {
                continue;
            }
            let up = i == screen.current;
            let clicked = ui.interact(tab.intersect(view), ui.scope_id().with(("tab", i)), Sense::click()).clicked();
            if clicked && !up {
                actions.push(Action::SelectTab(i));
            } else if clicked && screen.list.is_some() && screen.clip.is_some() && !screen.browsing {
                // the tab that is up, when it plays a folder: a click shows the folder's clips, and another puts
                // the list away
                self.toggle_menu(Menu::List, tab);
            }
            if up {
                clipped.rect_filled(tab, 0.0, TAB_ON);
            }
            if up && !lone {
                let close = Rect::from_center_size(Pos2::new(tab.right() - 8.0 - 6.0, tab.center().y), Vec2::splat(16.0));
                let mark = ui.interact(close.intersect(view), ui.scope_id().with(("tab-close", i)), Sense::click());
                if mark.clicked() {
                    actions.push(Action::CloseTab(i));
                }
                self.hint(&mark, keys::named(Key::W, keys::CTRL));
                icons::draw_sized(&clipped, close.center(), 12.0, Icon::Close, Color32::WHITE.gamma_multiply(0.7));
            }
            clipped.rect_filled(Rect::from_min_size(Pos2::new(tab.right() - 1.0, tab.top()), Vec2::new(1.0, tab.height())), 0.0, LINE);
            let (family, color) = if up { (MONO_MEDIUM, Color32::WHITE) } else { (MONO, TAB_OFF) };
            if let Some(place) = place(i) {
                let text = Rect::from_min_max(Pos2::new(tab.left() + 12.0, tab.top()), Pos2::new(tab.right() - (TAB_PAD - 12.0), tab.bottom())).intersect(view);
                self.folder_tab(ui, text, (i, up), place, actions);
                continue;
            }
            let galley = cut_middle(&clipped, name, font(family, 11.5), color, width - pad);
            clipped.galley(Pos2::new(tab.left() + 12.0, text_top(&clipped, font(family, 11.5), tab.center().y)), galley, color);
        }
        // the + that opens a new tab
        let plus = Rect::from_min_size(Pos2::new(left.min(x - PLUS), strip.top()), Vec2::new(PLUS, STRIP - 1.0));
        let response = ui.interact(plus, ui.scope_id().with("tab-new"), Sense::click());
        painter.text(plus.center(), Align2::CENTER_CENTER, "+", font(MONO, 16.0), if response.hovered() { FG } else { DIM });
        if response.clicked() {
            actions.push(Action::NewTab);
        }
        self.hint(&response, keys::named(Key::N, keys::NONE));
    }

    /// the tridant mark at the strip's right end (fig. 32): the brand's star, 14 across, in the strip's dim ink and
    /// brighter under the pointer, never in the brand's colors. resting on it brings up its card, which stays while
    /// the pointer is on the mark or the card, so the card's link can be pressed; a press on the mark opens about.
    fn mark(&mut self, ui: &egui::Ui, painter: &egui::Painter, mark: Rect, actions: &mut Vec<Action>) {
        let now = ui.input(|i| i.time);
        let response = ui.interact(mark, ui.scope_id().with("mark"), Sense::click());
        if response.clicked() {
            self.open_panel(panel::Which::Settings, panel::ABOUT);
        }
        let (star, size) = self.on_pixels(mark.center(), Vec2::splat(14.0));
        brand::draw(painter, self.pictures.get(ui.ctx(), brand::Art::Star, size), star, 0.0, if response.hovered() { FG } else { DIM });
        let on = self.panel.is_none() && (response.hovered() || (self.card.is_some() && ui.ctx().pointer_latest_pos().is_some_and(|p| self.card_rect.contains(p))));
        let (since, wait) = if on {
            self.mark_left = None;
            (*self.mark_since.get_or_insert(now), TIP_IN)
        } else {
            self.mark_since = None;
            (*self.mark_left.get_or_insert(now), FACTS_OUT)
        };
        if on != self.card.is_some() {
            if now - since >= wait {
                // the star's holds are drawn from the time it comes up: a simple pseudo-random
                let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
                self.card = on.then_some((now, seed));
            } else {
                ui.ctx().request_repaint_after(Duration::from_secs_f64(wait - (now - since)));
            }
        }
        if let Some((up, seed)) = self.card {
            self.card_rect = self.card(ui, mark, now - up, seed, actions);
        }
    }

    /// the mark's card, under the strip, its right edge on the mark's, 6 below the strip: the star at 40, moving as
    /// `brand::motion` says, then "eyepiece 0.1.0", "a tridant system" and tridant.io as a link. the star draws only
    /// while the card is up, and stands still when the system asks for less motion. returns where it was drawn.
    fn card(&mut self, ui: &egui::Ui, mark: Rect, t: f64, seed: u64, actions: &mut Vec<Action>) -> Rect {
        let painter = ui.painter();
        let version = painter.layout_no_wrap(format!("eyepiece {}", panel::VERSION), font(MONO_MEDIUM, 11.5), FG);
        let mut line = LayoutJob::default();
        for (words, color) in [("a ", DIM), ("tridant", FG), (" system", DIM)] {
            line.append(words, 0.0, TextFormat { font_id: font(SANS, 12.0), color, ..Default::default() });
        }
        let system = painter.layout_job(line);
        let link = painter.layout_no_wrap("tridant.io".to_string(), font(SANS, 12.0), FG);
        // 10 round the star and 14 at the right; three lines 8 apart, each as tall as its type
        let words = version.size().x.max(system.size().x).max(link.size().x);
        let size = Vec2::new(10.0 + 40.0 + 12.0 + words + 14.0, 10.0 + 11.5 + 8.0 + 12.0 + 8.0 + 12.0 + 10.0);
        let card = Rect::from_min_size(Pos2::new(mark.right() - size.x, mark.top() + STRIP + 6.0), size);
        // a press on the card is not a press on what is under it
        ui.interact(card, ui.scope_id().with("card"), Sense::click());
        painter.rect_filled(card, 0.0, CHIP);
        painter.rect_stroke(card, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
        let (turn, scale) = if self.still {
            (0.0, 1.0)
        } else {
            let (turn, scale, rest) = brand::motion(t, seed);
            match rest {
                Some(rest) => ui.ctx().request_repaint_after(Duration::from_secs_f64(rest)),
                None => ui.ctx().request_repaint(),
            }
            (turn, scale)
        };
        let middle = Pos2::new(card.left() + 10.0 + 20.0, card.center().y);
        let (star, pixels) = self.on_pixels(middle, Vec2::splat(brand::TURNING_SIZE));
        brand::draw(painter, self.pictures.get(ui.ctx(), brand::Art::Turning, pixels), Rect::from_center_size(star.center(), star.size() * scale), turn, FG);
        let left = card.left() + 10.0 + 40.0 + 12.0;
        let top = card.top() + 10.0;
        painter.galley(Pos2::new(left, text_top(painter, font(MONO_MEDIUM, 11.5), top + 5.75)), version, FG);
        painter.galley(Pos2::new(left, text_top(painter, font(SANS, 12.0), top + 19.5 + 6.0)), system, FG);
        // tridant.io, underlined dotted as a folder tab's count is: a press opens it in the system's browser
        let at = Pos2::new(left, text_top(painter, font(SANS, 12.0), top + 39.5 + 6.0));
        let width = link.size().x;
        painter.galley(at, link, FG);
        if ui.interact(Rect::from_min_size(Pos2::new(left, top + 36.5), Vec2::new(width, 18.0)), ui.scope_id().with("card-link"), Sense::click()).clicked() {
            actions.push(Action::OpenUrl(panel::TRIDANT.to_string()));
        }
        dotted(painter, font(SANS, 12.0), at, width, FG);
        card
    }

    /// a box `size` across round `middle` on whole physical pixels, and how many it takes: a texture drawn there
    /// is sharp.
    fn on_pixels(&self, middle: Pos2, size: Vec2) -> (Rect, [usize; 2]) {
        let pixels = (size * self.scale).round().max(Vec2::splat(1.0));
        let min = ((middle.to_vec2() * self.scale - pixels / 2.0).round() / self.scale).to_pos2();
        (Rect::from_min_size(min, pixels / self.scale), [pixels.x as usize, pixels.y as usize])
    }

    /// the label of tab `i`, which plays a folder, inside `text`, as the design's fig. 14 has it: the folder's name and
    /// a slash, dim; the clip's place, underlined dotted, between arrows to the clip before and the clip after; the
    /// clip's name, cut in its middle to the room left. an arrow does what its page key does, on its own tab.
    fn folder_tab(&mut self, ui: &mut egui::Ui, text: Rect, (i, up): (usize, bool), place: Place, actions: &mut Vec<Action>) {
        let painter = ui.painter().with_clip_rect(text);
        let (family, color) = if up { (MONO_MEDIUM, Color32::WHITE) } else { (MONO, TAB_OFF) };
        let top = text_top(&painter, font(family, 11.5), text.center().y);
        let [title, arrow, count, space] = folder_pieces(&painter, place, family);
        painter.galley(Pos2::new(text.left(), top), painter.layout_no_wrap(format!("{} /", place.folder), font(family, 11.5), DIM), DIM);
        let at = text.left() + title + arrow;
        for (left, glyph, key) in [(at - arrow, "‹", Key::Left), (at + count, "›", Key::Right)] {
            let hit = Rect::from_min_max(Pos2::new(left, text.top()), Pos2::new(left + arrow, text.bottom())).intersect(text);
            let mut lit = false;
            if hit.is_positive() {
                let response = ui.interact(hit, ui.scope_id().with(("tab-step", i, glyph)), Sense::click());
                if response.clicked() {
                    if !up {
                        actions.push(Action::SelectTab(i));
                    }
                    actions.push(Action::Key(key, keys::CTRL));
                }
                self.hint(&response, keys::named(key, keys::CTRL));
                lit = response.hovered();
            }
            let color = if lit { FG } else { DIM };
            painter.galley(Pos2::new(left + NAV_PAD, top), painter.layout_no_wrap(glyph.to_string(), font(family, 11.5), color), color);
        }
        let figures = painter.layout_no_wrap(format!("{} of {}", place.at + 1, place.count), font(MONO_MEDIUM, 11.5), FG);
        dotted(&painter, font(MONO_MEDIUM, 11.5), Pos2::new(at, top), count, FG);
        painter.galley(Pos2::new(at, top), figures, FG);
        let name = at + count + arrow + space;
        if name < text.right() {
            painter.galley(Pos2::new(name, top), cut_middle(&painter, place.clip, font(family, 11.5), color, text.right() - name), color);
        }
    }

    /// the bar along the bottom: one row of controls over the full-width timebar.
    fn bar(&mut self, ui: &mut egui::Ui, bar: Rect, model: &Model, now: f64, actions: &mut Vec<Action>) {
        let painter = ui.painter().clone();
        painter.rect_filled(bar, 0.0, BAR);
        painter.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width(), 1.0)), 0.0, LINE);
        // a press on the bar is not a press on the picture under it
        ui.interact(bar, ui.scope_id().with("bar"), Sense::click_and_drag());
        let middle = row_middle(bar);
        let narrow = bar.width() < NARROW;

        // from the left: previous frame, play, next frame, the readout
        let mut x = bar.left() + PAD;
        // the next item in the row, `lead` further along than the usual gap
        let mut place = |lead: f32, w: f32, h: f32| {
            let r = Rect::from_min_size(Pos2::new(x + lead, middle - h / 2.0), Vec2::new(w, h));
            x += lead + w + GAP;
            r
        };
        // the pin, first and a little apart: while it is on the controls stay up, wherever the pointer goes or rests
        // (the owner's word, 2026-11-24). it has no key
        let pin = icon_button(ui, place(0.0, ICON_BUTTON.x, ICON_BUTTON.y), "pin", Icon::Pin, if self.pinned { Lit::Active } else { Lit::No });
        if pin.clicked() {
            self.pinned = !self.pinned;
        }
        self.hint(&pin, Some((if self.pinned { "let the controls go" } else { "keep the controls up" }.to_string(), String::new())));
        let prev = icon_button(ui, place(8.0, ICON_BUTTON.x, ICON_BUTTON.y), "prev", Icon::Prev, Lit::No);
        if prev.clicked() {
            actions.push(Action::Step(-1));
        }
        self.hint(&prev, keys::named(Key::F, keys::NONE));
        let play = icon_button(ui, place(0.0, ICON_BUTTON.x, ICON_BUTTON.y), "play", if model.playing { Icon::Pause } else { Icon::Play }, Lit::No);
        if play.clicked() {
            actions.push(Action::TogglePlay);
        }
        self.hint(&play, keys::named(Key::Space, keys::NONE));
        let next = icon_button(ui, place(0.0, ICON_BUTTON.x, ICON_BUTTON.y), "next", Icon::Next, Lit::No);
        if next.clicked() {
            actions.push(Action::Step(1));
        }
        self.hint(&next, keys::named(Key::G, keys::NONE));
        // the readout: where the clip is, in frames or timecode, and the total. a click switches between the two
        let galley = painter.layout_job(readout(model, self.timecode, narrow));
        let rect = place(8.0, galley.size().x + 2.0 * PAD, TEXT_BUTTON);
        let switch = text_button(ui, rect, "readout", None, galley, false);
        if switch.clicked() {
            self.toggle_timecode();
        }
        self.hint(&switch, keys::named(Key::T, keys::NONE));

        // from the right, in the design's order: fullscreen, sound, screenshots, metadata, the view, and the three
        // that say what they are set to: zoom, speed, loop
        let mut right = bar.right() - PAD;
        let mut place = |w: f32, h: f32| {
            right -= w;
            let r = Rect::from_min_size(Pos2::new(right, middle - h / 2.0), Vec2::new(w, h));
            right -= GAP;
            r
        };
        let full = icon_button(ui, place(ICON_BUTTON.x, ICON_BUTTON.y), "full", Icon::Full, Lit::No);
        if full.clicked() {
            actions.push(Action::Fullscreen);
        }
        self.hint(&full, keys::named(Key::P, keys::NONE));
        // sound: the button mutes, and shows when nothing is to be heard. its slider comes out to its left while
        // the pointer is on either, or holds the slider. plain, like everything on the bar
        let button = place(ICON_BUTTON.x, ICON_BUTTON.y);
        let silent = model.muted || model.volume <= 0.0 || !carries_sound(model);
        let mute = icon_button(ui, button, "mute", if silent { Icon::Mute } else { Icon::Sound }, Lit::No);
        if mute.clicked() {
            actions.push(Action::Mute);
        }
        self.hint(&mute, keys::line(Key::M, keys::NONE));
        let slider = ui.scope_id().with("volume");
        let over = ui.ctx().pointer_latest_pos().is_some_and(|p| self.volume_rect.contains(p)) || ui.ctx().is_being_dragged(slider);
        let out = ui.ctx().animate_bool_with_time(ui.scope_id().with("volume-out"), over, 0.12);
        self.volume_rect = button;
        if out > 0.0 {
            let room = place(68.0 * out, 16.0);
            if out >= 1.0 {
                if let Some(to) = widgets::slider(ui, "volume", room.shrink2(Vec2::new(4.0, 0.0)), if model.muted { 0.0 } else { model.volume }, FG) {
                    actions.push(Action::Volume(to));
                }
            }
            self.volume_rect = self.volume_rect.union(room);
        }
        let rect = place(ICON_BUTTON.x, ICON_BUTTON.y);
        // lit while markup mode is on, which it opens to
        let lit = if self.markup.on {
            Lit::Active
        } else if self.menu == Some(Menu::Shot) {
            Lit::Open
        } else {
            Lit::No
        };
        let shot = icon_button(ui, rect, "shot", Icon::Shot, lit);
        if shot.clicked() {
            self.toggle_menu(Menu::Shot, rect);
        }
        self.hint(&shot, keys::line(Key::S, keys::NONE));
        // metadata: a click pins the panel; resting the pointer on the button brings it up for as long as the
        // pointer stays on the button or the panel. a fault in the file puts an amber dot on the button
        let rect = place(ICON_BUTTON.x, ICON_BUTTON.y);
        let lit = if self.facts_pinned {
            Lit::Active
        } else if self.facts_up {
            Lit::Open
        } else {
            Lit::No
        };
        let info = icon_button(ui, rect, "info", Icon::Info, lit);
        if matches!(&model.facts, Facts::Ready(meta) if meta.faults() > 0) {
            painter.circle_filled(Pos2::new(rect.right() - 7.5, rect.top() + 7.5), 2.5, AMBER);
        }
        if info.clicked() {
            self.toggle_facts();
        }
        self.hint(&info, keys::named(Key::I, keys::NONE));
        self.facts_button = rect;
        let on = info.hovered() || (self.facts_up && ui.ctx().pointer_latest_pos().is_some_and(|p| self.facts_rect.contains(p)));
        let (since, wait) = if on {
            self.facts_left = None;
            (*self.facts_since.get_or_insert(now), FACTS_IN)
        } else {
            self.facts_since = None;
            (*self.facts_left.get_or_insert(now), FACTS_OUT)
        };
        if now - since >= wait {
            self.facts_up = on;
        } else if on != self.facts_up {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(wait - (now - since)));
        }
        let turned = model.look != Look::default() || model.guides.grid || model.guides.center;
        let rect = place(ICON_BUTTON.x, ICON_BUTTON.y);
        let lit = if self.menu == Some(Menu::View) {
            Lit::Open
        } else if turned {
            Lit::Active
        } else {
            Lit::No
        };
        let view = icon_button(ui, rect, "view", Icon::Guides, lit);
        if view.clicked() {
            self.toggle_menu(Menu::View, rect);
        }
        self.hint(&view, keys::line(Key::H, keys::NONE));
        let speed = format!("{}×", (model.speed * 100.0).round() / 100.0);
        let looping = match (model.seam, model.looping) {
            (Some(_), _) if narrow => "analysis".to_string(),
            (Some(_), _) => format!("analysis {:.1}s", model.seam_seconds),
            (None, Loop::Off) => (if narrow { "off" } else { "loop off" }).to_string(),
            (None, Loop::One) => (if narrow { "one" } else { "loop one" }).to_string(),
            (None, Loop::All) => (if narrow { "all" } else { "loop all" }).to_string(),
        };
        for (menu, name, icon, label, tip) in [
            (Menu::Zoom, "zoom", None, model.zoom_label.clone(), keys::line(Key::Digit0, keys::NONE)),
            (Menu::Speed, "speed", None, speed, keys::line(Key::Minus, keys::SHIFT)),
            (Menu::Loop, "loop", Some(Icon::Loop), looping, keys::named(Key::R, keys::NONE)),
        ] {
            let galley = painter.layout_no_wrap(label, font(MONO_MEDIUM, 11.5), FG);
            let rect = place(galley.size().x + 2.0 * PAD + if icon.is_some() { 16.0 + 6.0 } else { 0.0 }, TEXT_BUTTON);
            let button = text_button(ui, rect, name, icon, galley, self.menu == Some(menu));
            if button.clicked() {
                self.toggle_menu(menu, rect);
            }
            self.hint(&button, tip);
        }

        self.timebar(ui, Rect::from_min_max(Pos2::new(bar.left(), middle + TEXT_BUTTON / 2.0), bar.max), bar.top(), model, actions);
    }

    /// opens a menu over its button, or closes it when it is the one open.
    fn toggle_menu(&mut self, menu: Menu, button: Rect) {
        self.menu = (self.menu != Some(menu)).then_some(menu);
        self.menu_button = button;
    }

    /// notes the button of the strip or the bar the pointer is on, with what its tooltip says: what it does, and its
    /// key.
    fn hint(&mut self, button: &egui::Response, tip: Option<(String, String)>) {
        if button.hovered() {
            self.hinted = tip.map(|tip| (button.id, button.rect, tip));
        }
    }

    /// the tooltip of the button the pointer rests on: in a chip 7 over the bar or under the strip, as the menus stand,
    /// from the button's edge on the side of the window it is on. it comes a moment after the pointer does, and never
    /// while a menu, the metadata panel, the mark's card or a panel is up, or a button is held.
    fn tooltip(&mut self, ui: &egui::Ui, full: Rect, now: f64) {
        let quiet = self.menu.is_some() || self.commands.is_some() || self.panel.is_some() || self.card.is_some() || self.facts_up || ui.input(|i| i.pointer.any_down());
        let Some((id, button, (does, key))) = self.hinted.take().filter(|_| !quiet) else {
            self.tip = None;
            return;
        };
        let since = self.tip.filter(|(on, _)| *on == id).map_or(now, |(_, since)| since);
        self.tip = Some((id, since));
        if now - since < TIP_IN {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(TIP_IN - (now - since)));
            return;
        }
        let (left, low) = (button.center().x < full.center().x, button.center().y > full.center().y);
        let corner = Pos2::new(if left { button.left() } else { button.right() }, if low { full.bottom() - (1.0 + ROW + TIME) - 7.0 } else { full.top() + STRIP + 7.0 });
        let anchor = Align2([if left { egui::Align::Min } else { egui::Align::Max }, if low { egui::Align::Max } else { egui::Align::Min }]);
        // on a mac command does what control does, and a hint names the key held there
        let key = if cfg!(target_os = "macos") { key.replace("ctrl", "cmd") } else { key };
        let key = if key.is_empty() { key } else { format!(" · {key}") };
        chip(ui, corner, anchor, &[(does, FG), (key, DIM)], 1.0, SANS);
    }

    /// the timebar: the whole width of the window. it thickens under the pointer; a click or a drag goes there.
    /// in loop analysis it is the round: the clip's last seconds, the seam marked in the middle, its first seconds.
    /// `over` is the top of the bar, which the box naming the frame under the pointer stands above.
    fn timebar(&mut self, ui: &mut egui::Ui, area: Rect, over: f32, model: &Model, actions: &mut Vec<Action>) {
        let painter = ui.painter().clone();
        let response = ui.interact(area, ui.scope_id().with("timebar"), Sense::click_and_drag());
        let (at, last) = match model.seam {
            Some((place, length)) => (place, length.saturating_sub(1).max(1) as f32),
            None => (model.frame, model.frames.saturating_sub(1).max(1) as f32),
        };
        let stop_at = |x: f32| (((x - area.left()) / area.width()).clamp(0.0, 1.0) * last).round() as usize;
        let scrubbing = response.is_pointer_button_down_on();
        let thick = ui.ctx().animate_bool_with_time(ui.scope_id().with("thick"), response.hovered() || scrubbing, 0.1);
        let height = TRACK_THIN + (TRACK_THICK - TRACK_THIN) * thick;
        let track = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - height), area.max);
        let head = area.left() + area.width() * (at as f32 / last).min(1.0);
        painter.rect_filled(track, 0.0, TRACK);
        painter.rect_filled(Rect::from_min_max(track.min, Pos2::new(head, track.bottom())), 0.0, FILL);
        if model.seam.is_some() {
            painter.rect_filled(Rect::from_min_max(Pos2::new(area.center().x - 0.5, track.top() - 9.0), Pos2::new(area.center().x + 0.5, track.bottom())), 0.0, Color32::WHITE);
        } else {
            // a sequence's missing frames: amber, the color of a fault in the file
            for gap in model.gaps {
                let x = area.left() + area.width() * (*gap as f32 / last).min(1.0);
                painter.rect_filled(Rect::from_min_max(Pos2::new(x - 1.0, track.top() - 4.0), Pos2::new(x + 1.0, track.bottom())), 0.0, AMBER);
            }
            if let Some(marks) = model.marks {
                markup::ticks(&painter, area, track, last, marks);
            }
        }
        painter.rect_filled(Rect::from_min_max(Pos2::new(head - 1.0, track.top() - 4.0), Pos2::new(head + 1.0, track.bottom())), 0.0, Color32::WHITE);

        // a drag follows the pointer; a click goes there too, even one that came and went within a single pass
        if let Some(p) = response.interact_pointer_pos().filter(|_| scrubbing || response.clicked()) {
            let to = stop_at(p.x);
            if to != at {
                actions.push(if model.seam.is_some() { Action::SeekPlace(to) } else { Action::Seek(to) });
            }
        }
        // the frame under the pointer: its number, and over it the picture nearest it of those had, which is at once
        // one of the clip's timebar strip and then the frame itself, or the key frame nearest it, once the spare
        // reader has read that. the design's box (fig. 04): its line, then the picture 160 by 90 two in from it and the
        // number in the 20 under the picture
        self.time_hover = response.hover_pos().filter(|_| model.seam.is_none()).map(|p| stop_at(p.x));
        if let (Some(p), Some(frame)) = (response.hover_pos(), self.time_hover) {
            let nearest = self.time_strip.iter().chain(&self.time_read).min_by_key(|(of, ..)| of.abs_diff(frame));
            if let Some((of, size, pixels)) = nearest.filter(|(of, ..)| self.time_picture.as_ref().is_none_or(|(shown, _)| shown != of)) {
                self.time_picture = Some((*of, ui.ctx().load_texture("timebar picture", egui::ColorImage::from_rgb(*size, pixels), egui::TextureOptions::LINEAR)));
            }
            let galley = painter.layout_no_wrap(stamp(frame, model.first, model.clock, self.timecode).1, font(MONO_MEDIUM, 10.5), FG);
            let size = match &self.time_picture {
                Some(_) => Vec2::new(164.0, 3.0 + 90.0 + 20.0 + 1.0),
                None => galley.size() + Vec2::new(16.0, 10.0),
            };
            let left = (p.x - size.x / 2.0).clamp(area.left() + 4.0, area.right() - size.x - 4.0);
            let rect = Rect::from_min_size(Pos2::new(left, over - 7.0 - size.y), size);
            painter.rect_filled(rect, 0.0, SOLID);
            painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
            let mut number = rect;
            if let Some((_, picture)) = &self.time_picture {
                let place = Rect::from_min_size(rect.min + Vec2::splat(3.0), Vec2::new(160.0, 90.0));
                painter.rect_filled(place, 0.0, Color32::BLACK);
                let fitted = Rect::from_center_size(place.center(), picture.size_vec2() * (place.width() / picture.size_vec2().x).min(place.height() / picture.size_vec2().y));
                painter.image(picture.id(), fitted, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                number = Rect::from_min_max(Pos2::new(rect.left(), place.bottom()), Pos2::new(rect.right(), place.bottom() + 20.0));
            }
            painter.galley(Pos2::new(number.center().x - galley.size().x / 2.0, text_top(&painter, font(MONO_MEDIUM, 10.5), number.center().y)), galley, FG);
        }
    }
}

/// the dotted line under words that are a link, or a count that opens a list: the row of pixels 3 under the figures
/// of the type laid out at `at`, every other one lit, for `width`. as a mesh of its own: a line or a rect a pixel
/// across is rounded away to nothing.
fn dotted(painter: &egui::Painter, font_id: FontId, at: Pos2, width: f32, color: Color32) {
    let figure = painter.layout_no_wrap("0".to_string(), font_id, color);
    let glyph = &figure.rows[0].row.glyphs[0];
    let under = (at.y + glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y).round() + 3.0;
    let (mut dots, mut dot) = (egui::Mesh::default(), at.x.round());
    while dot < at.x + width {
        dots.add_colored_rect(Rect::from_min_size(Pos2::new(dot, under), Vec2::splat(1.0)), color);
        dot += 2.0;
    }
    painter.add(dots);
}

/// how far the strip's tabs are scrolled: no further than their end, and, when another tab has just come up, far
/// enough for that tab to be in view, moving as little as that takes.
fn tab_scroll(scroll: f32, widths: &[f32], up: usize, room: f32, came_up: bool) -> f32 {
    let before: f32 = widths[..up.min(widths.len())].iter().sum();
    let width = widths.get(up).copied().unwrap_or(0.0);
    let scroll = if came_up { scroll.max(before + width - room).min(before) } else { scroll };
    scroll.clamp(0.0, (widths.iter().sum::<f32>() - room).max(0.0))
}

/// how wide the pieces of a folder tab's label are before the clip's name: the folder's name and its slash, an arrow
/// with its room, the clip's place, and the space before the name.
fn folder_pieces(painter: &egui::Painter, place: Place, family: &str) -> [f32; 4] {
    let wide = |text: String, family: &str| painter.layout_no_wrap(text, font(family, 11.5), FG).size().x;
    [wide(format!("{} /", place.folder), family), wide("‹".to_string(), family) + 2.0 * NAV_PAD, wide(format!("{} of {}", place.at + 1, place.count), MONO_MEDIUM), wide(" ".to_string(), family)]
}

/// a name cut in its middle to fit `width`, so what ends it (a version, a frame range) stays in view.
fn cut_middle(painter: &egui::Painter, name: &str, font_id: FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let whole = painter.layout_no_wrap(name.to_string(), font_id.clone(), color);
    let letters: Vec<char> = name.chars().collect();
    if whole.size().x <= width || letters.len() < 3 {
        return whole;
    }
    // the type is monospaced: so many letters fit, one of them the mark
    let keep = ((width / (whole.size().x / letters.len() as f32)).floor() as usize).clamp(2, letters.len() - 1) - 1;
    let tail = keep / 2;
    let cut: String = letters[..keep - tail].iter().chain(&['…']).chain(&letters[letters.len() - tail..]).collect();
    painter.layout_no_wrap(cut, font_id, color)
}

/// what just happened, low in the middle of the window and over everything: what was done in bright, the rest of
/// it, after the first dot, in dim.
fn toast(ui: &egui::Ui, full: Rect, said: &str) {
    let (done, rest) = said.split_once(" · ").map_or((said, ""), |(done, rest)| (done, rest));
    let mut job = LayoutJob::default();
    job.append(done, 0.0, TextFormat { font_id: font(SANS, 12.5), color: FG, ..Default::default() });
    if !rest.is_empty() {
        job.append(&format!(" · {rest}"), 0.0, TextFormat { font_id: font(SANS, 12.5), color: DIM, ..Default::default() });
    }
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("toast")));
    let galley = painter.layout_job(job);
    let rect = Align2::CENTER_BOTTOM.anchor_size(Pos2::new(full.center().x, full.bottom() - 64.0), galley.size() + Vec2::new(24.0, 18.0));
    painter.rect_filled(rect, 0.0, SOLID);
    painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
    painter.galley(Pos2::new(rect.left() + 12.0, text_top(&painter, font(SANS, 12.5), rect.center().y)), galley, FG);
}

/// a tab whose file did not open: its name and why, in the middle.
fn message(ui: &egui::Ui, area: Rect, title: &str, why: &str) {
    let painter = ui.painter().with_clip_rect(area);
    painter.rect_filled(area, 0.0, MESSAGE);
    let width = (area.width() - 40.0).min(460.0);
    let title = painter.layout(title.to_string(), font("sans-semibold", 15.0), FG, width);
    let why = painter.layout(why.to_string(), font(SANS, 13.0), DIM, width);
    let top = area.center().y - (title.size().y + 8.0 + why.size().y) / 2.0;
    let under = top + title.size().y + 8.0;
    painter.galley(Pos2::new(area.center().x - title.size().x / 2.0, top), title, FG);
    painter.galley(Pos2::new(area.center().x - why.size().x / 2.0, under), why, DIM);
}

/// the height the bar's row of controls is centered on: the middle of what is seen of the bar, from the line along
/// its top to the timebar's track at rest.
fn row_middle(bar: Rect) -> f32 {
    (bar.top() + 1.0 + bar.bottom() - TRACK_THIN) / 2.0
}

/// what the loop analysis chip says, while it is on.
fn seam_chip(model: &Model) -> Option<Vec<(String, Color32)>> {
    model.seam.map(|_| seam_words(model.seam_seconds, model.seam_change))
}

/// the loop analysis chip's words, for the seconds each side of the seam and what is known of the seam itself.
fn seam_words(seconds: f64, change: Option<Seam>) -> Vec<(String, Color32)> {
    let mut parts = vec![("loop analysis".to_string(), FG), (format!(" · last {seconds:.1} s "), FG), ("into".to_string(), DIM), (format!(" first {seconds:.1} s"), FG)];
    // a last frame that is the first frame again is a fault in the file: the loop shows it twice, and stutters
    match change {
        Some(seam) if seam.identical => parts.push((" · the last frame and the first are the same".to_string(), AMBER)),
        Some(seam) if seam.change < 0.0005 => parts.push((" · the seam changes under 0.1%".to_string(), DIM)),
        Some(seam) => parts.push((format!(" · the seam changes {:.1}%", seam.change * 100.0), DIM)),
        None => {}
    }
    parts
}

/// what the speed chip says, when the clip is not running at its own rate: "2× speed", "−4× reverse".
fn speed_chip(model: &Model) -> Option<Vec<(String, Color32)>> {
    let number = |rate: f64| format!("{}× ", (rate * 100.0).round() / 100.0);
    // a clip with sound says when its sound is off for the speed
    let off = if model.sound && !carries_sound(model) { " · audio off" } else { "" };
    if model.shuttle != 0 && (model.shuttle != 1 || model.speed != 1.0) {
        let sign = if model.shuttle < 0 { "−" } else { "" };
        return Some(vec![(format!("{sign}{}", number(model.shuttle.abs() as f64)), FG), (format!("{}{off}", if model.shuttle < 0 { "reverse" } else { "shuttle" }), DIM)]);
    }
    (model.speed != 1.0).then(|| vec![(number(model.speed), FG), (format!("speed{off}"), DIM)])
}

/// whether play as it is set carries a clip's sound: forward, from half speed to double. L pressed once is 1×.
fn carries_sound(model: &Model) -> bool {
    match model.shuttle {
        0 => (0.5..=2.0).contains(&model.speed),
        shuttle => shuttle == 1,
    }
}

/// a chip: a few words in a dark box, placed by one of its corners, its first part in `first`'s type and the rest
/// mono: a tooltip's words are archivo and its key mono, and the chips on the picture, which say values, are mono
/// throughout. returns the box.
fn chip(ui: &egui::Ui, corner: Pos2, anchor: Align2, parts: &[(String, Color32)], alpha: f32, first: &str) -> Rect {
    let in_type = |i: usize| if i == 0 && first == SANS { font(SANS, 12.0) } else { font(MONO_MEDIUM, 11.0) };
    let mut job = LayoutJob::default();
    for (i, (text, color)) in parts.iter().enumerate() {
        job.append(text, 0.0, TextFormat { font_id: in_type(i), color: color.gamma_multiply(alpha), ..Default::default() });
    }
    let painter = ui.painter();
    let galley = painter.layout_job(job);
    let rect = anchor.anchor_size(corner, galley.size() + Vec2::new(16.0, 12.0));
    painter.rect_filled(rect, 0.0, CHIP.gamma_multiply(alpha));
    painter.rect_stroke(rect, 0.0, (1.0, LINE.gamma_multiply(alpha)), egui::StrokeKind::Inside);
    painter.galley(Pos2::new(rect.left() + 8.0, text_top(painter, in_type(0), rect.center().y)), galley, FG);
    rect
}

/// how a button on the bar is lit: not, because its menu is open, or because what it sets is on
#[derive(Clone, Copy, PartialEq)]
enum Lit {
    No,
    Open,
    Active,
}

/// a 30 x 28 button holding one icon.
fn icon_button(ui: &egui::Ui, rect: Rect, name: &str, icon: Icon, lit: Lit) -> egui::Response {
    let response = ui.interact(rect, ui.scope_id().with(name), Sense::click());
    if response.is_pointer_button_down_on() || lit == Lit::Active {
        ui.painter().rect_filled(rect, 3.0, PRESSED);
    } else if response.hovered() || lit == Lit::Open {
        ui.painter().rect_filled(rect, 3.0, HOVER);
    }
    icons::draw(ui.painter(), rect, icon, FG);
    response
}

/// a 28-high button holding text, with an icon before it or none, lit while its menu is open.
fn text_button(ui: &mut egui::Ui, rect: Rect, name: &str, icon: Option<Icon>, galley: Arc<egui::Galley>, open: bool) -> egui::Response {
    let response = ui.interact(rect, ui.scope_id().with(name), Sense::click());
    if response.hovered() || open {
        ui.painter().rect_filled(rect, 3.0, HOVER);
    }
    let mut left = rect.left() + PAD;
    if let Some(icon) = icon {
        icons::draw(ui.painter(), Rect::from_min_size(Pos2::new(left, rect.center().y - 8.0), Vec2::splat(16.0)), icon, FG);
        left += 16.0 + 6.0;
    }
    ui.painter().galley(Pos2::new(left, text_top(ui.painter(), font(MONO_MEDIUM, 11.5), rect.center().y)), galley, FG);
    response
}

/// "F 000120 / 000600", or "TC 00:00:02:00 / 00:00:10:00": a small dim label, the place, and the total in dim.
/// on a narrow bar the timecode's total is left out.
fn readout(model: &Model, timecode: bool, narrow: bool) -> LayoutJob {
    let (label, at) = stamp(model.frame, model.first, model.clock, timecode);
    // the total is a count and a length of time: neither starts where the clip's own numbers do
    let total = Some(stamp(model.frames, 0, model.clock.map(|clock| Timecode { start: 0, ..clock }), timecode).1).filter(|_| label == "F" || !narrow);
    let text = |size: f32, color: Color32, spacing: f32| TextFormat { font_id: font(MONO_MEDIUM, size), color, extra_letter_spacing: spacing, ..Default::default() };
    let mut job = LayoutJob::default();
    job.append(label, 0.0, text(9.5, DIM, 9.5 * 0.12));
    job.append(&at, 2.0 + 7.0, text(11.5, FG, 11.5 * 0.02));
    if let Some(total) = total {
        job.append(&format!(" / {total}"), 0.0, text(11.5, DIM, 11.5 * 0.02));
    }
    job
}

/// a frame as the readout names it: "F" and its number, counted from `first`, or "TC" and what the clip's clock
/// says, when that is asked for and the clip has one.
fn stamp(frame: usize, first: usize, clock: Option<Timecode>, timecode: bool) -> (&'static str, String) {
    match clock.filter(|_| timecode) {
        Some(clock) => ("TC", clock.at(frame)),
        None => ("F", format!("{:06}", frame + first)),
    }
}

/// a line of trouble with the amber edge: the one place amber is used, for a fault in the file. returns its box.
fn banner(ui: &egui::Ui, at: Pos2, width: f32, text: &str) -> Rect {
    let painter = ui.painter();
    let galley = painter.layout(text.to_string(), font(SANS, 12.5), DIM, width - 24.0);
    let rect = Rect::from_min_size(at, galley.size() + Vec2::new(22.0, 16.0));
    painter.rect_filled(rect, 0.0, BANNER);
    painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())), 0.0, AMBER);
    painter.galley(rect.min + Vec2::new(12.0, 8.0), galley, DIM);
    rect
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// the settings a test's screen carries: the defaults
    pub(in crate::ui) static DEFAULTS: std::sync::LazyLock<Settings> = std::sync::LazyLock::new(Settings::default);

    /// the row of controls sits in the middle of the bar as it is seen: as far under the line along the bar's top as
    /// it is over the timebar's track at rest.
    /// text is centered by eye: the middle of its figures and capitals is on the line it is centered on, in every type
    /// the controls use. (centered by its line's box, which has more room above the letters than below, it sits low.)
    #[test]
    fn text_is_centered_by_its_figures_not_by_its_lines_box() {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for (family, size) in [(MONO_MEDIUM, 11.5), (MONO, 11.5), (MONO_MEDIUM, 11.0), (MONO_MEDIUM, 10.0), (SANS, 12.0)] {
                let top = text_top(ui.painter(), font(family, size), 100.0);
                let figure = ui.painter().layout_no_wrap("0".to_string(), font(family, size), Color32::WHITE);
                let glyph = &figure.rows[0].row.glyphs[0];
                let middle = top + glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y / 2.0;
                assert!((middle - 100.0).abs() < 0.2, "{family} {size}: the middle of a figure is at {middle}, not at 100");
            }
        });
        out.textures_delta.clear();
    }

    /// the design's fig. 09: a step taken with the controls away says the frame it landed on, at the lower left, for
    /// a moment; with the controls up it says nothing, the readout being there.
    #[test]
    fn a_step_with_the_controls_away_says_its_frame() {
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        let screen = |frame| Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(Model { frame, ..model() }), error: None, opening: None, list: None, browsing: false, fullscreen: false, toast: None, settings: &DEFAULTS };
        let pass = |ui: &mut Ui, frame| ui.run(&screen(frame)).textures.clear();
        pass(&mut ui, 0);
        assert!(ui.flash.is_none(), "nothing is said of a clip that only came up");
        pass(&mut ui, 1);
        let said: String = ui.flash.as_ref().expect("the step is said").0.iter().map(|(words, _)| words.as_str()).collect();
        assert_eq!(said, "F 000001");
        // the pointer over the picture brings the controls up: a step then is not said again
        ui.flash = None;
        ui.pointer_move(320.0, 180.0);
        pass(&mut ui, 1);
        pass(&mut ui, 2);
        assert!(ui.flash.is_none(), "said with the controls up");
        assert_eq!((stamp(742, 0, Some(Timecode::counted(60)), false), stamp(742, 0, Some(Timecode::counted(24)), true)), (("F", "000742".to_string()), ("TC", "00:00:30:22".to_string())));
        // the file's own timecode, and frame numbers from 1; a clip with no one rate has no clock and is counted in frames
        let own = Timecode::parse("01:00:00;00", 30, 30);
        assert_eq!((stamp(1800, 0, own, true), stamp(0, 1, own, false), stamp(5, 1, None, true)), (("TC", "01:01:00;02".to_string()), ("F", "000001".to_string()), ("F", "000006".to_string())));
        // the readout: where the clip is by its own clock, and how long it is
        let job = readout(&Model { frame: 30, frames: 60, clock: own, ..model() }, true, false);
        assert_eq!(job.text, "TC01:00:01;00 / 00:00:02;00");
    }

    /// thirty tabs at their least width in room for five: the strip scrolls to the tab that comes up, and stays
    /// where the wheel puts it until another does.
    #[test]
    fn more_tabs_than_fit_scroll_to_the_one_that_is_up() {
        let widths = [TAB_MIN; 30];
        let room = 5.0 * TAB_MIN;
        // tabs that fit do not scroll
        assert_eq!(tab_scroll(40.0, &widths[..5], 4, room, true), 0.0);
        // the first tab up: no scroll. the last: scrolled to the end. one in the middle, from there: just into view
        assert_eq!(tab_scroll(0.0, &widths, 0, room, true), 0.0);
        assert_eq!(tab_scroll(0.0, &widths, 29, room, true), 25.0 * TAB_MIN);
        assert_eq!(tab_scroll(25.0 * TAB_MIN, &widths, 10, room, true), 10.0 * TAB_MIN);
        assert_eq!(tab_scroll(0.0, &widths, 10, room, true), 6.0 * TAB_MIN);
        // the wheel's scroll stands while the same tab is up, within the tabs' ends
        assert_eq!(tab_scroll(3.0 * TAB_MIN, &widths, 29, room, false), 3.0 * TAB_MIN);
        assert_eq!((tab_scroll(-50.0, &widths, 29, room, false), tab_scroll(9999.0, &widths, 0, room, false)), (0.0, 25.0 * TAB_MIN));
    }

    /// a sign the chip's font does not have is drawn as an empty box.
    #[test]
    fn the_loop_analysis_chip_is_all_in_its_font() {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let has = |words: &str| ui.ctx().fonts_mut(|f| f.has_glyphs(&font(MONO_MEDIUM, 11.0), words));
            for change in [None, Some(Seam { identical: true, change: 0.0 }), Some(Seam { identical: false, change: 0.024 }), Some(Seam { identical: false, change: 0.0001 })] {
                let words: String = seam_words(3.0, change).into_iter().map(|(text, _)| text).collect();
                assert!(has(&words), "not all in the font: {words}");
            }
            // the design's arrow between the two halves is not in the bundled fonts, which is why the chip says "into"
            assert!(!has("→"));
        });
        out.textures_delta.clear();
    }

    /// a 64 x 64 clip standing on its first frame, fitted into a 640 x 360 window, as the controls are told of it
    pub(super) fn model() -> Model<'static> {
        Model {
            frame: 0,
            frames: 3,
            playing: false,
            rate: Some((60, 1)),
            clock: Some(Timecode::counted(60)),
            first: 0,
            fault: None,
            gaps: &[],
            linear: false,
            looping: Loop::One,
            seam: None,
            seam_seconds: 2.0,
            speed: 1.0,
            shuttle: 0,
            zoom_label: "fit 563%".to_string(),
            zoom: 5.625,
            zoomed: false,
            exact: false,
            overflows: false,
            pixel: None,
            value: None,
            picture: (140.0, 0.0, 360.0, 360.0),
            shown_size: (64, 64),
            look: Look::default(),
            guides: Guides { grid: false, center: false, tile: (120, 120), color: [61, 240, 255] },
            facts: Facts::Reading,
            decoded: "",
            chroma: None,
            sound: false,
            volume: 1.0,
            muted: true,
            shots_to: "pictures / eyepiece",
            folder: false,
            seam_change: None,
            extras: Extras { caption: false, guides: false, markup: true },
            marks: None,
        }
    }

    /// drives the controls over that clip with the pointer, a pass for each step, and gathers what they asked for
    pub(super) fn drive(ui: &mut Ui, steps: &[(f32, f32, Option<(crate::event::Button, bool)>)]) -> Vec<Action> {
        drive_with(ui, model(), steps)
    }

    /// `drive`, over the clip `model` tells of
    pub(super) fn drive_with(ui: &mut Ui, model: Model, steps: &[(f32, f32, Option<(crate::event::Button, bool)>)]) -> Vec<Action> {
        drive_on(ui, &Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(model), error: None, opening: None, list: None, browsing: false, fullscreen: false, toast: None, settings: &DEFAULTS }, steps)
    }

    /// `drive`, over what `screen` shows
    fn drive_on(ui: &mut Ui, screen: &Screen, steps: &[(f32, f32, Option<(crate::event::Button, bool)>)]) -> Vec<Action> {
        let mut actions = Vec::new();
        for &(x, y, button) in steps {
            match button {
                Some((button, down)) => ui.pointer_button(button, down, x, y),
                None => ui.pointer_move(x, y),
            }
            let mut out = ui.run(screen);
            out.textures.clear();
            actions.extend(out.actions);
        }
        actions
    }

    /// a click at one place, and the pass after it
    fn click(x: f32, y: f32) -> [(f32, f32, Option<(crate::event::Button, bool)>); 4] {
        use crate::event::Button::Left;
        [(x, y, None), (x, y, Some((Left, true))), (x, y, Some((Left, false))), (x, y, None)]
    }

    /// zoomed in past the window, the navigator stands at the lower left, clear of the bar: a plan of the whole
    /// picture, 112 on its longer side, with the part in view marked. a press in it brings that part of the picture
    /// to the middle of the window. fitted, there is none, and a click there is a click on the picture.
    #[test]
    fn the_navigator_pans_a_picture_zoomed_past_the_window() {
        let screen = |clip| Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(clip), error: None, opening: None, list: None, browsing: false, fullscreen: false, toast: None, settings: &DEFAULTS };
        // a 1920 x 1080 picture at 1:1 in the 640 x 360 window, its middle in the window's
        let zoomed = || Model { picture: (-640.0, -360.0, 1920.0, 1080.0), overflows: true, zoomed: true, ..model() };
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        drive_on(&mut ui, &screen(zoomed()), &[(300.0, 100.0, None)]);
        // the controls are up once they have faded in: the navigator's foot is then 62 above the window's, and it
        // is 112 by 63 from 10 in. a quarter of the way along and down it:
        std::thread::sleep(Duration::from_millis(250));
        let at = (10.0 + 28.0, 360.0 - 62.0 - 63.0 + 15.75);
        assert_eq!(drive_on(&mut ui, &screen(zoomed()), &click(at.0, at.1)), vec![Action::Pan((480.0, 270.0))]);
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(drive_on(&mut ui, &screen(model()), &click(at.0, at.1)), vec![Action::TogglePlay], "fitted, no navigator");
    }

    /// the address as a field, through the controls: control + L makes it one with the whole path selected, control
    /// + C hands that path over for the clipboard, and what the clipboard held is typed over it, here a path in
    /// quotes as the system's file browser copies one, gone to on enter.
    #[test]
    fn the_address_field_copies_its_path_and_takes_a_pasted_one() {
        let folder = std::env::temp_dir();
        let mut ui = Ui::new((1240, 700), 1.0, false, Some(folder.clone()), None);
        let screen = Screen { tabs: vec!["new tab"], current: 0, places: Vec::new(), clip: None, error: None, opening: None, list: None, browsing: true, fullscreen: false, toast: None, settings: &DEFAULTS };
        let pass = |ui: &mut Ui| {
            let mut out = ui.run(&screen);
            out.textures.clear();
            out.copied
        };
        pass(&mut ui);
        assert!(!ui.typing());
        ui.browser_address();
        pass(&mut ui);
        pass(&mut ui);
        assert!(ui.typing());
        ui.key(Key::C, Mods { ctrl: true, ..Mods::default() });
        assert_eq!(pass(&mut ui), Some(ui.browser_folder().to_string_lossy().into_owned()));
        let other = folder.join("eyepiece nowhere");
        ui.paste(format!("\"{}\"", other.display()));
        pass(&mut ui);
        ui.key(Key::Enter, Mods::default());
        pass(&mut ui);
        pass(&mut ui);
        assert!(!ui.typing());
        assert_eq!(ui.browser_folder(), other);
    }

    /// back, at the left of the address, pressed twice as fast as a double click goes back two folders, not one.
    #[test]
    fn back_pressed_twice_fast_goes_back_two_folders() {
        let mut ui = Ui::new((1240, 700), 1.0, false, Some(std::env::temp_dir()), None);
        let screen = Screen { tabs: vec!["browser"], current: 0, places: Vec::new(), clip: None, error: None, opening: None, list: None, browsing: true, fullscreen: false, toast: None, settings: &DEFAULTS };
        let pass = |ui: &mut Ui| ui.run(&screen).textures.clear();
        pass(&mut ui);
        let start = ui.browser_folder().to_path_buf();
        for name in ["one", "two"] {
            ui.browser_address();
            pass(&mut ui);
            pass(&mut ui);
            ui.paste(name.to_string());
            pass(&mut ui);
            ui.key(Key::Enter, Mods::default());
            pass(&mut ui);
            pass(&mut ui);
        }
        assert_eq!(ui.browser_folder(), start.join("one").join("two"));
        // the first of the address's buttons, 6 in from the sidebar's 150, in the line under the strip
        drive_on(&mut ui, &screen, &click(171.0, 50.0));
        assert_eq!(ui.browser_folder(), start.join("one"));
        drive_on(&mut ui, &screen, &click(171.0, 50.0));
        assert_eq!(ui.browser_folder(), start);
    }

    /// a name with letters our type lacks is written with the system's, not with empty boxes: ours has no ł, which
    /// a folder of the owner's is named with.
    #[test]
    fn letters_our_type_lacks_are_written_in_the_systems() {
        let writes = |fonts: egui::FontDefinitions| {
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts);
            let mut all = false;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                all = [SANS, MONO, MONO_MEDIUM].iter().all(|family| ui.ctx().fonts_mut(|f| f.has_glyphs(&font(family, 12.0), "Sławek, Ελένη, Юрий")));
            });
            out.textures_delta.clear();
            all
        };
        assert!(!writes(fonts()), "our own type writes these after all: the system's is not needed");
        assert!(writes(fonts_with_system()));
    }

    /// the pin at the bar's far left keeps the controls up when the pointer leaves the window, where they go at
    /// once without it; a second click lets them go again. the bar's row is 32 above the window's bottom, the pin
    /// 23 in, and the previous-frame button after it.
    #[test]
    fn the_pin_keeps_the_controls_up() {
        let mut ui = Ui::new((900, 360), 1.0, false, None, None);
        assert_eq!(drive(&mut ui, &click(23.0, 328.0)), vec![]);
        assert!(ui.pinned() && ui.shown);
        ui.pointer_leave();
        let mut out = ui.run(&Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(model()), error: None, opening: None, list: None, browsing: false, fullscreen: false, toast: None, settings: &DEFAULTS });
        out.textures.clear();
        assert!(ui.shown && out.shown, "pinned, the controls stay when the pointer leaves");
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(drive(&mut ui, &click(63.0, 328.0)), vec![Action::Step(-1)], "the previous-frame button stands after the pin");
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(drive(&mut ui, &click(23.0, 328.0)), vec![]);
        assert!(!ui.pinned());
        ui.pointer_leave();
        let mut out = ui.run(&Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(model()), error: None, opening: None, list: None, browsing: false, fullscreen: false, toast: None, settings: &DEFAULTS });
        out.textures.clear();
        assert!(!ui.shown, "not pinned, they go with the pointer");
    }

    /// filling the display takes the controls away at once (the owner's word), though the pointer is on the bar and
    /// the window, changing size, says it moved; pinned, they stay.
    #[test]
    fn entering_fullscreen_takes_the_controls_away() {
        let screen = |fullscreen| Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(model()), error: None, opening: None, list: None, browsing: false, fullscreen, toast: None, settings: &DEFAULTS };
        let mut ui = Ui::new((900, 360), 1.0, false, None, None);
        drive_on(&mut ui, &screen(false), &[(600.0, 328.0, None), (601.0, 328.0, None)]);
        assert!(ui.shown, "the pointer on the bar brings the controls up");
        drive_on(&mut ui, &screen(true), &[(601.0, 328.0, None), (640.0, 300.0, None)]);
        assert!(!ui.shown, "they went as the display was filled");
        std::thread::sleep(Duration::from_millis(700));
        drive_on(&mut ui, &screen(true), &[(650.0, 300.0, None)]);
        assert!(ui.shown, "and a move of the pointer a moment later brings them back");
        let mut ui = Ui::new((900, 360), 1.0, false, None, None);
        ui.set_pinned(true);
        drive_on(&mut ui, &screen(false), &[(600.0, 328.0, None)]);
        drive_on(&mut ui, &screen(true), &[(601.0, 328.0, None)]);
        assert!(ui.shown, "pinned, they stay");
    }

    /// a plain click on the picture plays or pauses (the owner's word, in place of the design's show and hide). of
    /// a double click the first does too, and the second takes it back and fills the display. a drag does not, nor
    /// does the click that only closes the right-click menu.
    #[test]
    fn a_plain_click_on_the_picture_plays_or_pauses() {
        use crate::event::Button::{Left, Right};
        let mut ui = Ui::new((900, 360), 1.0, false, None, None);
        assert_eq!(drive(&mut ui, &click(450.0, 180.0)), vec![Action::TogglePlay]);
        std::thread::sleep(Duration::from_millis(350));
        let twice = [(450.0, 180.0, None), (450.0, 180.0, Some((Left, true))), (450.0, 180.0, Some((Left, false))), (450.0, 180.0, Some((Left, true))), (450.0, 180.0, Some((Left, false))), (450.0, 180.0, None)];
        assert_eq!(drive(&mut ui, &twice), vec![Action::TogglePlay, Action::Fullscreen, Action::PlayAsBefore]);
        std::thread::sleep(Duration::from_millis(350));
        let drag = [(450.0, 180.0, None), (450.0, 180.0, Some((Left, true))), (480.0, 200.0, None), (480.0, 200.0, Some((Left, false))), (480.0, 200.0, None)];
        assert_eq!(drive(&mut ui, &drag), vec![Action::MoveWindow]);
        std::thread::sleep(Duration::from_millis(350));
        let menu = [(450.0, 180.0, None), (450.0, 180.0, Some((Right, true))), (450.0, 180.0, Some((Right, false))), (450.0, 180.0, None)];
        assert_eq!(drive(&mut ui, &menu), vec![]);
        assert!(ui.commands.is_some(), "the right-click menu is up");
        assert_eq!(drive(&mut ui, &click(60.0, 300.0)), vec![], "the click that closes the menu");
        assert!(ui.commands.is_none());
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(drive(&mut ui, &click(60.0, 300.0)), vec![Action::TogglePlay]);
    }

    /// on a mac the window's buttons are the system's own, at the strip's left: the strip keeps their room, has none
    /// of its own there or at the right, and the tabs start after the room. "clip" is 4 of the type's 6.9-wide
    /// letters in a tab 40 wider, with its close mark 14 from the tab's end. the mark is the last thing at the right,
    /// where the window's close button is elsewhere.
    #[test]
    fn where_the_system_draws_the_windows_buttons_the_strip_keeps_their_room() {
        let mut ui = Ui::new((900, 360), 1.0, false, None, None);
        ui.set_buttons_room(Some(78.0));
        let close_mark = 27.6 + 40.0 - 14.0;
        for (x, asked) in [(20.0, vec![]), (60.0, vec![]), (78.0 + close_mark, vec![Action::CloseTab(0)]), (780.0, vec![]), (884.0, vec![])] {
            assert_eq!(drive(&mut ui, &click(x, 16.0)), asked, "a click at {x}");
            std::thread::sleep(Duration::from_millis(350));
        }
        assert!(ui.panel.as_ref().is_some_and(|panel| panel.which == panel::Which::Settings), "the mark at the far right opened about");
        // elsewhere they are the strip's, at the right, close last, and the tabs start at the left edge
        let mut ui = Ui::new((900, 360), 1.0, false, None, None);
        for (x, asked) in [(880.0, vec![Action::Close]), (840.0, vec![Action::ToggleMaximize]), (800.0, vec![Action::Minimize]), (close_mark, vec![Action::CloseTab(0)])] {
            assert_eq!(drive(&mut ui, &click(x, 16.0)), asked, "a click at {x}");
            std::thread::sleep(Duration::from_millis(350));
        }
    }

    /// the design's fig. 14: a folder's tab has arrows beside its count that do what control with an arrow does, on
    /// that tab, whether its clip opened or not. a click elsewhere on the tab that is up still shows the list. here
    /// "browse /" is 8 of the type's 6.9-wide letters from 12, each arrow 16.9 wide with its room, the count 41.4
    #[test]
    fn a_folders_tab_steps_through_it_with_the_arrows_beside_its_count() {
        let (previous, count, next) = (12.0 + 55.2 + 8.45, 12.0 + 55.2 + 16.9 + 20.7, 12.0 + 55.2 + 16.9 + 41.4 + 8.45);
        let place = |at| Some(Place { folder: "browse", at, count: 4, clip: "hap_64.mov" });
        let list = || FolderList { title: "browse", at: 0, items: Vec::new() };
        let screen = |clip, error| Screen { tabs: vec!["browse / 1 of 4 · hap_64.mov", "browse / 2 of 4 · prores_64.mov"], current: 0, places: vec![place(0), place(1)], clip, error, opening: None, list: Some(list()), browsing: false, fullscreen: false, toast: None, settings: &DEFAULTS };
        let mut ui = Ui::new((900, 360), 1.0, false, None, None);
        let up = screen(Some(model()), None);
        assert_eq!(drive_on(&mut ui, &up, &click(next, 16.0)), vec![Action::Key(Key::Right, keys::CTRL)]);
        assert!(ui.menu.is_none(), "an arrow does not open the list");
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(drive_on(&mut ui, &up, &click(previous, 16.0)), vec![Action::Key(Key::Left, keys::CTRL)]);
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(drive_on(&mut ui, &up, &click(count, 16.0)), vec![]);
        assert!(ui.menu == Some(Menu::List), "the count shows the list");
        // the other folder's tab, from the end of this one (its name is 10 letters, then the tab's 40): its arrow
        // brings it up and steps it
        let other = 12.0 + 55.2 + 2.0 * 16.9 + 41.4 + 6.9 + 69.0 + 28.0;
        std::thread::sleep(Duration::from_millis(350));
        assert_eq!(drive_on(&mut ui, &up, &click(other + next, 16.0)), vec![Action::SelectTab(1), Action::Key(Key::Right, keys::CTRL)]);
        // a clip of the folder that did not open: its tab steps on all the same
        let mut ui = Ui::new((900, 360), 1.0, false, None, None);
        assert_eq!(drive_on(&mut ui, &screen(None, Some("it does not open")), &click(next, 16.0)), vec![Action::Key(Key::Right, keys::CTRL)]);
        // and the type has the arrows: a sign it lacks is drawn as an empty box
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for family in [MONO, MONO_MEDIUM] {
                assert!(ui.ctx().fonts_mut(|f| f.has_glyphs(&font(family, 11.5), "‹›")), "{family} has no arrows");
            }
        });
        out.textures_delta.clear();
    }

    /// the design's hybrid (fig. 04, plan section 06): in the pass the pointer comes onto the timebar, the picture of
    /// the strip nearest the frame under it shows; the picture read for that frame takes its place once it comes, and
    /// stays the nearest had. the timebar is the bottom 13 of the window; 180 frames run along its 640
    #[test]
    fn the_timebar_shows_the_nearest_picture_had_at_once() {
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        for frame in [120, 0] {
            ui.timebar_picture(frame, (2, 1), vec![frame as u8; 6], true);
        }
        // (egui finds what is under the pointer by the pass before: two passes)
        let shown = |ui: &mut Ui, x: f32| {
            drive_with(ui, Model { frames: 180, ..model() }, &[(x, 357.0, None), (x, 357.0, None)]);
            (ui.time_hover, ui.time_picture.as_ref().map(|(of, picture)| (*of, picture.size())))
        };
        // frame 60 is as near the strip's 0 as its 120: the earlier
        assert_eq!(shown(&mut ui, 640.0 / 3.0), (Some(60), Some((0, [2, 1]))));
        ui.timebar_picture(60, (4, 2), vec![60; 24], false);
        assert_eq!(shown(&mut ui, 640.0 / 3.0), (Some(60), Some((60, [4, 2]))));
        assert_eq!(shown(&mut ui, 220.0), (Some(62), Some((60, [4, 2]))));
        assert_eq!(shown(&mut ui, 600.0), (Some(168), Some((120, [2, 1]))));
        // another clip: nothing is had of it
        ui.forget_timebar_picture();
        assert_eq!(shown(&mut ui, 600.0), (Some(168), None));
    }

    /// a right click and the passes after it, at one place
    fn right_click(x: f32, y: f32) -> [(f32, f32, Option<(crate::event::Button, bool)>); 5] {
        use crate::event::Button::Right;
        [(x, y, None), (x, y, Some((Right, true))), (x, y, Some((Right, false))), (x, y, None), (x, y, None)]
    }

    /// a right click on the picture opens every command where it was made; a click on a row sends that row's key and
    /// does nothing else (the window neither moves nor fills the display), and the menu closes. a second click at once
    /// on the picture under it is no double click.
    #[test]
    fn a_row_of_the_right_click_menu_sends_its_key_and_nothing_else() {
        use crate::event::Button::Left;
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        // the menu is laid out once unseen, then stands 8 in from the top, too tall for the window: under its field
        // (26, and 10 after it), the left column's third row (next frame) is 125 to 149 down
        let mut actions = drive(&mut ui, &right_click(100.0, 60.0));
        assert!(ui.commands.is_some(), "the right click opened the menu");
        // long enough for the right click not to count with the clicks after it
        std::thread::sleep(Duration::from_millis(350));
        actions.extend(drive(&mut ui, &[(200.0, 137.0, None), (200.0, 137.0, Some((Left, true))), (200.0, 137.0, Some((Left, false)))]));
        assert_eq!(actions, vec![Action::Key(Key::G, keys::NONE)]);
        assert!(ui.commands.is_none(), "the menu closes once a row is chosen");
        let again = drive(&mut ui, &[(200.0, 137.0, None), (200.0, 137.0, Some((Left, true))), (200.0, 137.0, Some((Left, false)))]);
        assert_eq!(again, vec![]);
    }

    /// the right-click menu's field has the keys as it opens: what is typed finds the commands that say it, and enter
    /// runs the first of them.
    #[test]
    fn typing_in_the_right_click_menu_finds_a_command_and_enter_runs_it() {
        let mut ui = Ui::new((900, 700), 1.0, false, None, None);
        drive(&mut ui, &right_click(100.0, 60.0));
        assert!(ui.typing(), "the field takes the keys at once");
        for c in "mirror".chars() {
            ui.char(c);
        }
        drive(&mut ui, &[(5.0, 5.0, None)]);
        ui.key(Key::Enter, keys::NONE);
        assert_eq!(drive(&mut ui, &[(5.0, 5.0, None)]), vec![Action::Key(Key::H, keys::NONE)]);
        assert!(ui.commands.is_none());
    }

    /// the right-click menu stands where the right click was made when the window has room for it, and inside the
    /// window when it has not.
    #[test]
    fn the_right_click_menu_stands_at_the_pointer_and_inside_the_window() {
        let mut ui = Ui::new((900, 1600), 1.0, false, None, None);
        drive(&mut ui, &right_click(100.0, 60.0));
        assert_eq!(ui.commands_rect.min, Pos2::new(100.0, 60.0));
        assert!(ui.commands_rect.height() < 1600.0 - 16.0, "the whole list shows, unscrolled: {:?}", ui.commands_rect);
        drive(&mut ui, &right_click(850.0, 1500.0));
        let room = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 1600.0)).shrink(8.0);
        assert!(ui.commands.is_some() && room.contains_rect(ui.commands_rect), "{:?} is not inside {room:?}", ui.commands_rect);
    }

    /// a press outside a panel closes it and does nothing else; a press in it does not close it; the next press
    /// does what it does. in the least window, 640 x 360, the panel is the window less 16 each side, under the strip:
    /// 16 to 624 across and 48 to 344 down.
    #[test]
    fn a_press_outside_a_panel_closes_it_and_does_nothing_else() {
        use crate::event::Button::Left;
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        // the timebar, the window's bottom 14, which the panel leaves clear: the middle of its three frames
        let next = (320.0, 354.0);
        drive(&mut ui, &[(next.0, next.1, None)]);
        ui.show_help();
        assert_eq!(drive(&mut ui, &click(300.0, 200.0)), vec![], "a press inside the panel");
        assert!(ui.panel_up(), "a press inside the panel leaves it up");
        assert_eq!(ui.panel.as_ref().unwrap().rect, Rect::from_min_max(Pos2::new(16.0, 48.0), Pos2::new(624.0, 344.0)));
        std::thread::sleep(Duration::from_millis(350));
        let actions = drive(&mut ui, &[(next.0, next.1, None), (next.0, next.1, None), (next.0, next.1, Some((Left, true))), (next.0, next.1, Some((Left, false)))]);
        assert_eq!(actions, vec![], "the press on the timebar only closed the panel");
        assert!(!ui.panel_up());
        // the bar stepped aside for the panel: the pointer moves, and it comes back
        std::thread::sleep(Duration::from_millis(350));
        // (the timebar goes there as the button goes down, and again as it comes up: the clip here never moves)
        let actions = drive(&mut ui, &[(next.0 + 1.0, next.1, None), (next.0, next.1, None), (next.0, next.1, Some((Left, true))), (next.0, next.1, Some((Left, false)))]);
        assert!(!actions.is_empty() && actions.iter().all(|a| *a == Action::Seek(1)), "{actions:?}");
        // nor does a double click on the picture whose first press closed the panel fill the display
        ui.show_help();
        std::thread::sleep(Duration::from_millis(350));
        let beside = (8.0, 180.0, None);
        let actions = drive(&mut ui, &[beside, beside, (beside.0, beside.1, Some((Left, true))), (beside.0, beside.1, Some((Left, false))), (beside.0, beside.1, Some((Left, true))), (beside.0, beside.1, Some((Left, false)))]);
        assert_eq!(actions, vec![]);
    }

    /// a panel keeps the strip up, and says so: in fullscreen the pointer then shows, to press the panel's controls,
    /// and on a mac the system's window buttons stay with the strip.
    #[test]
    fn a_panel_in_fullscreen_keeps_the_pointer_and_the_strip() {
        let mut ui = Ui::new((1088, 612), 1.0, false, None, None);
        let screen = Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(model()), error: None, opening: None, list: None, browsing: false, fullscreen: true, toast: None, settings: &DEFAULTS };
        ui.toggle_settings();
        let mut out = ui.run(&screen);
        out.textures.clear();
        assert!(out.shown, "the strip is up under a panel");
    }

    /// the keys of a panel (figs. 33, 39): esc closes either. help keeps the table of keys' rule: ? closes it, and any
    /// other key closes it and goes on to the player. settings goes through its pages with up and down, closes on
    /// control + , and holds the player's other keys.
    #[test]
    fn a_panels_keys() {
        let mut ui = Ui::new((1088, 612), 1.0, false, None, None);
        let page = |ui: &Ui| ui.panel.as_ref().map(|panel| (panel.which, panel.page));
        ui.show_help();
        assert_eq!(page(&ui), Some((panel::Which::Help, 0)), "? opens help on its keys page");
        assert!(ui.panel_key(Key::Slash, keys::SHIFT) && !ui.panel_up(), "? closes it");
        ui.show_help();
        assert!(!ui.panel_key(Key::G, keys::NONE) && !ui.panel_up(), "G closes it, then steps");
        ui.show_help();
        assert!(ui.panel_key(Key::Escape, keys::NONE) && !ui.panel_up());
        ui.toggle_settings();
        assert_eq!(page(&ui), Some((panel::Which::Settings, 0)), "control + , opens settings on general");
        assert!(ui.panel_key(Key::Down, keys::NONE) && ui.panel_key(Key::Down, keys::NONE));
        assert_eq!(page(&ui), Some((panel::Which::Settings, 2)));
        assert!(ui.panel_key(Key::Up, keys::NONE) && ui.panel_key(Key::G, keys::NONE) && ui.panel_key(Key::Space, keys::NONE));
        assert_eq!(page(&ui), Some((panel::Which::Settings, 1)), "the player's keys wait");
        assert!(ui.panel_key(Key::Comma, keys::CTRL) && !ui.panel_up(), "control + , closes it");
        ui.toggle_settings();
        ui.toggle_settings();
        assert!(!ui.panel_up());
        assert!(!ui.panel_key(Key::G, keys::NONE), "with no panel up a key is the player's");
    }

    /// the strip's right end (fig. 32), on windows just left of minimize: help, settings and the mark, 32 wide each.
    /// a press on ? or the gear opens its panel, lit while it is up; a press on the mark opens settings on about. in
    /// the panel a press on its pages goes to them, and a switch is a change of the settings, asked for at once.
    #[test]
    fn the_strips_buttons_open_the_panels_and_a_switch_changes_a_setting() {
        let mut ui = Ui::new((1088, 612), 1.0, false, None, None);
        let page = |ui: &Ui| ui.panel.as_ref().map(|panel| (panel.which, panel.page));
        // from the right: 120 of window buttons, then the mark, the gear and ?
        let (help, gear, mark) = (1088.0 - 120.0 - 96.0 + 16.0, 1088.0 - 120.0 - 64.0 + 16.0, 1088.0 - 120.0 - 32.0 + 16.0);
        assert_eq!(drive(&mut ui, &click(gear, 16.0)), vec![]);
        assert_eq!(page(&ui), Some((panel::Which::Settings, 0)));
        // the panel, 760 x 500 in the middle: its sidebar's pages from 87.5 down, 29 apart
        assert_eq!(ui.panel.as_ref().unwrap().rect, Rect::from_min_size(Pos2::new(164.0, 56.0), Vec2::new(760.0, 500.0)));
        std::thread::sleep(Duration::from_millis(350));
        drive(&mut ui, &click(220.0, 87.5 + 29.0 * 2.0 + 14.0));
        assert_eq!(page(&ui), Some((panel::Which::Settings, 2)), "a press on playback");
        std::thread::sleep(Duration::from_millis(350));
        drive(&mut ui, &click(220.0, 87.5 + 14.0));
        // general's first row, 102 down: a label and a note, 48.65 high, its switch at the page's right, 20 in
        std::thread::sleep(Duration::from_millis(350));
        let asked = drive(&mut ui, &click(924.0 - 20.0 - 13.0, 102.0 + 24.3));
        assert_eq!(asked, vec![Action::Settings(Box::new(Settings { play_on_open: false, ..Settings::default() }))]);
        // a press on the strip is a press outside: it closes the panel, and the next on ? opens help
        std::thread::sleep(Duration::from_millis(350));
        drive(&mut ui, &click(help, 16.0));
        assert!(!ui.panel_up());
        std::thread::sleep(Duration::from_millis(350));
        drive(&mut ui, &click(help, 16.0));
        assert_eq!(page(&ui), Some((panel::Which::Help, 0)));
        ui.panel = None;
        std::thread::sleep(Duration::from_millis(350));
        drive(&mut ui, &click(mark, 16.0));
        assert_eq!(page(&ui), Some((panel::Which::Settings, panel::ABOUT)));
    }

    /// resting on the mark brings its card up, under the strip with its right edge on the mark's; it stays while the
    /// pointer is on the mark or the card, and goes a moment after it leaves both.
    #[test]
    fn the_marks_card_stays_while_the_pointer_is_on_it() {
        let mut ui = Ui::new((1088, 612), 1.0, false, None, None);
        let mark = (1088.0 - 120.0 - 16.0, 16.0);
        drive(&mut ui, &[(mark.0, mark.1, None), (mark.0 + 1.0, mark.1, None)]);
        assert!(ui.card.is_none(), "not before the pointer rests");
        std::thread::sleep(Duration::from_millis(550));
        drive(&mut ui, &[(mark.0, mark.1, None), (mark.0, mark.1, None)]);
        assert!(ui.card.is_some(), "resting on the mark brings the card up");
        assert_eq!((ui.card_rect.right(), ui.card_rect.top()), (1088.0 - 120.0, STRIP + 6.0));
        // across the strip's foot and onto the card: it stays
        let on = ui.card_rect.center();
        drive(&mut ui, &[(mark.0, 36.0, None), (on.x, on.y, None)]);
        std::thread::sleep(Duration::from_millis(300));
        drive(&mut ui, &[(on.x, on.y, None), (on.x + 1.0, on.y, None)]);
        assert!(ui.card.is_some(), "the pointer on the card keeps it up");
        drive(&mut ui, &[(300.0, 300.0, None)]);
        std::thread::sleep(Duration::from_millis(250));
        drive(&mut ui, &[(301.0, 300.0, None), (300.0, 300.0, None)]);
        assert!(ui.card.is_none(), "it goes once the pointer has left both");
    }

    #[test]
    fn the_bars_controls_are_centered_between_its_top_line_and_the_timebar() {
        let bar = Rect::from_min_size(Pos2::new(0.0, 309.0), Vec2::new(640.0, 1.0 + ROW + TIME));
        let middle = row_middle(bar);
        assert_eq!(middle - (bar.top() + 1.0), (bar.bottom() - TRACK_THIN) - middle);
    }
}
