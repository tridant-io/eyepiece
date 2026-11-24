//! the controls: drawn by us, to the colors, type and measurements of the approved design (docs/design/plan.html).
//! platform-neutral: the window reports the pointer and the keys, this returns triangles for the renderer and what
//! was asked for. one unit here is one of the design's pixels; the display's scale turns it into physical pixels.

mod icons;
mod widgets;

use crate::event::{Key, Mods};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontFamily, FontId, Pos2, Rect, Sense, Vec2};
use icons::Icon;
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

// ---- the design's measurements
const STRIP: f32 = 32.0;
const WINDOW_BUTTON: f32 = 40.0;
const TAB_MAX: f32 = 310.0;
const ROW: f32 = 36.0;
const TIME: f32 = 14.0;
const TRACK_THIN: f32 = 4.0;
const TRACK_THICK: f32 = 8.0;
const ICON_BUTTON: Vec2 = Vec2::new(30.0, 28.0);
const TEXT_BUTTON: f32 = 28.0;
const GAP: f32 = 2.0;
const PAD: f32 = 8.0;
const MENU: f32 = 268.0;
/// the controls fade in and out over this long, and go away this long after the pointer stops
const FADE: f32 = 0.14;
const IDLE: f64 = 2.4;
/// the zoom chip goes this long after the pointer rests
const CHIP_IDLE: f64 = 2.0;

const MONO: &str = "mono";
const MONO_MEDIUM: &str = "mono-medium";
const SANS: &str = "sans";

fn font(family: &str, size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(family.into()))
}

/// what the controls show, read from the player each frame.
pub struct Model<'a> {
    /// the file's name, for its tab
    pub name: &'a str,
    pub frame: usize,
    pub frames: usize,
    pub playing: bool,
    /// frames a second as the file's own ratio, when it has one rate
    pub rate: Option<(i64, i64)>,
    /// why the frame on screen cannot be shown, when it cannot
    pub fault: Option<&'a str>,
    /// the zoom as the bar says it ("fit 43.2%", "200%"), as a number (display pixels for one video pixel), whether
    /// one is set rather than fit, and whether it is a whole multiple
    pub zoom_label: String,
    pub zoom: f32,
    pub zoomed: bool,
    pub exact: bool,
    /// the video pixel under the pointer, and the red, green and blue the screen is sent for it, on the scale of the
    /// file's bit depth
    pub pixel: Option<(usize, usize)>,
    pub value: Option<[u32; 3]>,
}

/// what the user asked for through the controls.
#[derive(Debug, PartialEq)]
pub enum Action {
    TogglePlay,
    Step(i64),
    Seek(usize),
    /// fit (None), or this many display pixels for one video pixel, about the middle of the window
    Zoom(Option<f32>),
    /// the double click on the picture, at this place in physical pixels: between fit and the last zoom
    ToggleFit((f32, f32)),
    /// the picture, dragged by this many physical pixels
    Pan((f32, f32)),
    /// the window: move it with the pointer, whose button is down; minimize; maximize or put back; close
    MoveWindow,
    Minimize,
    ToggleMaximize,
    Close,
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
}

/// the menus that open over the bar
#[derive(Clone, Copy, PartialEq)]
enum Menu {
    Zoom,
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
    /// what is typed in the zoom menu's field
    zoom_text: String,
}

impl Ui {
    pub fn new(size: (u32, u32), scale: f32) -> Ui {
        let ctx = egui::Context::default();
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
        ctx.set_fonts(fonts);
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
            timecode: false,
            menu: None,
            menu_rect: Rect::NOTHING,
            menu_button: Rect::NOTHING,
            zoom_text: String::new(),
        }
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

    /// escape: closes the open menu. false when there was nothing of ours to close.
    pub fn escape(&mut self) -> bool {
        self.menu.take().is_some()
    }

    /// the view changed by key or wheel: the zoom chip shows again.
    pub fn view_changed(&mut self) {
        self.view_changed = true;
    }

    /// the readout counts in timecode, or back in frames.
    pub fn toggle_timecode(&mut self) {
        self.timecode = !self.timecode;
    }

