//! the controls: drawn by us, to the colors, type and measurements of the approved design (docs/design/plan.html).
//! platform-neutral: the window reports the pointer and the keys, this returns triangles for the renderer and what
//! was asked for. one unit here is one of the design's pixels; the display's scale turns it into physical pixels.

mod browser;
mod icons;
mod keys;
mod menus;
mod widgets;

use crate::event::{Key, Mods};
use crate::media::meta::Meta;
use crate::settings::Loop;
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
/// the controls fade in and out over this long, and go away this long after the pointer stops
const FADE: f32 = 0.14;
const IDLE: f64 = 2.4;
/// the zoom chip goes this long after the pointer rests
const CHIP_IDLE: f64 = 2.0;
/// the metadata panel comes up this long after the pointer comes onto its button, and goes this long after it has
/// left both the button and the panel
const FACTS_IN: f64 = 0.35;
const FACTS_OUT: f64 = 0.18;
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
    /// its clip, when it has one
    pub clip: Option<Model<'a>>,
    /// why its file did not open, when it did not
    pub error: Option<&'a str>,
    /// the browser is up: in a tab with nothing in it, or laid over the clip
    pub browsing: bool,
    pub fullscreen: bool,
    /// what just happened, said low in the middle of the window for a moment
    pub toast: Option<&'a str>,
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

/// the clip on screen, as the controls show it.
pub struct Model<'a> {
    pub frame: usize,
    pub frames: usize,
    pub playing: bool,
    /// frames a second as the file's own ratio, when it has one rate
    pub rate: Option<(i64, i64)>,
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
    /// the clip has sound; how loud the player is set, 0 to 1; and whether it is muted
    pub sound: bool,
    pub volume: f32,
    pub muted: bool,
    /// where screenshots go, as the menu names it
    pub shots_to: &'a str,
}

/// what the user asked for through the controls.
#[derive(Debug, PartialEq)]
pub enum Action {
    TogglePlay,
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
    /// open this file in the tab that is up
    Open(PathBuf),
    /// play this folder: asked for in the browser
    PlayFolder(PathBuf),
    /// lay the browser over the clip, or take it away
    Browse(bool),
    Shot(Shot),
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
    /// the strip and the bar are up
    pub shown: bool,
    /// the frame under the pointer on the timebar: its picture is wanted
    pub hovered: Option<usize>,
}

/// the menus that open over the bar
#[derive(Clone, Copy, PartialEq)]
enum Menu {
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
    /// the controls are wanted on screen, and until when with the pointer at rest
    shown: bool,
    awake_until: f64,
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
    /// the frame under the pointer on the timebar, and the picture last had for the timebar with the frame it is of
    time_hover: Option<usize>,
    time_picture: Option<(usize, egui::TextureHandle)>,
    /// the table of keys is up; the right-click menu is open, from where it was asked for, and was drawn there last
    keys: bool,
    commands: Option<Pos2>,
    commands_rect: Rect,
    /// whether each of the last two presses went to the right-click menu or the table of keys, the earlier first
    taken: [bool; 2],
    /// the button the pointer is on in this pass, where it is and what its tooltip says; and the one it came onto,
    /// and when
    hinted: Option<(egui::Id, Rect, (String, String))>,
    tip: Option<(egui::Id, f64)>,
    browser: browser::Browser,
}

