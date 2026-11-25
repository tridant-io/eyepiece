//! markup mode (fig. 19): the pointer draws on the frame with the tool and the ink chosen in a row of tools at the top
//! middle of the picture. marks are kept in the file's own pixels (`crate::markup`), and the same drawing puts them,
//! with the guides and a caption, into a saved picture (fig. 20).

use super::icons::{self, Icon};
use super::widgets::{CHOSEN, ON_CHOSEN};
use super::{font, keys, text_top, Action, Model, Shot, Ui, FG, HOVER, LINE, MONO_MEDIUM, SANS, SOLID};
use crate::event::Key;
use crate::markup::{Mark, Marks, Placing, Point, Shape, Tool, INKS};
use crate::timecode::Timecode;
use crate::view::Guides;
use egui::text::LayoutJob;
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

// a mark as the prototype's `drawMarks` draws it, in the design's pixels whatever the zoom: its line, and its text on a
// dark box reaching 5 round the letters, 23 high
const INK_WIDTH: f32 = 2.5;
const TEXT_FONT: &str = "sans-semibold";
const TEXT_SIZE: f32 = 13.0;
const TEXT_PAD: f32 = 5.0;
const TEXT_BOX: f32 = 23.0;
const TEXT_BACK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 204); // rgba(0,0,0,.8)
// the field text is typed into (`.pl-mtxt`): 170 wide inside a padding of 6 across and 5 down, a white line round it
const FIELD_WIDTH: f32 = 170.0;
const FIELD_BACK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 209); // rgba(0,0,0,.82)
// the row of tools (`.pl-mk`): 3 inside its line, items 2 apart, a tool 30 wide, an ink 22, a rule with 4 clear each side
const TOOLS_PAD: f32 = 3.0;
const TOOLS_GAP: f32 = 2.0;
const TOOL: f32 = 30.0;
const SWATCH: f32 = 22.0;
const RULE: f32 = 9.0;
const ITEM_HIGH: f32 = 28.0;
// the caption under a saved picture, as fig. 20 draws it on a picture 1200 wide at 2.1 of its pixels to one of the
// design's: a band 62 high, its words 23 high and 20 in, light on near black
const CAPTION_BAND: f32 = 62.0 / 2.1;
const CAPTION_TEXT: f32 = 23.0 / 2.1;
const CAPTION_LEFT: f32 = 20.0 / 2.1;
const CAPTION_BACK: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
const CAPTION_INK: Color32 = Color32::from_rgb(0xe8, 0xe8, 0xe8);
/// room under a saved picture's own for its caption, however many lines it takes: only what is drawn is kept
const CAPTION_ROOM: f32 = 1000.0;

/// markup mode, and what it is in the middle of. the tool and the ink stay chosen when the mode ends.
#[derive(Default)]
pub(super) struct Markup {
    pub(super) on: bool,
    tool: Tool,
    ink: usize,
    /// the mark being drawn, and the frame it goes on
    drawing: Option<(usize, Mark)>,
    writing: Option<Writing>,
    /// where the row of tools was drawn last: the controls stay up while the pointer is on it
    pub(super) row: Option<Rect>,
}

/// text being typed: the frame and the place in the file it goes on, where its field stands, and what is typed so far
struct Writing {
    frame: usize,
    at: Point,
    field: Pos2,
    text: String,
    /// the field opened in this pass: the keyboard goes to it
    fresh: bool,
}

/// where the clip on screen is in the window, for putting marks on it
fn placing(model: &Model) -> Placing {
    Placing { picture: model.picture, look: model.look, size: model.look.shown(model.shown_size) }
}