    /// one pass: takes what the window reported since the last one, lays out and draws the controls.
    pub fn run(&mut self, model: &Model) -> Output {
        let now = self.started.elapsed().as_secs_f64();
        let mut raw = egui::RawInput { time: Some(now), focused: true, events: std::mem::take(&mut self.events), ..Default::default() };
        raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(self.size.0 / self.scale, self.size.1 / self.scale)));
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(self.scale);
        let mut actions = Vec::new();
        let ctx = self.ctx.clone();
        let out = ctx.run_ui(raw, |ui| self.draw(ui, model, now, &mut actions));
        let again = out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.repaint_delay).filter(|d| *d < Duration::from_secs(3600));
        self.moved = false;
        Output { actions, primitives: ctx.tessellate(out.shapes, out.pixels_per_point), textures: out.textures_delta, scale: out.pixels_per_point, again }
    }

    fn draw(&mut self, ui: &mut egui::Ui, model: &Model, now: f64, actions: &mut Vec<Action>) {
        let full = ui.max_rect();
        // the picture, under everything else. a plain click on it shows or hides the controls. a drag moves the
        // window when the whole frame is in view, and the picture when it is zoomed. a double click goes between
        // fit and the last zoom
        let picture = ui.interact(full, ui.scope_id().with("picture"), Sense::click_and_drag());
        if model.zoomed {
            if picture.dragged_by(egui::PointerButton::Primary) {
                let by = picture.drag_delta() * self.scale;
                if by != Vec2::ZERO {
                    actions.push(Action::Pan((by.x, by.y)));
                }
            }
        } else if picture.drag_started_by(egui::PointerButton::Primary) {
            actions.push(Action::MoveWindow);
        }
        if picture.double_clicked() {
            if let Some(p) = picture.interact_pointer_pos() {
                actions.push(Action::ToggleFit((p.x * self.scale, p.y * self.scale)));
                self.view_changed = true;
            }
        }
        let strip = Rect::from_min_size(full.min, Vec2::new(full.width(), STRIP));
        let bar = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - (1.0 + ROW + TIME)), full.max);
        let pointer = ui.ctx().pointer_latest_pos();
        let over = self.inside && pointer.is_some_and(|p| bar.contains(p) || strip.contains(p) || (self.menu.is_some() && self.menu_rect.contains(p)));
        let held = ui.input(|i| i.pointer.any_down());

        // a press anywhere but on the open menu or its button closes the menu
        if self.menu.is_some() && ui.input(|i| i.pointer.any_pressed()) && !pointer.is_some_and(|p| self.menu_rect.contains(p) || self.menu_button.contains(p)) {
            self.menu = None;
        }

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
        if self.menu.is_some() {
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
        let alpha = ui.ctx().animate_bool_with_time(ui.scope_id().with("chrome"), self.shown, FADE);

        // what states the view sits under the strip when it is up, as in the design
        let top = full.top() + 10.0 + STRIP * alpha;
        if let Some(why) = model.fault {
            banner(ui, Pos2::new(full.left() + 10.0, top), full.width() - 20.0, &format!("this frame cannot be shown: {why}"));
        }
        self.zoom_chip(ui, Pos2::new(full.right() - 10.0, top), model, now);

        if alpha <= 0.0 {
            return;
        }
        ui.set_opacity(alpha);
        self.strip(ui, strip, model, actions);
        self.bar(ui, bar, model, actions);
        if self.menu == Some(Menu::Zoom) {
            self.zoom_menu(ui, bar, model, alpha, actions);
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
        let mut job = LayoutJob::default();
        for (text, color) in parts {
            job.append(&text, 0.0, TextFormat { font_id: font(MONO_MEDIUM, 11.0), color: color.gamma_multiply(alpha), ..Default::default() });
        }
        let painter = ui.painter();
        let galley = painter.layout_job(job);
        let rect = Align2::RIGHT_TOP.anchor_size(right_top, galley.size() + Vec2::new(16.0, 12.0));
        painter.rect_filled(rect, 0.0, CHIP.gamma_multiply(alpha));
        painter.rect_stroke(rect, 0.0, (1.0, LINE.gamma_multiply(alpha)), egui::StrokeKind::Inside);
        painter.galley(rect.min + Vec2::new(8.0, 6.0), galley, FG);
    }

    /// the strip along the top: the clip's tab, and the window's own buttons. it is the title bar: a drag on it
    /// moves the window and a double click maximizes it.
    fn strip(&mut self, ui: &mut egui::Ui, strip: Rect, model: &Model, actions: &mut Vec<Action>) {
        let painter = ui.painter().clone();
        painter.rect_filled(strip, 0.0, BAR);
        painter.rect_filled(Rect::from_min_max(Pos2::new(strip.left(), strip.bottom() - 1.0), strip.max), 0.0, LINE);
        let grab = ui.interact(strip, ui.scope_id().with("strip"), Sense::click_and_drag());
        if grab.double_clicked() {
            actions.push(Action::ToggleMaximize);
        } else if grab.drag_started_by(egui::PointerButton::Primary) {
            actions.push(Action::MoveWindow);
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
                actions.push(action);
            }
        }

        // the clip's tab: the one on screen is the bright, filled one
        let room = (x - strip.left()).min(TAB_MAX) - 12.0 - 8.0;
        let galley = painter.layout_no_wrap(model.name.to_string(), font(MONO_MEDIUM, 11.5), Color32::WHITE);
        let width = galley.size().x.min(room.max(0.0));
        let tab = Rect::from_min_size(strip.min, Vec2::new(12.0 + width + 8.0, STRIP - 1.0));
        painter.rect_filled(tab, 0.0, TAB_ON);
        painter.rect_filled(Rect::from_min_size(Pos2::new(tab.right(), tab.top()), Vec2::new(1.0, tab.height())), 0.0, LINE);
        let text = Rect::from_min_size(Pos2::new(tab.left() + 12.0, tab.top()), Vec2::new(width, tab.height()));
        painter.with_clip_rect(text).galley(Pos2::new(text.left(), text.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    }

    /// the bar along the bottom: one row of controls over the full-width timebar.
    fn bar(&mut self, ui: &mut egui::Ui, bar: Rect, model: &Model, actions: &mut Vec<Action>) {
        let painter = ui.painter().clone();
        painter.rect_filled(bar, 0.0, BAR);
        painter.rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width(), 1.0)), 0.0, LINE);
        // a press on the bar is not a press on the picture under it
        ui.interact(bar, ui.scope_id().with("bar"), Sense::click_and_drag());
        let row = Rect::from_min_size(Pos2::new(bar.left(), bar.top() + 1.0), Vec2::new(bar.width(), ROW));

        // from the left: previous frame, play, next frame, the readout
        let mut x = row.left() + PAD;
        // the next item in the row, `lead` further along than the usual gap
        let mut place = |lead: f32, w: f32, h: f32| {
            let r = Rect::from_min_size(Pos2::new(x + lead, row.center().y - h / 2.0), Vec2::new(w, h));
            x += lead + w + GAP;
            r
        };
        if icon_button(ui, place(0.0, ICON_BUTTON.x, ICON_BUTTON.y), "prev", Icon::Prev) {
            actions.push(Action::Step(-1));
        }
        if icon_button(ui, place(0.0, ICON_BUTTON.x, ICON_BUTTON.y), "play", if model.playing { Icon::Pause } else { Icon::Play }) {
            actions.push(Action::TogglePlay);
        }
        if icon_button(ui, place(0.0, ICON_BUTTON.x, ICON_BUTTON.y), "next", Icon::Next) {
            actions.push(Action::Step(1));
        }
        // the readout: where the clip is, in frames or timecode, and the total. a click switches between the two
        let galley = painter.layout_job(readout(model, self.timecode));
        let rect = place(8.0, galley.size().x + 2.0 * PAD, TEXT_BUTTON);
        if text_button(ui, rect, "readout", galley, false) {
            self.toggle_timecode();
        }

        // from the right, in the design's order (those not built yet leave no gap): the zoom, as a number
        let mut right = row.right() - PAD;
        let mut place = |w: f32, h: f32| {
            right -= w;
            let r = Rect::from_min_size(Pos2::new(right, row.center().y - h / 2.0), Vec2::new(w, h));
            right -= GAP;
            r
        };
        let galley = painter.layout_no_wrap(model.zoom_label.clone(), font(MONO_MEDIUM, 11.5), FG);
        let rect = place(galley.size().x + 2.0 * PAD, TEXT_BUTTON);
        if text_button(ui, rect, "zoom", galley, self.menu == Some(Menu::Zoom)) {
            self.menu = if self.menu == Some(Menu::Zoom) { None } else { Some(Menu::Zoom) };
            self.menu_button = rect;
            self.zoom_text = percent(model.zoom);
        }

        self.timebar(ui, Rect::from_min_max(Pos2::new(bar.left(), row.bottom()), bar.max), model, actions);
    }

    /// the zoom menu: the stops, a long slider, and a field to type into.
    fn zoom_menu(&mut self, ui: &mut egui::Ui, bar: Rect, model: &Model, alpha: f32, actions: &mut Vec<Action>) {
        let anchor = Pos2::new((self.menu_button.center().x - MENU / 2.0).clamp(bar.left() + 6.0, bar.right() - MENU - 6.0), bar.top() - 7.0);
        let shown = widgets::menu(ui, "zoom-menu", anchor, alpha, |ui| {
            widgets::heading(ui, "zoom", &["0", "1", "2"]);
            let stops: [(&str, Option<f32>); 5] = [("fit", None), ("1:1", Some(1.0)), ("2×", Some(2.0)), ("4×", Some(4.0)), ("8×", Some(8.0))];
            let on = stops.iter().position(|(_, z)| match z {
                None => !model.zoomed,
                Some(z) => model.zoomed && (model.zoom - z).abs() < 1e-4,
            });
            let labels: Vec<&str> = stops.iter().map(|s| s.0).collect();
            if let Some(i) = widgets::segments(ui, "zoom-stops", &labels, on) {
                actions.push(Action::Zoom(stops[i].1));
                self.zoom_text = stops[i].1.map_or_else(String::new, percent);
            }
            // the slider runs from 12.5% to 6400%, evenly in doublings
            let (low, high) = (crate::view::LADDER[0], crate::view::LADDER[crate::view::LADDER.len() - 1]);
            let at = ((model.zoom / low).ln() / (high / low).ln()).clamp(0.0, 1.0);
            let typing = widgets::slider_and_field(ui, "zoom-slider", at, &mut self.zoom_text, "%");
            if let Some(t) = typing.slid {
                let zoom = low * (high / low).powf(t);
                actions.push(Action::Zoom(Some(zoom)));
                self.zoom_text = percent(zoom);
            } else if typing.entered {
                if let Ok(p) = self.zoom_text.trim().trim_end_matches('%').parse::<f32>() {
                    actions.push(Action::Zoom(Some((p / 100.0).clamp(low, high))));
                }
            } else if !typing.focused {
                self.zoom_text = percent(model.zoom);
            }
            widgets::note(ui, "100% is one video pixel on one display pixel. from there up, pixels are drawn as squares with no smoothing.");
        });
        self.menu_rect = shown;
        self.view_changed |= !actions.is_empty();
    }

    /// the timebar: the whole width of the window. it thickens under the pointer; a click or a drag goes there.
    fn timebar(&mut self, ui: &mut egui::Ui, area: Rect, model: &Model, actions: &mut Vec<Action>) {
        let painter = ui.painter().clone();
        let response = ui.interact(area, ui.scope_id().with("timebar"), Sense::click_and_drag());
        let last = model.frames.saturating_sub(1).max(1) as f32;
        let frame_at = |x: f32| (((x - area.left()) / area.width()).clamp(0.0, 1.0) * last).round() as usize;
        let scrubbing = response.is_pointer_button_down_on();
        let thick = ui.ctx().animate_bool_with_time(ui.scope_id().with("thick"), response.hovered() || scrubbing, 0.1);
        let height = TRACK_THIN + (TRACK_THICK - TRACK_THIN) * thick;
        let track = Rect::from_min_max(Pos2::new(area.left(), area.bottom() - height), area.max);
        let head = area.left() + area.width() * (model.frame as f32 / last).min(1.0);
        painter.rect_filled(track, 0.0, TRACK);
        painter.rect_filled(Rect::from_min_max(track.min, Pos2::new(head, track.bottom())), 0.0, FILL);
        painter.rect_filled(Rect::from_min_max(Pos2::new(head - 1.0, track.top() - 4.0), Pos2::new(head + 1.0, track.bottom())), 0.0, Color32::WHITE);

        // a drag follows the pointer; a click goes there too, even one that came and went within a single pass
        if let Some(p) = response.interact_pointer_pos().filter(|_| scrubbing || response.clicked()) {
            let to = frame_at(p.x);
            if to != model.frame {
                actions.push(Action::Seek(to));
            }
        }
        // the frame under the pointer, by number. (its picture comes with the timebar preview.)
        if let Some(p) = response.hover_pos() {
            let galley = painter.layout_no_wrap(format!("{:06}", frame_at(p.x)), font(MONO_MEDIUM, 10.5), FG);
            let size = galley.size() + Vec2::new(16.0, 10.0);
            let left = (p.x - size.x / 2.0).clamp(area.left() + 4.0, area.right() - size.x - 4.0);
            let rect = Rect::from_min_size(Pos2::new(left, area.top() - ROW - 8.0 - size.y), size);
            painter.rect_filled(rect, 0.0, SOLID);
            painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
            painter.galley(rect.center() - galley.size() / 2.0, galley, FG);
        }
    }
}