impl Ui {
    /// `folder` is where the browser starts: the one it was last in.
    pub fn new(size: (u32, u32), scale: f32, timecode: bool, folder: Option<PathBuf>, pictures: Option<Pictures>) -> Ui {
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts());
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
            shown: false,
            awake_until: 0.0,
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
            time_hover: None,
            time_picture: None,
            keys: false,
            commands: None,
            commands_rect: Rect::NOTHING,
            taken: [false; 2],
            hinted: None,
            tip: None,
            browser: browser::Browser::new(folder, pictures),
        }
    }

    /// a key, a typed character and the wheel for the browser, while it is up.
    pub fn browser_key(&mut self, key: Key, mods: Mods) {
        self.browser.key(key, mods);
    }

    pub fn browser_char(&mut self, c: char) {
        self.browser.char(c);
    }

    pub fn browser_wheel(&mut self, notches: i32) {
        self.browser.wheel(notches);
    }

    /// the folder the browser is in: kept between runs.
    pub fn browser_folder(&self) -> &Path {
        self.browser.folder()
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

    pub fn pointer_button(&mut self, button: crate::event::Button, down: bool, x: f32, y: f32) {
        let button = match button {
            crate::event::Button::Left => egui::PointerButton::Primary,
            crate::event::Button::Right => egui::PointerButton::Secondary,
            crate::event::Button::Middle => egui::PointerButton::Middle,
        };
        self.events.push(egui::Event::PointerButton { pos: self.point(x, y), button, pressed: down, modifiers: egui::Modifiers::NONE });
    }

    pub fn pointer_leave(&mut self) {
        self.events.push(egui::Event::PointerGone);
        self.inside = false;
    }

    /// true while a field in a menu is being typed into: keys then go there, not to the player.
    pub fn typing(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// a key for the field being typed into.
    pub fn key(&mut self, key: Key, mods: Mods) {
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
        let over = self.ctx.pointer_latest_pos().is_some_and(|p| ((self.facts_pinned || self.facts_up) && self.facts_rect.contains(p)) || (self.commands.is_some() && self.commands_rect.contains(p)));
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

    /// escape: closes the open menu. false when there was nothing of ours to close.
    pub fn escape(&mut self) -> bool {
        self.menu.take().is_some() | self.commands.take().is_some()
    }

    /// the table of keys comes up over the clip, in place of any menu.
    pub fn show_keys(&mut self) {
        (self.keys, self.menu, self.commands) = (true, None, None);
    }

    /// takes the table of keys away. false when it was not up.
    pub fn hide_keys(&mut self) -> bool {
        std::mem::take(&mut self.keys)
    }

    /// the view changed by key or wheel: the zoom chip shows again.
    pub fn view_changed(&mut self) {
        self.view_changed = true;
    }

    /// a picture of a frame of the clip on screen, for the timebar: red, green and blue, a byte each.
    pub fn timebar_picture(&mut self, frame: usize, (width, height): (u32, u32), pixels: &[u8]) {
        let image = egui::ColorImage::from_rgb([width as usize, height as usize], pixels);
        self.time_picture = Some((frame, self.ctx.load_texture("timebar picture", image, egui::TextureOptions::LINEAR)));
    }

    /// the clip on screen is another one now: the timebar's picture is not of it.
    pub fn forget_timebar_picture(&mut self) {
        self.time_picture = None;
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
        self.moved = false;
        Output { actions, primitives: ctx.tessellate(out.shapes, out.pixels_per_point), textures: out.textures_delta, scale: out.pixels_per_point, again, shown: self.shown, hovered: self.time_hover }
    }

    fn draw(&mut self, ui: &mut egui::Ui, screen: &Screen, now: f64, actions: &mut Vec<Action>) {
        let full = ui.max_rect();
        let strip = Rect::from_min_size(full.min, Vec2::new(full.width(), STRIP));
        match &screen.clip {
            Some(model) if !screen.browsing => {
                self.browser.away();
                self.clip(ui, screen, model, now, actions)
            }
            // no clip to look at: the strip stays up, and under it the browser, or why the file did not open
            _ => {
                (self.menu, self.shown, self.keys, self.commands) = (None, true, false, None);
                let body = Rect::from_min_max(strip.left_bottom(), full.max);
                match screen.error.filter(|_| !screen.browsing) {
                    Some(why) => {
                        self.browser.away();
                        message(ui, body, screen.tabs[screen.current], why)
                    }
                    None => match self.browser.draw(ui, body) {
                        Some(browser::Chosen::Open(path)) => actions.push(Action::Open(path)),
                        Some(browser::Chosen::PlayFolder(folder)) => actions.push(Action::PlayFolder(folder)),
                        Some(browser::Chosen::Back) => actions.push(Action::Browse(false)),
                        None => {}
                    },
                }
                self.strip(ui, strip, screen, true, actions);
                // tooltips are the clip's
                (self.hinted, self.tip) = (None, None);
            }
        }
        if let Some(said) = screen.toast {
            toast(ui, full, said);
        }
    }

    /// a clip is up: the picture takes the pointer, and the controls come and go over it.
    fn clip(&mut self, ui: &mut egui::Ui, screen: &Screen, model: &Model, now: f64, actions: &mut Vec<Action>) {
        let full = ui.max_rect();
        // the picture, under everything else. a plain click on it shows or hides the controls. a drag moves the
        // window while the whole frame is in view, and the picture when it is larger than the window. a double click
        // fills the display with it, or puts the window back (the owner's word, in place of the design's fit and
        // last zoom)
        let picture = ui.interact(full, ui.scope_id().with("picture"), Sense::click_and_drag());
        if model.overflows {
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
        // right-click menu or took the table of keys away is not one on the picture
        if pressed {
            self.taken = [self.taken[1], self.keys || (self.commands.is_some() && pointer.is_some_and(|p| self.commands_rect.contains(p)))];
        }
        if picture.double_clicked() && !self.taken[0] {
            actions.push(Action::Fullscreen);
        }
        let strip = Rect::from_min_size(full.min, Vec2::new(full.width(), STRIP));
        let bar = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - (1.0 + ROW + TIME)), full.max);
        let over = self.inside && pointer.is_some_and(|p| bar.contains(p) || strip.contains(p) || (self.menu.is_some() && self.menu_rect.contains(p)) || (self.facts_up && self.facts_rect.contains(p)));
        let held = ui.input(|i| i.pointer.any_down());

        // a press anywhere but on the open menu or its button closes the menu
        if self.menu.is_some() && pressed && !pointer.is_some_and(|p| self.menu_rect.contains(p) || self.menu_button.contains(p)) {
            self.menu = None;
        }
        // the right-click menu closes the same way, and a right click on the picture opens it there. a press anywhere
        // takes the table of keys away, and does nothing else (the table takes it)
        if self.commands.is_some() && pressed && !pointer.is_some_and(|p| self.commands_rect.contains(p)) {
            self.commands = None;
        }
        let opened = picture.secondary_clicked();
        if opened {
            self.commands = pointer;
        }
        self.keys &= !pressed;

        // the design's rule: moving the pointer brings the controls up; they go when it rests for a while off them,
        // or leaves the window; a plain click on the picture shows or hides them at once. an open menu keeps them up
        if self.moved {
            (self.shown, self.awake_until) = (true, now + IDLE);
        }
        if self.moved || std::mem::take(&mut self.view_changed) {
            self.chip_until = now + CHIP_IDLE;
        }
        if picture.clicked() {
            self.shown = !self.shown || self.moved;
            self.awake_until = now + IDLE;
        }
        if self.menu.is_some() || self.commands.is_some() {
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
        // the table of keys takes the window, as the design draws it: the strip and the bar step aside while it is up,
        // and the chips stay
        self.shown &= !self.keys;
        let alpha = ui.ctx().animate_bool_with_time(ui.scope_id().with("chrome"), self.shown, FADE);

        self.guides(ui, model);
        // the table of keys: over the picture, and under the chips, which still say what is not as the file has it
        if self.keys {
            keys::table(ui, full);
        }
        // what states the view sits under the strip when it is up, as in the design: a fault across the top, then
        // loop analysis, speed and the look from the left, and the zoom at the right
        let mut top = full.top() + 10.0 + STRIP * alpha;
        if let Some(why) = model.fault {
            top = banner(ui, Pos2::new(full.left() + 10.0, top), full.width() - 20.0, &format!("this frame cannot be shown: {why}")).bottom() + 8.0;
        }
        let mut left = full.left() + 10.0;
        let linear = model.linear.then(|| vec![("linear".to_string(), FG), (" · shown through srgb".to_string(), DIM)]);
        for parts in [seam_chip(model), speed_chip(model), model.look.label().map(|look| vec![(look, FG)]), linear].into_iter().flatten() {
            left = chip(ui, Pos2::new(left, top), Align2::LEFT_TOP, &parts, 1.0).right() + 8.0;
        }
        self.zoom_chip(ui, Pos2::new(full.right() - 10.0, top), model, now);
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
            let (drawn, chosen) = keys::menu(ui, at, room, self.commands_rect, opened);
            self.commands_rect = drawn;
            if let Some((key, mods)) = chosen {
                actions.push(Action::Key(key, mods));
                self.commands = None;
            }
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
            self.menu_rect = menus::show(ui, menu, corner, alpha, model, actions);
            // what a menu changes about the view is said by the chip
            self.view_changed |= actions.iter().any(|a| matches!(a, Action::Zoom(_)));
        }
        self.tooltip(ui, full, now);
    }

    /// the lines over the picture: the grid of tiles, the center lines and circle. one display pixel wide at 100%.
    fn guides(&self, ui: &egui::Ui, model: &Model) {
        let guides = model.guides;
        if !guides.grid && !guides.center {
            return;
        }
        let (x, y, w, h) = model.picture;
        let (video_w, video_h) = (model.shown_size.0 as f32, model.shown_size.1 as f32);
        let unit = 1.0 / self.scale;
        let width = self.scale.round().max(1.0);
        let color = Color32::from_rgb(guides.color[0], guides.color[1], guides.color[2]);
        let painter = ui.painter().with_clip_rect(Rect::from_min_size(Pos2::new(x * unit, y * unit), Vec2::new(w * unit, h * unit)));
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
        chip(ui, right_top, Align2::RIGHT_TOP, &parts, alpha);
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

        // the window's buttons, at the right: minimize, maximize, close. 10 x 10 marks in 40-wide buttons
        let mut x = strip.right();
        for (name, action) in [("close", Action::Close), ("maximize", Action::ToggleMaximize), ("minimize", Action::Minimize)] {
            x -= WINDOW_BUTTON;
            let rect = Rect::from_min_size(Pos2::new(x, strip.top()), Vec2::new(WINDOW_BUTTON, STRIP - 1.0));
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
        let room = (x - strip.left() - PLUS).max(0.0);
        let wide = |name: &str| (painter.layout_no_wrap(name.to_string(), font(MONO_MEDIUM, 11.5), FG).size().x + TAB_PAD).min(TAB_MAX);
        let mut widths: Vec<f32> = screen.tabs.iter().map(|name| wide(name)).collect();
        if widths.iter().sum::<f32>() > room {
            let each = (room / widths.len() as f32).max(TAB_MIN);
            widths.iter_mut().for_each(|w| *w = w.min(each));
        }
        let clipped = painter.with_clip_rect(Rect::from_min_max(strip.min, Pos2::new(x, strip.bottom())));
        let mut left = strip.left();
        for (i, (name, width)) in screen.tabs.iter().zip(widths).enumerate() {
            let tab = Rect::from_min_size(Pos2::new(left, strip.top()), Vec2::new(width, STRIP - 1.0));
            let up = i == screen.current;
            if ui.interact(tab, ui.scope_id().with(("tab", i)), Sense::click()).clicked() && !up {
                actions.push(Action::SelectTab(i));
            }
            if up {
                clipped.rect_filled(tab, 0.0, TAB_ON);
                let close = Rect::from_center_size(Pos2::new(tab.right() - 8.0 - 6.0, tab.center().y), Vec2::splat(16.0));
                let mark = ui.interact(close, ui.scope_id().with(("tab-close", i)), Sense::click());
                if mark.clicked() {
                    actions.push(Action::CloseTab(i));
                }
                self.hint(&mark, keys::named(Key::W, keys::CTRL));
                icons::draw_sized(&clipped, close.center(), 12.0, Icon::Close, Color32::WHITE.gamma_multiply(0.7));
            }
            clipped.rect_filled(Rect::from_min_size(Pos2::new(tab.right() - 1.0, tab.top()), Vec2::new(1.0, tab.height())), 0.0, LINE);
            let (family, color) = if up { (MONO_MEDIUM, Color32::WHITE) } else { (MONO, TAB_OFF) };
            let galley = cut_middle(&clipped, name, font(family, 11.5), color, width - TAB_PAD);
            clipped.galley(Pos2::new(tab.left() + 12.0, text_top(&clipped, font(family, 11.5), tab.center().y)), galley, color);
            left = tab.right();
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
        let prev = icon_button(ui, place(0.0, ICON_BUTTON.x, ICON_BUTTON.y), "prev", Icon::Prev, Lit::No);
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
        let shot = icon_button(ui, rect, "shot", Icon::Shot, if self.menu == Some(Menu::Shot) { Lit::Open } else { Lit::No });
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
    /// while a menu, the metadata panel or the table of keys is up, or a button is held.
    fn tooltip(&mut self, ui: &egui::Ui, full: Rect, now: f64) {
        let quiet = self.menu.is_some() || self.commands.is_some() || self.keys || self.facts_up || ui.input(|i| i.pointer.any_down());
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
        chip(ui, corner, anchor, &[(does, FG), (format!(" · {key}"), DIM)], 1.0);
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
        }
        painter.rect_filled(Rect::from_min_max(Pos2::new(head - 1.0, track.top() - 4.0), Pos2::new(head + 1.0, track.bottom())), 0.0, Color32::WHITE);

        // a drag follows the pointer; a click goes there too, even one that came and went within a single pass
        if let Some(p) = response.interact_pointer_pos().filter(|_| scrubbing || response.clicked()) {
            let to = stop_at(p.x);
            if to != at {
                actions.push(if model.seam.is_some() { Action::SeekPlace(to) } else { Action::Seek(to) });
            }
        }
        // the frame under the pointer: its number, and over it its picture once the spare reader has read one (the
        // one it read last stays up meanwhile). the design's box: the picture 160 by 90 inside a margin of 2
        self.time_hover = response.hover_pos().filter(|_| model.seam.is_none()).map(|p| stop_at(p.x));
        if let (Some(p), Some(frame)) = (response.hover_pos(), self.time_hover) {
            let galley = painter.layout_no_wrap(format!("{frame:06}"), font(MONO_MEDIUM, 10.5), FG);
            let size = match &self.time_picture {
                Some(_) => Vec2::new(164.0, 2.0 + 90.0 + galley.size().y + 10.0),
                None => galley.size() + Vec2::new(16.0, 10.0),
            };
            let left = (p.x - size.x / 2.0).clamp(area.left() + 4.0, area.right() - size.x - 4.0);
            let rect = Rect::from_min_size(Pos2::new(left, over - 7.0 - size.y), size);
            painter.rect_filled(rect, 0.0, SOLID);
            painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
            let mut number = rect;
            if let Some((_, picture)) = &self.time_picture {
                let place = Rect::from_min_size(rect.min + Vec2::splat(2.0), Vec2::new(160.0, 90.0));
                painter.rect_filled(place, 0.0, Color32::BLACK);
                let fitted = Rect::from_center_size(place.center(), picture.size_vec2() * (place.width() / picture.size_vec2().x).min(place.height() / picture.size_vec2().y));
                painter.image(picture.id(), fitted, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                number = Rect::from_min_max(Pos2::new(rect.left(), place.bottom()), rect.max);
            }
            painter.galley(Pos2::new(number.center().x - galley.size().x / 2.0, text_top(&painter, font(MONO_MEDIUM, 10.5), number.center().y)), galley, FG);
        }
    }
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
    job.append(done, 0.0, TextFormat { font_id: font(MONO_MEDIUM, 11.5), color: FG, ..Default::default() });
    if !rest.is_empty() {
        job.append(&format!(" · {rest}"), 0.0, TextFormat { font_id: font(MONO_MEDIUM, 11.5), color: DIM, ..Default::default() });
    }
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("toast")));
    let galley = painter.layout_job(job);
    let rect = Align2::CENTER_BOTTOM.anchor_size(Pos2::new(full.center().x, full.bottom() - 64.0), galley.size() + Vec2::new(24.0, 18.0));
    painter.rect_filled(rect, 0.0, SOLID);
    painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
    painter.galley(Pos2::new(rect.left() + 12.0, text_top(&painter, font(MONO_MEDIUM, 11.5), rect.center().y)), galley, FG);
}

/// a tab whose file did not open: its name and why, in the middle.
fn message(ui: &egui::Ui, area: Rect, title: &str, why: &str) {
    let painter = ui.painter().with_clip_rect(area);
    painter.rect_filled(area, 0.0, MESSAGE);
    let width = (area.width() - 40.0).min(460.0);
    let title = painter.layout(title.to_string(), font("sans-semibold", 15.0), FG, width);
    let why = painter.layout(why.to_string(), font(MONO, 12.0), DIM, width);
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

/// a chip: a few words in a dark box, placed by one of its corners. returns the box.
fn chip(ui: &egui::Ui, corner: Pos2, anchor: Align2, parts: &[(String, Color32)], alpha: f32) -> Rect {
    let mut job = LayoutJob::default();
    for (text, color) in parts {
        job.append(text, 0.0, TextFormat { font_id: font(MONO_MEDIUM, 11.0), color: color.gamma_multiply(alpha), ..Default::default() });
    }
    let painter = ui.painter();
    let galley = painter.layout_job(job);
    let rect = anchor.anchor_size(corner, galley.size() + Vec2::new(16.0, 12.0));
    painter.rect_filled(rect, 0.0, CHIP.gamma_multiply(alpha));
    painter.rect_stroke(rect, 0.0, (1.0, LINE.gamma_multiply(alpha)), egui::StrokeKind::Inside);
    painter.galley(Pos2::new(rect.left() + 8.0, text_top(painter, font(MONO_MEDIUM, 11.0), rect.center().y)), galley, FG);
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
fn icon_button(ui: &mut egui::Ui, rect: Rect, name: &str, icon: Icon, lit: Lit) -> egui::Response {
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
    let fps = model.rate.map(|(n, d)| (n as f64 / d as f64).round() as usize).filter(|f| *f > 0);
    let (label, at, total) = match fps.filter(|_| timecode) {
        Some(fps) => {
            let tc = |f: usize| format!("{:02}:{:02}:{:02}:{:02}", f / fps / 3600, f / fps / 60 % 60, f / fps % 60, f % fps);
            ("TC", tc(model.frame), (!narrow).then(|| tc(model.frames)))
        }
        None => ("F", format!("{:06}", model.frame), Some(format!("{:06}", model.frames))),
    };
    let text = |size: f32, color: Color32, spacing: f32| TextFormat { font_id: font(MONO_MEDIUM, size), color, extra_letter_spacing: spacing, ..Default::default() };
    let mut job = LayoutJob::default();
    job.append(label, 0.0, text(9.5, DIM, 9.5 * 0.12));
    job.append(&at, 2.0 + 7.0, text(11.5, FG, 11.5 * 0.02));
    if let Some(total) = total {
        job.append(&format!(" / {total}"), 0.0, text(11.5, DIM, 11.5 * 0.02));
    }
    job
}

/// a line of trouble with the amber edge: the one place amber is used, for a fault in the file. returns its box.
fn banner(ui: &egui::Ui, at: Pos2, width: f32, text: &str) -> Rect {
    let painter = ui.painter();
    let galley = painter.layout(text.to_string(), font(MONO, 11.5), DIM, width - 24.0);
    let rect = Rect::from_min_size(at, galley.size() + Vec2::new(22.0, 16.0));
    painter.rect_filled(rect, 0.0, BANNER);
    painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())), 0.0, AMBER);
    painter.galley(rect.min + Vec2::new(12.0, 8.0), galley, DIM);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn model() -> Model<'static> {
        Model {
            frame: 0,
            frames: 3,
            playing: false,
            rate: Some((60, 1)),
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
            sound: false,
            volume: 1.0,
            muted: true,
            shots_to: "pictures / eyepiece",
            folder: false,
            seam_change: None,
        }
    }

    /// drives the controls over that clip with the pointer, a pass for each step, and gathers what they asked for
    fn drive(ui: &mut Ui, steps: &[(f32, f32, Option<(crate::event::Button, bool)>)]) -> Vec<Action> {
        let screen = Screen { tabs: vec!["clip"], current: 0, clip: Some(model()), error: None, browsing: false, fullscreen: false, toast: None };
        let mut actions = Vec::new();
        for &(x, y, button) in steps {
            match button {
                Some((button, down)) => ui.pointer_button(button, down, x, y),
                None => ui.pointer_move(x, y),
            }
            let mut out = ui.run(&screen);
            out.textures.clear();
            actions.extend(out.actions);
        }
        actions
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
        // the menu is laid out once unseen, then stands 8 in from the top, too tall for the window: its third row (next
        // frame) is 89 to 113 down
        let mut actions = drive(&mut ui, &right_click(100.0, 60.0));
        assert!(ui.commands.is_some(), "the right click opened the menu");
        // long enough for the right click not to count with the clicks after it
        std::thread::sleep(Duration::from_millis(350));
        actions.extend(drive(&mut ui, &[(200.0, 101.0, None), (200.0, 101.0, Some((Left, true))), (200.0, 101.0, Some((Left, false)))]));
        assert_eq!(actions, vec![Action::Key(Key::G, keys::NONE)]);
        assert!(ui.commands.is_none(), "the menu closes once a row is chosen");
        let again = drive(&mut ui, &[(200.0, 101.0, None), (200.0, 101.0, Some((Left, true))), (200.0, 101.0, Some((Left, false)))]);
        assert_eq!(again, vec![]);
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

    /// the click that takes the table of keys away does nothing else; the next one does what it does.
    #[test]
    fn the_click_that_takes_the_table_of_keys_away_does_nothing_else() {
        use crate::event::Button::Left;
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        // the bar's next-frame button: 30 wide from 68, 32 up from the bottom
        let next = (83.0, 328.0);
        drive(&mut ui, &[(next.0, next.1, None)]);
        ui.show_keys();
        let actions = drive(&mut ui, &[(next.0, next.1, None), (next.0, next.1, None), (next.0, next.1, Some((Left, true))), (next.0, next.1, Some((Left, false)))]);
        assert_eq!(actions, vec![], "the click on the button only took the table away");
        assert!(!ui.keys);
        // the bar stepped aside for the table: the pointer moves, and it comes back
        let actions = drive(&mut ui, &[(next.0 + 1.0, next.1, None), (next.0, next.1, None), (next.0, next.1, Some((Left, true))), (next.0, next.1, Some((Left, false)))]);
        assert_eq!(actions, vec![Action::Step(1)]);
        // nor does a double click on the picture whose first click took the table away fill the display
        ui.show_keys();
        // long enough for the clicks before not to count with these
        std::thread::sleep(Duration::from_millis(350));
        let mid = (320.0, 180.0, None);
        let actions = drive(&mut ui, &[mid, mid, (mid.0, mid.1, Some((Left, true))), (mid.0, mid.1, Some((Left, false))), (mid.0, mid.1, Some((Left, true))), (mid.0, mid.1, Some((Left, false)))]);
        assert_eq!(actions, vec![]);
    }

    #[test]
    fn the_bars_controls_are_centered_between_its_top_line_and_the_timebar() {
        let bar = Rect::from_min_size(Pos2::new(0.0, 309.0), Vec2::new(640.0, 1.0 + ROW + TIME));
        let middle = row_middle(bar);
        assert_eq!(middle - (bar.top() + 1.0), (bar.bottom() - TRACK_THIN) - middle);
    }
}