fn ink(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

impl Markup {
    /// markup mode on or off. off, a mark half drawn and text half typed are dropped.
    pub(super) fn toggle(&mut self) {
        (self.on, self.drawing, self.writing) = (!self.on, None, None);
    }

    /// escape while text is typed: the text goes, and markup mode stays. false when none was being typed.
    pub(super) fn drop_text(&mut self) -> bool {
        self.writing.take().is_some()
    }

    /// what was typed becomes a mark, unless it is only spaces.
    fn keep(&self, writing: Writing, actions: &mut Vec<Action>) {
        let mark = Mark { shape: Shape::Text(writing.at, writing.text.trim().to_string()), ink: INKS[self.ink] };
        if !mark.tiny(1.0) {
            actions.push(Action::Mark(writing.frame, mark));
        }
    }

    /// the picture's pointer, in markup mode: a drag on the picture draws with the tool, and with the text tool a click
    /// opens a field there. a press on the picture keeps what was being typed. nothing else: the window does not move,
    /// the picture does not pan and the controls are not toggled.
    pub(super) fn pointer(&mut self, ui: &egui::Ui, picture: &egui::Response, model: &Model, scale: f32, actions: &mut Vec<Action>) {
        let place = placing(model);
        let (x, y, w, h) = model.picture;
        let on_picture = |p: &Pos2| (x..x + w).contains(&(p.x * scale)) && (y..y + h).contains(&(p.y * scale));
        let file = |p: Pos2| place.to_file((p.x * scale, p.y * scale));
        if ui.input(|i| i.pointer.primary_pressed()) && picture.hovered() {
            if let Some(writing) = self.writing.take() {
                self.keep(writing, actions);
            }
        }
        if picture.drag_started_by(egui::PointerButton::Primary) && self.tool != Tool::Text {
            let origin = ui.input(|i| i.pointer.press_origin());
            if let Some(origin) = origin.filter(on_picture) {
                self.drawing = Some((model.frame, Mark::start(self.tool, file(origin), INKS[self.ink])));
            }
        }
        if let (Some((_, mark)), Some(at)) = (&mut self.drawing, picture.interact_pointer_pos()) {
            mark.reach(file(at));
        }
        if picture.drag_stopped() {
            // how many of the design's pixels one of the file's takes on screen
            let per = w / model.shown_size.0.max(1) as f32 / scale;
            if let Some((frame, mark)) = self.drawing.take().filter(|(_, mark)| !mark.tiny(per)) {
                actions.push(Action::Mark(frame, mark));
            }
        }
        if self.tool == Tool::Text && picture.clicked_by(egui::PointerButton::Primary) {
            if let Some(at) = picture.interact_pointer_pos().filter(on_picture) {
                self.writing = Some(Writing { frame: model.frame, at: file(at), field: at, text: String::new(), fresh: true });
            }
        }
    }

    /// the marks on the frame on screen, and the one being drawn on it, over the picture and clipped to it.
    pub(super) fn marks(&self, ui: &egui::Ui, model: &Model, scale: f32) {
        let (x, y, w, h) = model.picture;
        let painter = ui.painter().with_clip_rect(Rect::from_min_size(Pos2::new(x / scale, y / scale), Vec2::new(w / scale, h / scale)));
        let drawing = self.drawing.iter().filter(|(frame, _)| *frame == model.frame).map(|(_, mark)| mark);
        draw_marks(&painter, model.marks.map_or(&[][..], |marks| marks.on(model.frame)).iter().chain(drawing), &placing(model), scale);
    }

    /// the field text is typed into, where the click was and kept inside `full`. enter keeps what was typed and escape
    /// drops it; the field losing the keyboard otherwise keeps it too.
    pub(super) fn field(&mut self, ui: &egui::Ui, full: Rect, actions: &mut Vec<Action>) {
        let color = ink(INKS[self.ink]);
        let Some(writing) = &mut self.writing else { return };
        let at = Pos2::new(writing.field.x.clamp(full.left() + 6.0, full.right() - FIELD_WIDTH), writing.field.y.clamp(full.top() + 6.0, full.bottom() - 40.0));
        let mut done = None;
        egui::Area::new(egui::Id::unique("markup-text")).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
            egui::Frame::new().fill(FIELD_BACK).stroke((1.0, Color32::WHITE)).inner_margin(egui::Margin::symmetric(6, 5)).show(ui, |ui| {
                let edit = egui::TextEdit::singleline(&mut writing.text).id(egui::Id::unique("markup-text-edit")).frame(egui::Frame::NONE).margin(egui::Margin::ZERO);
                let edit = ui.add(edit.font(font(TEXT_FONT, TEXT_SIZE)).text_color(color).hint_text("type, then enter").desired_width(FIELD_WIDTH));
                if std::mem::take(&mut writing.fresh) {
                    edit.request_focus();
                }
                if edit.lost_focus() {
                    done = Some(!ui.input(|i| i.key_pressed(egui::Key::Escape)));
                }
            });
        });
        match done {
            Some(true) => {
                if let Some(writing) = self.writing.take() {
                    self.keep(writing, actions);
                }
            }
            Some(false) => self.writing = None,
            None => {}
        }
    }

    /// the row of tools at the top middle of the picture while markup mode is on (`.pl-mk`): the four tools, the four
    /// inks, undo and clear, then save image and done. `top` is its top edge.
    pub(super) fn tools(&mut self, ui: &mut egui::Ui, middle: f32, top: f32, actions: &mut Vec<Action>) {
        enum Item {
            Pick(Tool, Icon),
            Ink(usize),
            Rule,
            Undo,
            Clear,
            Words(&'static str),
        }
        use Item::*;
        let items = [
            Pick(Tool::Rectangle, Icon::Rectangle),
            Pick(Tool::Circle, Icon::Circle),
            Pick(Tool::Pen, Icon::Pen),
            Pick(Tool::Text, Icon::Text),
            Rule,
            Ink(0),
            Ink(1),
            Ink(2),
            Ink(3),
            Rule,
            Undo,
            Clear,
            Rule,
            Words("save image"),
            Words("done"),
        ];
        let painter = ui.painter().clone();
        let words = |text: &str| painter.layout_no_wrap(text.to_string(), font(SANS, 12.0), FG);
        let wide = |item: &Item| match item {
            Pick(..) | Undo | Clear => TOOL,
            Ink(_) => SWATCH,
            Rule => RULE,
            Words(text) => words(text).size().x + 16.0,
        };
        let inner: f32 = items.iter().map(wide).sum::<f32>() + TOOLS_GAP * (items.len() - 1) as f32;
        let row = Rect::from_min_size(Pos2::new(middle - (inner / 2.0 + TOOLS_PAD + 1.0), top), Vec2::new(inner + 2.0 * (TOOLS_PAD + 1.0), ITEM_HIGH + 2.0 * (TOOLS_PAD + 1.0)));
        self.row = Some(row);
        painter.rect_filled(row, 0.0, SOLID);
        painter.rect_stroke(row, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
        // a press on the row is not a press on the picture under it
        ui.interact(row, ui.scope_id().with("markup-tools"), Sense::click_and_drag());
        let mut x = row.left() + 1.0 + TOOLS_PAD;
        for (i, item) in items.iter().enumerate() {
            let rect = Rect::from_min_size(Pos2::new(x, row.top() + 1.0 + TOOLS_PAD), Vec2::new(wide(item), ITEM_HIGH));
            x = rect.right() + TOOLS_GAP;
            if matches!(item, Rule) {
                painter.rect_filled(Rect::from_center_size(rect.center(), Vec2::new(1.0, 18.0)), 0.0, LINE);
                continue;
            }
            let response = ui.interact(rect, ui.scope_id().with(("markup-tool", i)), Sense::click());
            let chosen = matches!(item, Pick(tool, _) if *tool == self.tool);
            if chosen {
                painter.rect_filled(rect, 3.0, CHOSEN);
            } else if response.hovered() && !matches!(item, Ink(_)) {
                painter.rect_filled(rect, 3.0, HOVER);
            }
            match item {
                Pick(_, icon) => icons::draw(&painter, rect, *icon, if chosen { ON_CHOSEN } else { FG }),
                Undo => icons::draw(&painter, rect, Icon::Undo, FG),
                Clear => icons::draw(&painter, rect, Icon::Close, FG),
                // a disc 12 across in its ink, ringed faintly; the chosen one is ringed in white, 2 clear of it
                Ink(n) => {
                    painter.circle_filled(rect.center(), 6.0, ink(INKS[*n]));
                    if *n == self.ink {
                        painter.circle_stroke(rect.center(), 7.0, (2.0, SOLID));
                        painter.circle_stroke(rect.center(), 8.5, (1.0, Color32::WHITE));
                    } else {
                        painter.circle_stroke(rect.center(), 6.5, (1.0, Color32::from_white_alpha(89)));
                    }
                }
                Words(text) => {
                    painter.galley(Pos2::new(rect.left() + 8.0, text_top(&painter, font(SANS, 12.0), rect.center().y)), words(text), FG);
                }
                Rule => {}
            }
            if response.clicked() {
                match item {
                    Pick(tool, _) => self.tool = *tool,
                    Ink(n) => self.ink = *n,
                    // these go the way their keys do
                    Undo => actions.push(Action::Key(Key::Z, keys::CTRL)),
                    Words("done") => actions.push(Action::Key(Key::E, keys::NONE)),
                    Clear => actions.push(Action::ClearMarks),
                    Words(_) => actions.push(Action::Shot(Shot::Frame)),
                    Rule => {}
                }
            }
        }
    }
}

/// a tick over the timebar at each frame that carries marks: 1 wide and 5 high, 8 over the track. `last` is the
/// frame at the timebar's right end.
pub(super) fn ticks(painter: &egui::Painter, area: Rect, track: Rect, last: f32, marks: &Marks) {
    for frame in marks.frames() {
        let x = area.left() + area.width() * (frame as f32 / last).min(1.0);
        painter.rect_filled(Rect::from_min_max(Pos2::new(x, track.top() - 8.0), Pos2::new(x + 1.0, track.top() - 3.0)), 0.0, Color32::WHITE);
    }
}

/// draws marks: `place` puts the file's pixels in the target and `scale` of the target's pixels make one of the
/// design's. lines and text are the design's size, whatever the zoom.
fn draw_marks<'a>(painter: &egui::Painter, marks: impl Iterator<Item = &'a Mark>, place: &Placing, scale: f32) {
    let to = |p: &Point| {
        let (x, y) = place.to_window(*p);
        Pos2::new(x / scale, y / scale)
    };
    for mark in marks {
        let color = ink(mark.ink);
        let stroke = Stroke::new(INK_WIDTH, color);
        match &mark.shape {
            Shape::Rectangle(a, b) => {
                painter.rect_stroke(Rect::from_two_pos(to(a), to(b)), 0.0, stroke, egui::StrokeKind::Middle);
            }
            // egui's own ellipse takes 8 points a quarter up to a radius of 128 pixels, which shows its corners in a
            // saved picture: here a point every pixel and a half or so round it
            Shape::Circle(a, b) => {
                let rect = Rect::from_two_pos(to(a), to(b));
                let n = ((rect.width() + rect.height()) * scale).clamp(32.0, 2048.0) as usize;
                let round = (0..n).map(|i| i as f32 * std::f32::consts::TAU / n as f32).map(|t| rect.center() + Vec2::new(t.cos(), t.sin()) * rect.size() / 2.0);
                painter.add(egui::Shape::closed_line(round.collect(), stroke));
            }
            Shape::Pen(points) => {
                painter.add(egui::Shape::line(points.iter().map(to).collect(), stroke));
            }
            Shape::Text(at, text) => {
                let at = to(at);
                let galley = painter.layout_no_wrap(text.clone(), font(TEXT_FONT, TEXT_SIZE), color);
                let back = Rect::from_min_size(at - Vec2::splat(TEXT_PAD), Vec2::new(galley.size().x + 2.0 * TEXT_PAD, TEXT_BOX));
                painter.rect_filled(back, 0.0, TEXT_BACK);
                painter.galley(Pos2::new(at.x, text_top(painter, font(TEXT_FONT, TEXT_SIZE), back.center().y)), galley, color);
            }
        }
    }
}