/// a zoom as the field shows it: "200", "35.4".
fn percent(zoom: f32) -> String {
    let p = zoom * 100.0;
    if zoom >= 1.0 { format!("{}", p.round()) } else { format!("{}", (p * 10.0).round() / 10.0) }
}

/// a 30 x 28 button holding one icon. true when clicked.
fn icon_button(ui: &mut egui::Ui, rect: Rect, name: &str, icon: Icon) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(name), Sense::click());
    if response.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 3.0, PRESSED);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, HOVER);
    }
    icons::draw(ui.painter(), rect, icon, FG);
    response.clicked()
}

/// a 28-high button holding text, lit while its menu is open. true when clicked.
fn text_button(ui: &mut egui::Ui, rect: Rect, name: &str, galley: Arc<egui::Galley>, open: bool) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(name), Sense::click());
    if response.hovered() || open {
        ui.painter().rect_filled(rect, 3.0, HOVER);
    }
    ui.painter().galley(Pos2::new(rect.left() + PAD, rect.center().y - galley.size().y / 2.0), galley, FG);
    response.clicked()
}

/// "F 000120 / 000600", or "TC 00:00:02:00 / 00:00:10:00": a small dim label, the place, and the total in dim.
fn readout(model: &Model, timecode: bool) -> LayoutJob {
    let fps = model.rate.map(|(n, d)| (n as f64 / d as f64).round() as usize).filter(|f| *f > 0);
    let (label, at, total) = match fps.filter(|_| timecode) {
        Some(fps) => {
            let tc = |f: usize| format!("{:02}:{:02}:{:02}:{:02}", f / fps / 3600, f / fps / 60 % 60, f / fps % 60, f % fps);
            ("TC", tc(model.frame), tc(model.frames))
        }
        None => ("F", format!("{:06}", model.frame), format!("{:06}", model.frames)),
    };
    let text = |size: f32, color: Color32, spacing: f32| TextFormat { font_id: font(MONO_MEDIUM, size), color, extra_letter_spacing: spacing, ..Default::default() };
    let mut job = LayoutJob::default();
    job.append(label, 0.0, text(9.5, DIM, 9.5 * 0.12));
    job.append(&at, 2.0 + 7.0, text(11.5, FG, 11.5 * 0.02));
    job.append(&format!(" / {total}"), 0.0, text(11.5, DIM, 11.5 * 0.02));
    job
}

/// a line of trouble with the amber edge: the one place amber is used, for a fault in the file.
fn banner(ui: &egui::Ui, at: Pos2, width: f32, text: &str) {
    let painter = ui.painter();
    let galley = painter.layout(text.to_string(), font(MONO, 11.5), DIM, width - 24.0);
    let rect = Rect::from_min_size(at, galley.size() + Vec2::new(22.0, 16.0));
    painter.rect_filled(rect, 0.0, BANNER);
    painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())), 0.0, AMBER);
    painter.galley(rect.min + Vec2::new(12.0, 8.0), galley, DIM);
}