/// what a saved picture carries over it: the guides and the marks, and a caption under it, laid out by a pass of egui
/// of its own for `UiPainter` to paint after the picture (fig. 20).
pub struct Saved {
    /// the saved picture's height, the caption included
    pub height: u32,
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub textures: egui::TexturesDelta,
    /// the saved picture's pixels for one of the design's
    pub scale: f32,
}

/// lays out what goes over a picture `size` pixels big, `scale` of them to one of the design's: the guides, `marks`
/// where `place` puts them, and the caption under it all, so the caption covers no pixel.
pub fn saved(size: (u32, u32), scale: f32, place: &Placing, guides: Option<Guides>, marks: &[Mark], caption: Option<&str>) -> Saved {
    let ctx = egui::Context::default();
    ctx.set_fonts(super::fonts_with_system());
    let (w, h) = (size.0 as f32 / scale, size.1 as f32 / scale);
    let mut raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(w, h + CAPTION_ROOM))), ..Default::default() };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
    let mut under = 0.0;
    let out = ctx.run_ui(raw, |ui| {
        if let Some(guides) = guides {
            Ui::draw_guides(ui.painter(), guides, place.picture, place.look.shown(place.size), scale);
        }
        let (x, y, pw, ph) = place.picture;
        let painter = ui.painter().with_clip_rect(Rect::from_min_size(Pos2::new(x / scale, y / scale), Vec2::new(pw / scale, ph / scale)));
        draw_marks(&painter, marks.iter(), place, scale);
        if let Some(words) = caption {
            under = caption_band(ui.painter(), words, Pos2::new(0.0, h), w);
        }
    });
    Saved { height: size.1 + (under * scale).round() as u32, primitives: ctx.tessellate(out.shapes, out.pixels_per_point), textures: out.textures_delta, scale: out.pixels_per_point }
}

/// the caption: a dark band as wide as the picture, from `at` down, its words on one line, or more on a narrow
/// picture. returns the band's height.
fn caption_band(painter: &egui::Painter, words: &str, at: Pos2, width: f32) -> f32 {
    let galley = painter.layout_job(LayoutJob::simple(words.to_string(), font(MONO_MEDIUM, CAPTION_TEXT), CAPTION_INK, width - 2.0 * CAPTION_LEFT));
    let rows = galley.rows.len().max(1);
    let height = CAPTION_BAND + galley.size().y / rows as f32 * (rows - 1) as f32;
    painter.rect_filled(Rect::from_min_size(at, Vec2::new(width, height)), 0.0, CAPTION_BACK);
    painter.galley(Pos2::new(at.x + CAPTION_LEFT, text_top(painter, font(MONO_MEDIUM, CAPTION_TEXT), at.y + CAPTION_BAND / 2.0)), galley, CAPTION_INK);
    height
}

/// what the caption says (fig. 20): the clip's name, the frame as the readout numbers it (from `first`), what the
/// clip's clock says, the size, and the rate. a clip with no one rate has no timecode and no rate.
pub fn caption(name: &str, frame: usize, first: usize, clock: Option<Timecode>, rate: Option<(i64, i64)>, size: (usize, usize)) -> String {
    let mut parts = vec![name.to_string(), format!("F {:06}", frame + first)];
    let fps = rate.filter(|(n, d)| *n > 0 && *d > 0).map(|(n, d)| n as f64 / d as f64);
    if let Some(clock) = clock {
        parts.push(format!("TC {}", clock.at(frame)));
    }
    parts.push(format!("{} × {}", size.0, size.1));
    if let Some(fps) = fps {
        parts.push(if fps.fract() == 0.0 { format!("{fps} fps") } else { format!("{fps:.2} fps") });
    }
    parts.join("   ")
}

#[cfg(test)]
mod tests {
    use super::super::tests::{drive, drive_with, model};
    use super::super::{Extras, Ui};
    use super::*;
    use crate::event::Button::{self, Left};
    use crate::event::Mods;

    type Step = (f32, f32, Option<(Button, bool)>);

    /// a press, a drag through the middle and the release, a pass for each
    fn drag(from: (f32, f32), to: (f32, f32)) -> Vec<Step> {
        let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
        vec![(from.0, from.1, None), (from.0, from.1, Some((Left, true))), (mid.0, mid.1, None), (to.0, to.1, None), (to.0, to.1, Some((Left, false))), (to.0, to.1, None)]
    }

    fn click(at: (f32, f32)) -> Vec<Step> {
        vec![(at.0, at.1, None), (at.0, at.1, Some((Left, true))), (at.0, at.1, Some((Left, false))), (at.0, at.1, None)]
    }

    /// a place in the test clip's file, from a point of the window: the 64 x 64 picture is 360 wide from 140 across
    fn file(at: (f32, f32)) -> Point {
        ((at.0 - 140.0) * 64.0 / 360.0, at.1 * 64.0 / 360.0)
    }

    fn near(a: Point, b: Point) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    /// a drag on the picture draws with the tool, on the frame on screen, in the file's pixels; it does not move the
    /// window, nor the picture when that is larger than the window, as the same drag does with markup mode off.
    #[test]
    fn a_drag_draws_and_moves_neither_the_window_nor_the_picture() {
        let (from, to) = ((200.0, 100.0), (260.0, 160.0));
        for overflows in [false, true] {
            let mut ui = Ui::new((640, 360), 1.0, false, None, None);
            let plain = drive_with(&mut ui, Model { overflows, ..model() }, &drag(from, to));
            assert!(plain.iter().any(|a| if overflows { matches!(a, Action::Pan(_)) } else { *a == Action::MoveWindow }), "markup off: {plain:?}");
            let mut ui = Ui::new((640, 360), 1.0, false, None, None);
            ui.toggle_markup();
            let marked = drive_with(&mut ui, Model { overflows, ..model() }, &drag(from, to));
            match &marked[..] {
                [Action::Mark(0, Mark { shape: Shape::Rectangle(a, b), ink })] => assert!(near(*a, file(from)) && near(*b, file(to)) && *ink == INKS[0], "{marked:?}"),
                _ => panic!("markup on, overflowing {overflows}: {marked:?}"),
            }
        }
    }

    /// the pointer is a cross where a drag would draw: on the picture in markup mode, not on the row of tools or the
    /// bar, and not with markup off or the pointer out of the window.
    #[test]
    fn the_pointer_is_a_cross_where_a_drag_would_draw() {
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        drive(&mut ui, &[(300.0, 200.0, None)]);
        assert!(!ui.cross, "markup off");
        ui.toggle_markup();
        drive(&mut ui, &[(300.0, 200.0, None)]);
        assert!(ui.cross, "on the picture");
        // the row of tools stands 10 to 46 down while the controls are away; 44 down is under the strip's band
        let row = ui.markup.row.expect("the row of tools");
        assert!(row.contains(Pos2::new(320.0, 44.0)), "{row:?}");
        drive(&mut ui, &[(320.0, 44.0, None)]);
        assert!(!ui.cross, "on the row of tools");
        drive(&mut ui, &[(300.0, 200.0, None), (300.0, 340.0, None)]);
        assert!(!ui.cross, "on the bar");
        drive(&mut ui, &[(300.0, 200.0, None)]);
        assert!(ui.cross);
        ui.pointer_leave();
        let mut out = ui.run(&super::super::Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(model()), error: None, opening: None, list: None, browsing: false, fullscreen: false, toast: None, settings: &super::super::tests::DEFAULTS });
        out.textures.clear();
        assert!(!out.cross, "out of the window");
    }

    /// a click and a double click on the picture neither play or pause nor fill the display in markup mode, as
    /// they do with it off; and the pointer in the middle of the picture does not bring the controls up.
    #[test]
    fn a_click_and_a_double_click_do_nothing_in_markup_mode() {
        let at = (300.0, 200.0);
        let double = [click(at), click(at)].concat();
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        assert_eq!(drive(&mut ui, &double), vec![Action::TogglePlay, Action::Fullscreen, Action::PlayAsBefore], "markup off");
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        assert_eq!(drive(&mut ui, &double), vec![]);
        assert!(!ui.shown, "the controls stayed away");
        // the bottom of the window brings them up
        drive(&mut ui, &[(300.0, 330.0, None)]);
        assert!(ui.shown);
    }

    /// with the text tool a click opens a field with the keyboard in it; enter keeps what was typed as a mark where
    /// the click was, a press elsewhere on the picture keeps it too, and escape drops it and leaves markup mode on.
    #[test]
    fn text_is_kept_by_enter_or_a_press_elsewhere_and_dropped_by_escape() {
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        ui.markup.tool = Tool::Text;
        let (first, second) = ((200.0, 100.0), (400.0, 250.0));
        drive(&mut ui, &click(first));
        assert!(ui.typing(), "the field has the keyboard");
        "hi ".chars().for_each(|c| ui.char(c));
        ui.key(Key::Enter, Mods::default());
        let kept = drive(&mut ui, &[(first.0, first.1, None)]);
        assert!(matches!(&kept[..], [Action::Mark(0, Mark { shape: Shape::Text(at, words), .. })] if near(*at, file(first)) && words == "hi"), "{kept:?}");
        assert!(ui.markup.writing.is_none() && ui.markup.on && !ui.typing());
        // typed, then a click elsewhere on the picture: kept, and a new field there
        drive(&mut ui, &click(first));
        "ok".chars().for_each(|c| ui.char(c));
        drive(&mut ui, &[(first.0, first.1, None)]);
        let kept = drive(&mut ui, &click(second));
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(matches!(&kept[0], Action::Mark(0, Mark { shape: Shape::Text(at, words), .. }) if near(*at, file(first)) && words == "ok"), "{kept:?}");
        assert!(ui.markup.writing.as_ref().is_some_and(|w| near(w.at, file(second))));
        // escape, as the window sends it while the field has the keyboard
        "gone".chars().for_each(|c| ui.char(c));
        ui.key(Key::Escape, Mods::default());
        assert!(ui.escape());
        assert_eq!(drive(&mut ui, &[(second.0, second.1, None), (second.0, second.1, None)]), vec![]);
        assert!(ui.markup.writing.is_none() && ui.markup.on, "escape took the text and left markup mode on");
        assert!(!ui.typing(), "the keys are the player's again");
        // the next escape ends markup mode
        assert!(ui.escape() && !ui.markup.on);
    }

    /// a click on the row of tools picks a tool or an ink, or does what undo, clear, save image and done say (their
    /// keys, or the screenshot), and draws nothing.
    #[test]
    fn the_row_of_tools_does_what_it_says() {
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        // the row stands under the strip's place, the controls up or not: where it is with them away, it is with them
        // up, so it never moves from under the pointer
        drive(&mut ui, &[(320.0, 300.0, None)]);
        let away = ui.markup.row.expect("the row of tools is up");
        drive(&mut ui, &[(320.0, 20.0, None)]);
        std::thread::sleep(std::time::Duration::from_millis(300));
        drive(&mut ui, &[(320.0, 20.0, None)]);
        assert_eq!(ui.markup.row, Some(away));
        // a click `along` from the row's left edge (or, negative, from its right), on the middle of its items
        let mut press = |along: f32| {
            let row = ui.markup.row.expect("the row of tools is up");
            let x = if along < 0.0 { row.right() + along } else { row.left() + along };
            drive(&mut ui, &click((x, row.top() + 4.0 + ITEM_HIGH / 2.0)))
        };
        // inside the line and the padding (4): the tools 32 apart, a rule 11, the inks 24 apart, a rule, undo, clear,
        // a rule, then the words
        assert_eq!(press(4.0 + 3.0 * 32.0 + 15.0), vec![]);
        assert_eq!(press(4.0 + 4.0 * 32.0 + 11.0 + 24.0 + 11.0), vec![]);
        assert_eq!(press(4.0 + 246.0 + 15.0), vec![Action::Key(Key::Z, keys::CTRL)]);
        assert_eq!(press(4.0 + 246.0 + 32.0 + 15.0), vec![Action::ClearMarks]);
        assert_eq!(press(4.0 + 246.0 + 64.0 + 11.0 + 20.0), vec![Action::Shot(Shot::Frame)]);
        assert_eq!(press(-14.0), vec![Action::Key(Key::E, keys::NONE)]);
        assert_eq!((ui.markup.tool, ui.markup.ink), (Tool::Text, 1));
    }

    /// the screenshot menu's switches and its "markup mode" button.
    #[test]
    fn the_screenshot_menu_switches_what_a_saved_picture_carries_and_starts_markup() {
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        // the screenshot button: the third from the bar's right end, 32 up
        drive(&mut ui, &[(553.0, 328.0, None), (553.0, 328.0, None)]);
        drive(&mut ui, &click((553.0, 328.0)));
        drive(&mut ui, &[(553.0, 328.0, None), (553.0, 328.0, None)]);
        let menu = ui.menu_rect;
        // inside its line and padding (13), from the bottom: the "markup mode" button 30 high, then the switches,
        // 15.6 high and 11 apart: markup, guides, the caption
        let switch = |n: f32| menu.bottom() - 13.0 - 30.0 - 11.0 - 7.8 - n * 26.6;
        let guides = drive(&mut ui, &click((menu.left() + 60.0, switch(1.0))));
        assert_eq!(guides, vec![Action::Extras(Extras { caption: false, guides: true, markup: true })]);
        let caption = drive(&mut ui, &click((menu.left() + 60.0, switch(2.0))));
        assert_eq!(caption, vec![Action::Extras(Extras { caption: true, guides: false, markup: true })]);
        let mode = drive(&mut ui, &click((menu.left() + 40.0, menu.bottom() - 13.0 - 15.0)));
        assert_eq!(mode, vec![Action::Key(Key::E, Mods::default())]);
    }

    #[test]
    fn the_caption_says_what_fig_20_says() {
        assert_eq!(caption("wall_main_v012.mov", 742, 0, Some(Timecode::counted(24)), Some((24000, 1001)), (1920, 1080)), "wall_main_v012.mov   F 000742   TC 00:00:30:22   1920 × 1080   23.98 fps");
        assert_eq!(caption("hapq_64.mov", 2, 0, Some(Timecode::counted(30)), Some((15360, 512)), (64, 64)), "hapq_64.mov   F 000002   TC 00:00:00:02   64 × 64   30 fps");
        assert_eq!(caption("still.exr", 0, 0, None, None, (16, 16)), "still.exr   F 000000   16 × 16");
        // the file's own timecode, and frame numbers from 1
        assert_eq!(caption("av.mov", 2, 1, Timecode::parse("01:00:00;00", 30, 30), Some((30000, 1001)), (64, 64)), "av.mov   F 000003   TC 01:00:00;02   64 × 64   29.97 fps");
    }

    /// every letter the caption, the row of tools and the field use is in the fonts they are drawn in.
    #[test]
    fn markups_words_can_be_drawn() {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let has = |words: &str, family: &str, size: f32| ui.ctx().fonts_mut(|f| f.has_glyphs(&font(family, size), words));
            assert!(has(&caption("wall_main_v012.mov", 742, 0, Timecode::parse("01:00:00;00", 24, 24), Some((24000, 1001)), (1920, 1080)), MONO_MEDIUM, CAPTION_TEXT));
            assert!(has("save image done", SANS, 12.0));
            assert!(has("type, then enter", TEXT_FONT, TEXT_SIZE));
        });
        out.textures_delta.clear();
    }
}
