//! markup mode (fig. 19): the pointer draws on the frame with the tool and the ink chosen in a row of tools at the top
//! middle of the picture. marks are kept in the file's own pixels (`crate::markup`), in groups that each show over a
//! range of frames: a group is a bar on the timebar, which a drag resizes and moves as a trim is set. the same drawing
//! puts the marks, with the guides and a caption, into a saved picture (fig. 20).

use super::icons::{self, Icon};
use super::widgets::{self, CHOSEN, ON_CHOSEN};
use super::{font, keys, stamp, text_top, Action, Model, Shot, Ui, DIM, FG, FILL, HOVER, LINE, MONO_MEDIUM, SANS, SOLID, TRACK_THICK};
use crate::event::Key;
use crate::markup::{dragged, lanes, scale_by, smooth, square, Mark, Marks, Part, Placing, Point, Shape, Tool, INKS};
use crate::timecode::Timecode;
use crate::view::Guides;
use egui::text::LayoutJob;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Vec2};

// a mark as the prototype's `drawMarks` draws it, in the design's pixels whatever the zoom: its line, and its text on a
// dark box reaching 5 round the letters, 23 high
const INK_WIDTH: f32 = 2.5;
const TEXT_FONT: &str = "sans-semibold";
const TEXT_SIZE: f32 = 13.0;
const TEXT_PAD: f32 = 5.0;
const TEXT_BOX: f32 = 23.0;
const TEXT_BACK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 204); // rgba(0,0,0,.8)
const TEXT_BACK_LIGHT: Color32 = Color32::from_rgba_premultiplied(204, 204, 204, 204); // rgba(255,255,255,.8)
// the field text is typed into (`.pl-mtxt`): 170 wide inside a padding of 6 across and 5 down, a white line round it
const FIELD_WIDTH: f32 = 170.0;
const FIELD_BACK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 209); // rgba(0,0,0,.82)
// the row of tools (`.pl-mk`): 3 inside its line, items 2 apart, a tool 30 wide, an ink 18 (the design's 22 for its
// four; ten at 18 fit a window 640 wide), a rule with 4 clear each side
const TOOLS_PAD: f32 = 3.0;
const TOOLS_GAP: f32 = 2.0;
const TOOL: f32 = 30.0;
const SWATCH: f32 = 18.0;
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
/// how near a press must come to a mark's line, or to a corner of a selected mark's bounds, to take it
const REACH: f32 = 6.0;
// a selected mark's bounds stand 4 out from it, a 1 line round them with a handle 7 across at each corner, in the
// light the chosen tool is lit with (no accent: that is for sliders, switches and fields)
const GRIP_OUT: f32 = 4.0;
const HANDLE: f32 = 7.0;
/// text is scaled from half the design's size to eight times it
const TEXT_SIZES: (f32, f32) = (0.5, 8.0);
// a mark's menu, as wide as the bar's menus, its rows 24 high as the right-click menu's
const MENU_WIDTH: f32 = 268.0;
const MENU_ROW: f32 = 24.0;
// a group's bar on the timebar: 2 high and 1 clear of the track at its thickest, so the track never grows over it; a
// second lane 3 higher for a group that overlaps another, and no third: there is no room over the track for one
const BAR_HIGH: f32 = 2.0;
const LANE: f32 = 3.0;
const LANES: usize = 2;
/// how near an end of a group's bar a press must be to take that end, and how far past the bar it still takes it;
/// and how near a marker of the trim, to take that
pub(super) const END_REACH: f32 = 4.0;

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
    /// shift is held: a box or a circle being drawn is kept as wide as it is high on screen
    pub(super) shift: bool,
    /// the mark selected: its frame, its place among the frame's marks, and the mark as it was. it is let go when the
    /// frame's marks change under it (undo, another frame, another tab)
    selected: Option<(usize, usize, Mark)>,
    /// what a press of the select tool took hold of, until the button comes up
    held: Option<Held>,
    /// the selected mark's menu, which a right click on it opened, where it was asked for; and where it was drawn last
    pub(super) menu: Option<Pos2>,
    pub(super) menu_rect: Option<Rect>,
    /// the group selected: its place among the clip's groups, and its range as last seen. it is let go when the
    /// groups change under it (undo, another clip, another tab)
    group: Option<(usize, (usize, usize))>,
    /// a mark was drawn on this frame: the group it went into is selected at the next pass, when it is there
    adopt: Option<usize>,
    /// what a press on a group's bar took hold of, until the button comes up
    grip: Option<Grip>,
    /// the pointer is on an end of a group's bar, or has one in hand: it is shown as a two-way arrow
    at_end: bool,
}

/// a hold on a group's bar: the group, the part of the bar, the frame under the press, and its range then and as the
/// drag has it so far
struct Grip {
    group: usize,
    part: Part,
    from: usize,
    was: (usize, usize),
    now: (usize, usize),
}

/// a hold on the mark selected: a corner of its bounds to scale it by (or none, to move it), where the press was and
/// its bounds then, in the design's pixels, and the mark as the drag has it so far
struct Held {
    corner: Option<usize>,
    from: Pos2,
    bounds: Rect,
    now: Option<Mark>,
}

/// text being typed: the frame and the place in the file it goes on, where its field stands, and what is typed so far
struct Writing {
    frame: usize,
    at: Point,
    field: Pos2,
    text: String,
    /// the field opened in this pass: the keyboard goes to it
    fresh: bool,
    /// the text mark it is the text of, put back in the field by its menu; its size and ink
    replaces: Option<usize>,
    size: f32,
    ink: [u8; 3],
}

/// where a mark is drawn, in the design's pixels: the line it is drawn with (a box's sides, a circle's round, a pen's
/// line), or a text's box
enum Seen {
    Line(Vec<Pos2>),
    Box(Rect),
}

impl Seen {
    fn of(painter: &egui::Painter, mark: &Mark, place: &Placing, scale: f32) -> Seen {
        let to = |p: &Point| {
            let (x, y) = place.to_window(*p);
            Pos2::new(x / scale, y / scale)
        };
        match &mark.shape {
            Shape::Rectangle(a, b) => {
                let r = Rect::from_two_pos(to(a), to(b));
                Seen::Line(vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()])
            }
            Shape::Circle(a, b) => {
                let r = Rect::from_two_pos(to(a), to(b));
                Seen::Line((0..=64).map(|i| i as f32 * std::f32::consts::TAU / 64.0).map(|t| r.center() + Vec2::new(t.cos(), t.sin()) * r.size() / 2.0).collect())
            }
            Shape::Pen(points) => Seen::Line(points.iter().map(to).collect()),
            Shape::Text(at, text, size) => Seen::Box(text_box(painter, to(at), text, *size, FG).0),
        }
    }

    fn bounds(&self) -> Rect {
        match self {
            Seen::Line(points) => Rect::from_points(points),
            Seen::Box(rect) => *rect,
        }
    }

    /// a press at `at` is on it: within reach of its line, or in its box
    fn under(&self, at: Pos2) -> bool {
        let off = |a: Pos2, b: Pos2| {
            let ab = b - a;
            let t = if ab.length_sq() > 0.0 { ((at - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
            at.distance(a + ab * t)
        };
        match self {
            Seen::Line(points) => points.windows(2).any(|w| off(w[0], w[1]) <= REACH),
            Seen::Box(rect) => rect.contains(at),
        }
    }
}

/// the mark of `seen` a press at `at` takes: the last drawn whose line it is on or whose text box holds it; else the
/// smallest whose bounds hold it, so a mark drawn inside another can be had from inside it
fn pick(seen: &[Seen], at: Pos2) -> Option<usize> {
    seen.iter().rposition(|s| s.under(at)).or_else(|| {
        let holding = seen.iter().enumerate().filter(|(_, s)| s.bounds().expand(REACH).contains(at));
        holding.min_by(|a, b| a.1.bounds().area().total_cmp(&b.1.bounds().area())).map(|(i, _)| i)
    })
}

/// the box a selected mark's handles stand on: its bounds, a little out from it
fn grip_box(seen: &Seen) -> Rect {
    seen.bounds().expand(GRIP_OUT)
}

/// a box's corners, clockwise from its upper left: each is across from the one two on
fn corners(r: Rect) -> [Pos2; 4] {
    [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()]
}

/// a text's box and its letters laid out, as drawn with its first letter's top left at `at`: 5 round the letters and
/// 23 high at the design's size, all of it grown by `size`
fn text_box(painter: &egui::Painter, at: Pos2, text: &str, size: f32, color: Color32) -> (Rect, std::sync::Arc<egui::Galley>) {
    let galley = painter.layout_no_wrap(text.to_string(), font(TEXT_FONT, TEXT_SIZE * size), color);
    let back = Rect::from_min_size(at - Vec2::splat(TEXT_PAD * size), Vec2::new(galley.size().x + 2.0 * TEXT_PAD * size, TEXT_BOX * size));
    (back, galley)
}

/// an ink's disc, 12 across and ringed faintly; the chosen one is ringed in white, 2 clear of it
fn swatch(painter: &egui::Painter, middle: Pos2, rgb: [u8; 3], chosen: bool) {
    painter.circle_filled(middle, 6.0, ink(rgb));
    if chosen {
        painter.circle_stroke(middle, 7.0, (2.0, SOLID));
        painter.circle_stroke(middle, 8.5, (1.0, Color32::WHITE));
    } else {
        painter.circle_stroke(middle, 6.5, (1.0, Color32::from_white_alpha(89)));
    }
}

/// a row of a mark's menu, as the right-click menu draws one: its words, and its key at the right. true when clicked
fn menu_row(ui: &mut egui::Ui, name: &str, key: Option<&str>) -> bool {
    let row = widgets::row(ui, MENU_ROW);
    let response = ui.interact(row, ui.scope_id().with(("mark-menu", name)), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(row, 3.0, HOVER);
    }
    let galley = ui.painter().layout_no_wrap(name.to_string(), font(SANS, 12.0), FG);
    ui.painter().galley(Pos2::new(row.left() + 6.0, text_top(ui.painter(), font(SANS, 12.0), row.center().y)), galley, FG);
    if let Some(key) = key {
        widgets::key_cap(ui, Pos2::new(row.right() - 6.0, row.center().y), key, false, DIM);
    }
    response.clicked()
}

/// where the clip on screen is in the window, for putting marks on it
fn placing(model: &Model) -> Placing {
    Placing { picture: model.picture, look: model.turned, size: model.turned.shown(model.shown_size) }
}

fn ink(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// the box behind text: dark, or light for an ink too dark to read on it (black, indigo), so every ink reads at 4.5:1
/// or better
fn text_back(rgb: [u8; 3]) -> Color32 {
    if contrast(rgb, [0, 0, 0]) < 4.5 { TEXT_BACK_LIGHT } else { TEXT_BACK }
}

/// the contrast of two colors, as wcag counts it: 1 to 21
fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let lum = |rgb: [u8; 3]| {
        let [r, g, b] = rgb.map(|c| {
            let c = c as f32 / 255.0;
            if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        });
        0.2126 * r + 0.7152 * g + 0.0722 * b
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

impl Markup {
    /// markup mode on or off. off, a mark half drawn, text half typed and the selection are dropped.
    pub(super) fn toggle(&mut self) {
        (self.on, self.drawing, self.writing) = (!self.on, None, None);
        (self.selected, self.held, self.menu) = (None, None, None);
    }

    /// escape while text is typed: the text goes, and markup mode stays. false when none was being typed.
    pub(super) fn drop_text(&mut self) -> bool {
        self.writing.take().is_some()
    }

    /// closes the selected mark's menu. false when it was not open.
    pub(super) fn close_menu(&mut self) -> bool {
        self.menu.take().is_some()
    }

    /// lets go of the mark selected: its frame and its place among the frame's marks, when one was.
    pub(super) fn let_go(&mut self) -> Option<(usize, usize)> {
        (self.held, self.menu) = (None, None);
        self.selected.take().map(|(frame, index, _)| (frame, index))
    }

    /// escape: lets go of the mark selected and of the group selected. false when neither was.
    pub(super) fn let_go_all(&mut self) -> bool {
        self.let_go().is_some() | (self.group.take().is_some() | self.grip.take().is_some())
    }

    /// the group selected, by its place among the clip's groups
    pub(super) fn group(&self) -> Option<usize> {
        self.group.map(|(group, _)| group)
    }

    /// the group selected has this range now: its keys set it, and it stays selected
    pub(super) fn group_ranged(&mut self, frames: (usize, usize)) {
        if let Some((_, was)) = &mut self.group {
            *was = frames;
        }
    }

    /// true once a pass while the pointer is on an end of a group's bar
    pub(super) fn over_end(&mut self) -> bool {
        std::mem::take(&mut self.at_end)
    }

    /// before a pass over a clip: the group a mark was just drawn into is selected, unless the one selected took it;
    /// and a group selected is let go when the clip's groups are no longer as they were when it was taken.
    pub(super) fn settle(&mut self, model: &Model) {
        let groups = model.marks.map_or(&[][..], Marks::groups);
        if let Some(frame) = self.adopt.take() {
            let home = model.marks.and_then(|marks| marks.home(frame, self.group()));
            self.group = home.map(|group| (group, groups[group].frames));
        }
        self.group = self.group.filter(|(group, frames)| groups.get(*group).is_some_and(|at| at.frames == *frames));
        if self.group.is_none() {
            self.grip = None;
        }
    }

    /// selects the mark at `index` among those on the frame on screen, and with it its group
    fn take(&mut self, model: &Model, index: usize, mark: &Mark) {
        self.selected = Some((model.frame, index, mark.clone()));
        let group = model.marks.and_then(|marks| Some((marks.group_of(model.frame, index)?, marks)));
        self.group = group.map(|(group, marks)| (group, marks.groups()[group].frames));
    }

    pub(super) fn selects(&self) -> bool {
        self.selected.is_some()
    }

    /// a drag on the picture draws: every tool but select
    pub(super) fn draws(&self) -> bool {
        self.tool != Tool::Select
    }

    /// what was typed becomes a mark, unless it is only spaces. text put back in the field by its menu is kept in its
    /// place, or taken off when nothing is left of it.
    fn keep(&mut self, writing: Writing, actions: &mut Vec<Action>) {
        let mark = Mark { shape: Shape::Text(writing.at, writing.text.trim().to_string(), writing.size), ink: writing.ink };
        let kept = (!mark.tiny(1.0)).then_some(mark);
        match writing.replaces {
            Some(index) => actions.push(Action::Change(writing.frame, index, kept)),
            None => {
                self.adopt = kept.is_some().then_some(writing.frame);
                actions.extend(kept.map(|mark| Action::Mark(writing.frame, mark)));
            }
        }
    }

    /// the mark being drawn as it is shown and kept: a pen's line smoothed by `smoothing` from the points as the pointer
    /// gave them, never from points smoothed before, which would melt the line as it is drawn.
    fn drawn(&self, model: &Model, scale: f32, smoothing: f32) -> Option<(usize, Mark)> {
        let (frame, mark) = self.drawing.as_ref()?;
        let Shape::Pen(points) = &mark.shape else { return Some((*frame, mark.clone())) };
        let place = placing(model);
        let on_screen = |p: Point| {
            let (x, y) = place.to_window(p);
            (x / scale, y / scale)
        };
        Some((*frame, Mark { shape: Shape::Pen(smooth(points, smoothing, on_screen)), ink: mark.ink }))
    }

    /// the picture's pointer, in markup mode: a drag on the picture draws with the tool, and with the text tool a click
    /// opens a field there; the select tool takes hold of a mark. a right click on a mark, with any tool, selects it and
    /// opens its menu: true then, and the player's menu stays shut. a press on the picture keeps what was being typed.
    /// nothing else: the window does not move, the picture does not pan and the controls are not toggled. `smoothing`
    /// is how much a pen's line is smoothed.
    pub(super) fn pointer(&mut self, ui: &egui::Ui, picture: &egui::Response, model: &Model, scale: f32, smoothing: f32, actions: &mut Vec<Action>) -> bool {
        let place = placing(model);
        let (x, y, w, h) = model.picture;
        let on_picture = |p: &Pos2| (x..x + w).contains(&(p.x * scale)) && (y..y + h).contains(&(p.y * scale));
        let file = |p: Pos2| place.to_file((p.x * scale, p.y * scale));
        let on = model.marks.map_or_else(Vec::new, |marks| marks.on(model.frame));
        // a selection lasts while its mark is as it was, on the frame on screen
        self.selected = self.selected.take().filter(|(frame, index, mark)| *frame == model.frame && on.get(*index) == Some(&mark));
        if self.selected.is_none() {
            (self.held, self.menu) = (None, None);
        }
        let seen: Vec<Seen> = on.iter().map(|mark| Seen::of(ui.painter(), mark, &place, scale)).collect();
        if ui.input(|i| i.pointer.primary_pressed()) && picture.hovered() {
            if let Some(writing) = self.writing.take() {
                self.keep(writing, actions);
            }
            // a drawing tool lets go of a mark a right click selected: delete and backspace are theirs again
            if self.draws() {
                (self.selected, self.menu) = (None, None);
            }
        }
        let mut asked = false;
        if picture.secondary_clicked() {
            if let Some((at, index)) = picture.interact_pointer_pos().and_then(|at| Some((at, pick(&seen, at)?))) {
                self.take(model, index, on[index]);
                (self.held, self.menu, asked) = (None, Some(at), true);
            }
        }
        if self.tool == Tool::Select {
            self.select(ui, picture, &place, scale, (model, &on), &seen, actions);
        }
        if picture.drag_started_by(egui::PointerButton::Primary) && self.draws() && self.tool != Tool::Text {
            let origin = ui.input(|i| i.pointer.press_origin());
            if let Some(origin) = origin.filter(on_picture) {
                self.drawing = Some((model.frame, Mark::start(self.tool, file(origin), INKS[self.ink])));
            }
        }
        if let (Some((_, mark)), Some(at)) = (&mut self.drawing, picture.interact_pointer_pos()) {
            let mut at = (at.x * scale, at.y * scale);
            // squared in the window, from where its first corner is there now, so it is square as it is seen
            if let (true, Shape::Rectangle(from, _) | Shape::Circle(from, _)) = (self.shift, &mark.shape) {
                at = square(place.to_window(*from), at);
            }
            mark.reach(place.to_file(at));
        }
        if picture.drag_stopped() {
            // how many of the design's pixels one of the file's takes on screen
            let per = w / model.shown_size.0.max(1) as f32 / scale;
            if let Some((frame, mark)) = self.drawn(model, scale, smoothing).filter(|(_, mark)| !mark.tiny(per)) {
                actions.push(Action::Mark(frame, mark));
                self.adopt = Some(frame);
            }
            self.drawing = None;
        }
        if self.tool == Tool::Text && picture.clicked_by(egui::PointerButton::Primary) {
            if let Some(at) = picture.interact_pointer_pos().filter(on_picture) {
                self.writing = Some(Writing { frame: model.frame, at: file(at), field: at, text: String::new(), fresh: true, replaces: None, size: 1.0, ink: INKS[self.ink] });
            }
        }
        asked
    }

    /// the select tool on the marks `on` the frame on screen, `seen` where they are drawn: a press on a corner of the
    /// selected mark's bounds takes hold of it to scale the mark about the opposite corner, a press on a mark selects
    /// it and takes hold of it to move it, and a press on no mark lets go. the drag changes the mark as it goes, and
    /// the release keeps the change, with the mark still selected.
    fn select(&mut self, ui: &egui::Ui, picture: &egui::Response, place: &Placing, scale: f32, (model, on): (&Model, &[&Mark]), seen: &[Seen], actions: &mut Vec<Action>) {
        if let Some(at) = ui.input(|i| i.pointer.press_origin()).filter(|_| ui.input(|i| i.pointer.primary_pressed()) && picture.hovered()) {
            let corner = self.selected.as_ref().and_then(|(_, index, _)| corners(grip_box(&seen[*index])).iter().position(|c| c.distance(at) <= REACH));
            self.held = match (corner, pick(seen, at)) {
                (Some(_), _) => self.selected.as_ref().map(|(_, index, _)| Held { corner, from: at, bounds: grip_box(&seen[*index]), now: None }),
                (None, Some(index)) => {
                    self.take(model, index, on[index]);
                    Some(Held { corner: None, from: at, bounds: grip_box(&seen[index]), now: None })
                }
                (None, None) => {
                    (self.selected, self.group) = (None, None);
                    None
                }
            };
        }
        let (Some(held), Some((_, _, was))) = (&mut self.held, &self.selected) else { return };
        if picture.dragged_by(egui::PointerButton::Primary) {
            if let Some(now) = picture.interact_pointer_pos() {
                held.now = Some(edited(place, scale, held, was, now, self.shift));
            }
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            if let (Some(Held { now: Some(now), .. }), Some((frame, index, was))) = (self.held.take(), &mut self.selected) {
                if now != *was {
                    actions.push(Action::Change(*frame, *index, Some(now.clone())));
                    *was = now;
                }
            }
        }
    }

    /// the marks that show on the frame on screen (those of every group whose range holds it), and the one being
    /// drawn on it, over the picture and clipped to it: the one selected as a drag has it so far, with its bounds and
    /// their handles; and not the text in the field.
    pub(super) fn marks(&self, ui: &egui::Ui, model: &Model, scale: f32, smoothing: f32) {
        let on = model.marks.map_or_else(Vec::new, |marks| marks.on(model.frame));
        let (x, y, w, h) = model.picture;
        let painter = ui.painter().with_clip_rect(Rect::from_min_size(Pos2::new(x / scale, y / scale), Vec2::new(w / scale, h / scale)));
        let place = placing(model);
        let selected = self.selected.as_ref().filter(|(frame, ..)| *frame == model.frame).map(|(_, index, mark)| (*index, self.held.as_ref().and_then(|held| held.now.as_ref()).unwrap_or(mark)));
        let editing = self.writing.as_ref().filter(|writing| writing.frame == model.frame).and_then(|writing| writing.replaces);
        let shown = on.iter().enumerate().filter(|(index, _)| editing != Some(*index)).map(|(index, mark)| selected.filter(|(at, _)| *at == index).map_or(*mark, |(_, now)| now));
        let drawing = self.drawn(model, scale, smoothing).filter(|(frame, _)| *frame == model.frame).map(|(_, mark)| mark);
        draw_marks(&painter, shown.chain(drawing.as_ref()), &place, scale);
        if let Some((_, mark)) = selected {
            let grips = grip_box(&Seen::of(&painter, mark, &place, scale));
            ui.painter().rect_stroke(grips, 0.0, (1.0, SOLID), egui::StrokeKind::Outside);
            ui.painter().rect_stroke(grips, 0.0, (1.0, CHOSEN), egui::StrokeKind::Middle);
            for corner in corners(grips) {
                let handle = Rect::from_center_size(corner, Vec2::splat(HANDLE));
                ui.painter().rect_filled(handle, 0.0, CHOSEN);
                ui.painter().rect_stroke(handle, 0.0, (1.0, SOLID), egui::StrokeKind::Outside);
            }
        }
    }

    /// the selected mark's menu, at the right click that opened it and kept inside `full`: delete, edit text for a
    /// text, own range for a mark that shares its group, then the inks. a press anywhere else closes it, as a choice
    /// does. what it does is a change to the clip's notes, which undo takes back.
    pub(super) fn mark_menu(&mut self, ui: &egui::Ui, full: Rect, model: &Model, scale: f32, actions: &mut Vec<Action>) {
        let (Some(at), Some((frame, index, mark))) = (self.menu, self.selected.clone()) else {
            self.menu = None;
            return;
        };
        if ui.input(|i| i.pointer.any_pressed()) && !ui.ctx().pointer_latest_pos().is_some_and(|p| self.menu_rect.is_some_and(|r| r.contains(p))) {
            self.menu = None;
            return;
        }
        enum Chosen {
            Delete,
            EditText,
            OwnRange,
            Ink([u8; 3]),
        }
        // a mark that shares its group can have a range of its own: the row is there for it
        let shares = model.marks.and_then(|marks| Some(&marks.groups()[marks.group_of(frame, index)?])).filter(|group| group.marks.len() > 1).map(|group| group.frames);
        let mut chosen = None;
        let last = self.menu_rect.unwrap_or(Rect::NOTHING);
        let corner = Pos2::new(at.x.min(full.right() - 8.0 - last.width()).max(full.left() + 8.0), at.y.min(full.bottom() - 8.0 - last.height()).max(full.top() + 8.0));
        self.menu_rect = Some(widgets::menu(ui, "mark-menu", corner, Align2::LEFT_TOP, MENU_WIDTH, 1.0, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            // what can be done to the mark, a row each
            if menu_row(ui, "delete", Some("del")) {
                chosen = Some(Chosen::Delete);
            }
            if matches!(mark.shape, Shape::Text(..)) && menu_row(ui, "edit text", None) {
                chosen = Some(Chosen::EditText);
            }
            if shares.is_some() && menu_row(ui, "own range", None) {
                chosen = Some(Chosen::OwnRange);
            }
            ui.add_space(10.0);
            widgets::heading(ui, "color", &[]);
            ui.add_space(4.0);
            let line = widgets::row(ui, MENU_ROW);
            for (n, rgb) in INKS.iter().enumerate() {
                let cell = Rect::from_min_size(Pos2::new(line.left() + n as f32 * (SWATCH + TOOLS_GAP), line.top()), Vec2::new(SWATCH, MENU_ROW));
                swatch(ui.painter(), cell.center(), *rgb, *rgb == mark.ink);
                if ui.interact(cell, ui.scope_id().with(("mark-ink", n)), Sense::click()).clicked() {
                    chosen = Some(Chosen::Ink(*rgb));
                }
            }
        }));
        match chosen {
            Some(Chosen::Delete) => {
                actions.push(Action::Change(frame, index, None));
                self.selected = None;
            }
            Some(Chosen::Ink(rgb)) => {
                let inked = Mark { ink: rgb, ..mark };
                actions.push(Action::Change(frame, index, Some(inked.clone())));
                self.selected = Some((frame, index, inked));
            }
            // it is the last of the frame's marks then, in a group after every other, and both stay selected
            Some(Chosen::OwnRange) => {
                if let (Some(frames), Some(marks)) = (shares, model.marks) {
                    actions.push(Action::OwnRange(frame, index));
                    self.selected = Some((frame, marks.on(frame).len() - 1, mark));
                    self.group = Some((marks.groups().len(), frames));
                }
            }
            Some(Chosen::EditText) => {
                if let Shape::Text(at, text, size) = mark.shape {
                    let (x, y) = placing(model).to_window(at);
                    let field = Pos2::new(x / scale, y / scale);
                    self.writing = Some(Writing { frame, at, field, text, fresh: true, replaces: Some(index), size, ink: mark.ink });
                    self.selected = None;
                }
            }
            None => return,
        }
        self.menu = None;
    }

    /// the field text is typed into, where the click was and kept inside `full`. enter keeps what was typed and escape
    /// drops it; the field losing the keyboard otherwise keeps it too.
    pub(super) fn field(&mut self, ui: &egui::Ui, full: Rect, actions: &mut Vec<Action>) {
        let Some(writing) = &mut self.writing else { return };
        let color = ink(writing.ink);
        let at = Pos2::new(writing.field.x.clamp(full.left() + 6.0, full.right() - FIELD_WIDTH), writing.field.y.clamp(full.top() + 6.0, full.bottom() - 40.0));
        let mut done = None;
        egui::Area::new(egui::Id::unique("markup-text")).order(egui::Order::Foreground).fixed_pos(at).show(ui.ctx(), |ui| {
            // a light field for an ink too dark for the dark one, as the text's box has
            let back = if text_back(writing.ink) == TEXT_BACK { FIELD_BACK } else { TEXT_BACK_LIGHT };
            egui::Frame::new().fill(back).stroke((1.0, Color32::WHITE)).inner_margin(egui::Margin::symmetric(6, 5)).show(ui, |ui| {
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

    /// the row of tools at the top middle of the picture while markup mode is on (`.pl-mk`): select and the four tools,
    /// the inks, undo, redo and clear, then save image and done. `top` is its top edge.
    pub(super) fn tools(&mut self, ui: &mut egui::Ui, middle: f32, top: f32, actions: &mut Vec<Action>) {
        enum Item {
            Pick(Tool, Icon),
            Ink(usize),
            Rule,
            Undo,
            Redo,
            Clear,
            Words(&'static str),
        }
        use Item::*;
        let tools = [Pick(Tool::Select, Icon::Select), Pick(Tool::Rectangle, Icon::Rectangle), Pick(Tool::Circle, Icon::Circle), Pick(Tool::Pen, Icon::Pen), Pick(Tool::Text, Icon::Text), Rule];
        let rest = [Rule, Undo, Redo, Clear, Rule, Words("save image"), Words("done")];
        let items: Vec<Item> = tools.into_iter().chain((0..INKS.len()).map(Ink)).chain(rest).collect();
        let painter = ui.painter().clone();
        let words = |text: &str| painter.layout_no_wrap(text.to_string(), font(SANS, 12.0), FG);
        let wide = |item: &Item| match item {
            Pick(..) | Undo | Redo | Clear => TOOL,
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
                Redo => icons::draw(&painter, rect, Icon::Redo, FG),
                Clear => icons::draw(&painter, rect, Icon::Close, FG),
                // with a mark selected, its ink is the one lit
                Ink(n) => swatch(&painter, rect.center(), INKS[*n], self.selected.as_ref().map_or(*n == self.ink, |(.., mark)| mark.ink == INKS[*n])),
                Words(text) => {
                    painter.galley(Pos2::new(rect.left() + 8.0, text_top(&painter, font(SANS, 12.0), rect.center().y)), words(text), FG);
                }
                Rule => {}
            }
            if response.clicked() {
                match item {
                    Pick(tool, _) => self.tool = *tool,
                    Ink(n) => {
                        self.ink = *n;
                        // and the mark selected takes it, as its own menu's colors do
                        if let Some((frame, index, mark)) = self.selected.take() {
                            let inked = Mark { ink: INKS[*n], ..mark };
                            actions.push(Action::Change(frame, index, Some(inked.clone())));
                            self.selected = Some((frame, index, inked));
                        }
                        if let Some(writing) = &mut self.writing {
                            writing.ink = INKS[*n];
                        }
                    }
                    // these go the way their keys do
                    Undo => actions.push(Action::Key(Key::Z, keys::CTRL)),
                    Redo => actions.push(Action::Key(Key::Z, keys::CTRL_SHIFT)),
                    Words("done") => actions.push(Action::Key(Key::E, keys::NONE)),
                    Clear => actions.push(Action::ClearMarks),
                    Words(_) => actions.push(Action::Shot(Shot::Frame)),
                    Rule => {}
                }
            }
        }
    }
}

/// the mark `was` as a hold on it has it with the pointer at `now`: moved as far as the pointer went, or scaled about
/// the corner of its bounds across from the one held, each way on its own or with `keep` (shift) in proportion. text
/// is always scaled in proportion, its letters with it, and is never turned over.
fn edited(place: &Placing, scale: f32, held: &Held, was: &Mark, now: Pos2, keep: bool) -> Mark {
    let at = |p: Pos2| (p.x * scale, p.y * scale);
    let Some(corner) = held.corner else {
        let by = (now - held.from) * scale;
        return place.on_screen(was, |p| (p.0 + by.x, p.1 + by.y));
    };
    let ends = corners(held.bounds);
    let (about, from) = (at(ends[(corner + 2) % 4]), at(ends[corner]));
    let mut by = scale_by(about, from, at(now), keep);
    if let Shape::Text(_, _, size) = was.shape {
        let grown = (size * by.0.abs().max(by.1.abs())).clamp(TEXT_SIZES.0, TEXT_SIZES.1);
        by = (grown / size, grown / size);
    }
    let mut mark = place.on_screen(was, |p| (about.0 + (p.0 - about.0) * by.0, about.1 + (p.1 - about.1) * by.1));
    if let Shape::Text(_, _, size) = &mut mark.shape {
        *size *= by.0;
    }
    mark
}

impl Markup {
    /// the clip's groups on the timebar: each a thin bar just over the track from its in frame to its out, in a lane
    /// of its own where it overlaps another, white as the trim's markers are; the one selected brighter, with a stem
    /// down to the track at each end. a press on a bar selects its group; a drag on an end of it moves that end, and
    /// on the rest of it the whole range, as far as the pointer goes; the release keeps the range. the bars stand
    /// over the timebar, so a press on one goes nowhere in the clip. `track` is the track as drawn and `last` the
    /// frame at the timebar's right end.
    pub(super) fn bars(&mut self, ui: &egui::Ui, area: Rect, track: Rect, last: f32, marks: &Marks, actions: &mut Vec<Action>) {
        let painter = ui.painter();
        let x_of = |frame: usize| area.left() + area.width() * (frame as f32 / last).min(1.0);
        let frame_at = |x: f32| (((x - area.left()) / area.width()).clamp(0.0, 1.0) * last).round() as usize;
        let lane_of = lanes(marks.groups(), LANES);
        let highest = lane_of.iter().copied().max().unwrap_or(0);
        let down = ui.input(|i| i.pointer.primary_down());
        for (i, group) in marks.groups().iter().enumerate() {
            let frames = self.grip.as_ref().filter(|grip| grip.group == i).map_or(group.frames, |grip| grip.now);
            let top = area.bottom() - TRACK_THICK - 1.0 - BAR_HIGH - LANE * lane_of[i] as f32;
            let bar = Rect::from_min_max(Pos2::new(x_of(frames.0) - 1.0, top), Pos2::new(x_of(frames.1) + 1.0, top + BAR_HIGH));
            // the room a press has: the lane's own height, down to the track from the lowest, and up to the timebar's
            // top from a lane with none over it
            let over = if lane_of[i] == highest && highest == 0 { area.top() } else { top - 1.0 };
            let under = if lane_of[i] == 0 { top + BAR_HIGH + 1.0 } else { top + BAR_HIGH };
            let room = Rect::from_min_max(Pos2::new(bar.left() - END_REACH, over), Pos2::new(bar.right() + END_REACH, under));
            let response = ui.interact(room, ui.scope_id().with(("group-bar", i)), Sense::click_and_drag());
            let end_at = |x: f32| match ((x - bar.left()).abs() <= END_REACH, (x - bar.right()).abs() <= END_REACH) {
                (true, true) if x - bar.left() <= bar.right() - x => Part::In,
                (true, false) => Part::In,
                (_, true) => Part::Out,
                (false, false) => Part::Whole,
            };
            if down && response.is_pointer_button_down_on() && self.grip.is_none() {
                if let Some(at) = ui.input(|i| i.pointer.press_origin()) {
                    (self.group, self.grip) = (Some((i, group.frames)), Some(Grip { group: i, part: end_at(at.x), from: frame_at(at.x), was: group.frames, now: group.frames }));
                    // a mark of another group is let go
                    self.selected = self.selected.take().filter(|(frame, index, _)| marks.group_of(*frame, *index) == Some(i));
                    (self.held, self.menu) = (None, None);
                }
            }
            if let Some(grip) = self.grip.as_mut().filter(|grip| grip.group == i) {
                if let Some(at) = response.interact_pointer_pos().filter(|_| down) {
                    grip.now = dragged(grip.was, grip.part, grip.from, frame_at(at.x), last as usize);
                }
                self.at_end |= grip.part != Part::Whole;
                if !down {
                    if grip.now != grip.was {
                        actions.push(Action::Range(i, grip.now));
                        self.group = Some((i, grip.now));
                    }
                    self.grip = None;
                }
            } else if let Some(at) = response.hover_pos() {
                self.at_end |= end_at(at.x) != Part::Whole;
            }
            if self.group() == Some(i) {
                painter.rect_filled(bar, 0.0, Color32::WHITE);
                for x in [bar.left(), bar.right() - 2.0] {
                    painter.rect_filled(Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + 2.0, track.top())), 0.0, Color32::WHITE);
                }
            } else {
                painter.rect_filled(bar, 0.0, FILL);
            }
        }
    }

    /// what the chip of the group selected says, as the trim's says its points: where the group starts and ends,
    /// counted as the readout counts, and how many frames it shows on, both its ends among them. a drag says it as
    /// it goes.
    pub(super) fn group_chip(&self, model: &Model, timecode: bool) -> Option<Vec<(String, Color32)>> {
        let (group, frames) = self.group?;
        let (first, last) = self.grip.as_ref().filter(|grip| grip.group == group).map_or(frames, |grip| grip.now);
        let point = |frame: usize| stamp(frame, model.first, model.clock, timecode).1;
        Some(vec![(format!("group · {} ", point(first)), FG), ("to".to_string(), DIM), (format!(" {}", point(last)), FG), (format!(" · {} f", last + 1 - first), DIM)])
    }
}

/// what is said of notes read from a file that says they were made on another version of the clip's file (its size
/// or its time was another then): they are shown, labeled, until a change makes them this version's.
pub(super) fn earlier_chip(model: &Model) -> Option<Vec<(String, Color32)>> {
    let earlier = model.marks.is_some_and(|marks| marks.earlier && !marks.groups().is_empty());
    earlier.then(|| vec![("notes made on an earlier version of this file".to_string(), FG)])
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
            Shape::Text(at, text, size) => {
                let at = to(at);
                let (back, galley) = text_box(painter, at, text, *size, color);
                painter.rect_filled(back, 0.0, text_back(mark.ink));
                painter.galley(Pos2::new(at.x, text_top(painter, font(TEXT_FONT, TEXT_SIZE * size), back.center().y)), galley, color);
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
    use super::super::tests::{drive, drive_on, drive_with, model};
    use super::super::{Extras, Ui};
    use super::*;
    use crate::event::Button::{self, Left};
    use crate::event::Mods;
    use crate::view::Look;

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

    fn boxed(a: (f32, f32), b: (f32, f32)) -> Mark {
        Mark { shape: Shape::Rectangle(file(a), file(b)), ink: INKS[0] }
    }

    fn right_click(at: (f32, f32)) -> Vec<Step> {
        vec![(at.0, at.1, None), (at.0, at.1, Some((Button::Right, true))), (at.0, at.1, Some((Button::Right, false))), (at.0, at.1, None)]
    }

    /// `f` with a painter that has the player's fonts, as a pass of the controls has it
    fn with_painter(f: impl FnOnce(&egui::Painter)) {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut f = Some(f);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f.take().into_iter().for_each(|f| f(ui.painter())));
        out.textures_delta.clear();
    }

    /// which mark a press takes: the last drawn whose line passes within 6 of the design's pixels of it, or whose text
    /// box holds it; else the smallest whose bounds hold it, so a circle drawn inside a box is had from inside it;
    /// else none.
    #[test]
    fn a_press_takes_the_mark_under_it() {
        let marks = [
            boxed((200.0, 100.0), (400.0, 300.0)),
            Mark { shape: Shape::Circle(file((260.0, 160.0)), file((340.0, 240.0))), ink: INKS[0] },
            Mark { shape: Shape::Text(file((440.0, 40.0)), "note".into(), 1.0), ink: INKS[0] },
            Mark { shape: Shape::Pen(vec![file((180.0, 106.0)), file((300.0, 104.0)), file((420.0, 106.0))]), ink: INKS[0] },
        ];
        with_painter(|painter| {
            let seen: Vec<Seen> = marks.iter().map(|mark| Seen::of(painter, mark, &placing(&model()), 1.0)).collect();
            for (at, taken) in [((203.0, 200.0), Some(0)), ((300.0, 158.0), Some(1)), ((300.0, 200.0), Some(1)), ((220.0, 280.0), Some(0)), ((445.0, 45.0), Some(2)), ((300.0, 102.0), Some(3)), ((600.0, 330.0), None)] {
                assert_eq!(pick(&seen, Pos2::new(at.0, at.1)), taken, "a press at {at:?}");
            }
        });
    }

    /// the select tool: a press on a mark selects it and a drag from there moves it, as far as the pointer went on
    /// screen; a corner of its bounds dragged scales it about the opposite corner, each way on its own, or with shift
    /// both by the larger. only the mark taken changes, and it stays selected as changed.
    #[test]
    fn the_select_tool_moves_and_scales_the_mark_taken_and_no_other() {
        let mut marks = Marks::default();
        marks.add(0, None, boxed((200.0, 100.0), (260.0, 160.0)), 1, 1);
        marks.add(0, None, boxed((300.0, 200.0), (400.0, 250.0)), 1, 1);
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        ui.markup.tool = Tool::Select;
        // the second box's left side, dragged 40 across and 20 down, up to the release: the player changes the mark
        // before the next pass
        let moved = drive_with(&mut ui, Model { marks: Some(&marks), ..model() }, &drag((300.0, 225.0), (340.0, 245.0))[..5]);
        let [Action::Change(0, 1, Some(mark))] = &moved[..] else { panic!("{moved:?}") };
        assert!(matches!(mark.shape, Shape::Rectangle(a, b) if near(a, file((340.0, 220.0))) && near(b, file((440.0, 270.0)))), "{mark:?}");
        marks.change(0, 1, Some(mark.clone()));
        drive_with(&mut ui, Model { marks: Some(&marks), ..model() }, &[(340.0, 245.0, None)]);
        assert_eq!(ui.markup.selected.as_ref().map(|(f, i, m)| (*f, *i, m.clone())), Some((0, 1, mark.clone())), "still selected, as changed");
        // its bounds stand 4 out from it: the lower right corner, at 444,274, dragged to 494,284, scales it about the
        // upper left one, at 336,216
        for shift in [false, true] {
            let mut ui = Ui::new((640, 360), 1.0, false, None, None);
            ui.toggle_markup();
            ui.markup.tool = Tool::Select;
            ui.keys_held(Mods { shift, ..Mods::default() });
            drive_with(&mut ui, Model { marks: Some(&marks), ..model() }, &click((340.0, 230.0)));
            let scaled = drive_with(&mut ui, Model { marks: Some(&marks), ..model() }, &drag((444.0, 274.0), (494.0, 284.0)));
            let (sx, sy): (f32, f32) = ((494.0 - 336.0) / (444.0 - 336.0), (284.0 - 216.0) / (274.0 - 216.0));
            let (sx, sy) = if shift { (sx.max(sy), sx.max(sy)) } else { (sx, sy) };
            let to = |x: f32, y: f32| file((336.0 + (x - 336.0) * sx, 216.0 + (y - 216.0) * sy));
            let [Action::Change(0, 1, Some(Mark { shape: Shape::Rectangle(a, b), .. }))] = &scaled[..] else { panic!("shift {shift}: {scaled:?}") };
            assert!(near(*a, to(340.0, 220.0)) && near(*b, to(440.0, 270.0)), "shift {shift}: {a:?} {b:?}, wanted {:?} {:?}", to(340.0, 220.0), to(440.0, 270.0));
        }
    }

    /// with a mark selected, an ink in the row of tools recolors it (the owner's word, 2026-11-26), and is the ink of
    /// the marks drawn next. the row's inks start past its five tools and a rule: 1 + 3 in, 5 × 32, 9 + 2
    #[test]
    fn an_ink_in_the_row_recolors_the_mark_selected() {
        let mut marks = Marks::default();
        let text = Mark { shape: Shape::Text(file((200.0, 100.0)), "note".into(), 1.0), ink: INKS[0] };
        marks.add(0, None, text.clone(), 1, 1);
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        ui.markup.tool = Tool::Select;
        drive_with(&mut ui, Model { marks: Some(&marks), ..model() }, &click((210.0, 105.0)));
        let row = ui.markup.row.expect("the row of tools");
        let swatch = |n: usize| (row.left() + 4.0 + 5.0 * 32.0 + 11.0 + n as f32 * 20.0 + 9.0, row.top() + 4.0 + 14.0);
        let inked = drive_with(&mut ui, Model { marks: Some(&marks), ..model() }, &click(swatch(8)));
        assert_eq!(inked, vec![Action::Change(0, 0, Some(Mark { ink: INKS[8], ..text }))]);
        assert_eq!(INKS[ui.markup.ink], INKS[8]);
    }

    /// text sits on a box that every ink reads against at 4.5:1 or better: dark, or light for the inks too dark for it
    #[test]
    fn every_ink_reads_on_its_texts_box() {
        for rgb in INKS {
            let back = text_back(rgb);
            let back = [back.r(), back.g(), back.b()].map(|c| (c as f32 / back.a().max(1) as f32 * 255.0).round() as u8);
            assert!(contrast(rgb, back) >= 4.5, "{rgb:?} on {back:?}: {}", contrast(rgb, back));
        }
    }

    /// a text's corner dragged grows its letters, in proportion whatever shift says, about the opposite corner
    #[test]
    fn a_texts_corner_grows_its_letters() {
        let mut marks = Marks::default();
        let text = Mark { shape: Shape::Text(file((200.0, 100.0)), "note".into(), 1.0), ink: INKS[0] };
        marks.add(0, None, text.clone(), 1, 1);
        let mut grips = Rect::NOTHING;
        with_painter(|painter| grips = grip_box(&Seen::of(painter, &text, &placing(&model()), 1.0)));
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        ui.markup.tool = Tool::Select;
        drive_with(&mut ui, Model { marks: Some(&marks), ..model() }, &click((210.0, 105.0)));
        // twice as wide, no taller: the larger, twice, both ways
        let corner = grips.right_bottom();
        let grown = drive_with(&mut ui, Model { marks: Some(&marks), ..model() }, &drag((corner.x, corner.y), (corner.x + grips.width(), corner.y)));
        let [Action::Change(0, 0, Some(Mark { shape: Shape::Text(at, words, size), .. }))] = &grown[..] else { panic!("{grown:?}") };
        let held = grips.left_top();
        assert!(words == "note" && (*size - 2.0).abs() < 1e-4, "{grown:?}");
        assert!(near(*at, file((held.x + (200.0 - held.x) * 2.0, held.y + (100.0 - held.y) * 2.0))), "{at:?}");
    }

    /// delete takes off the mark selected (the player asks for it, with the key); escape lets go of the selection
    /// first, then ends markup mode; a press where there is no mark lets go too.
    #[test]
    fn delete_takes_the_mark_selected_and_escape_lets_go_of_it_first() {
        let mut marks = Marks::default();
        marks.add(0, None, boxed((200.0, 100.0), (260.0, 160.0)), 1, 1);
        marks.add(0, None, boxed((300.0, 200.0), (400.0, 250.0)), 1, 1);
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        ui.markup.tool = Tool::Select;
        let pass = |ui: &mut Ui, steps: &[Step]| drive_with(ui, Model { marks: Some(&marks), ..model() }, steps);
        assert!(pass(&mut ui, &click((300.0, 225.0))).is_empty() && ui.mark_selected());
        assert_eq!(ui.take_selected_mark(), Some((0, 1)), "delete");
        assert!(!ui.mark_selected());
        pass(&mut ui, &click((200.0, 130.0)));
        assert!(ui.mark_selected());
        pass(&mut ui, &click((560.0, 200.0)));
        assert!(!ui.mark_selected(), "a press on no mark lets go");
        pass(&mut ui, &click((200.0, 130.0)));
        assert!(ui.escape() && !ui.mark_selected() && ui.markup(), "escape lets go first");
        assert!(ui.escape() && !ui.markup(), "then ends markup mode");
        assert_eq!(ui.take_selected_mark(), None);
    }

    /// a mark a right click selected with a drawing tool is let go once that tool draws, so delete and backspace are
    /// theirs again: backspace is 1× speed, not a delete of a mark out of mind.
    #[test]
    fn drawing_lets_go_of_the_mark_selected() {
        let mut marks = Marks::default();
        marks.add(0, None, boxed((200.0, 100.0), (260.0, 160.0)), 1, 1);
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        let pass = |ui: &mut Ui, steps: &[Step]| drive_with(ui, Model { marks: Some(&marks), ..model() }, steps);
        pass(&mut ui, &right_click((200.0, 130.0)));
        assert!(ui.mark_selected());
        // clear of the mark's menu, which stands at the right click
        let drawn = pass(&mut ui, &drag((150.0, 250.0), (190.0, 290.0)));
        assert!(matches!(&drawn[..], [Action::Mark(0, _)]), "{drawn:?}");
        assert!(!ui.mark_selected() && ui.markup.menu.is_none(), "let go");
    }

    /// a right click on a mark, whatever the tool, selects it and opens its menu in place of the player's: delete
    /// takes it off, an ink colors it, and for text "edit text" puts it back in the field, where enter keeps it in its
    /// place; a box, a circle and a pen's line have delete alone above the inks. a right click off every mark opens
    /// the player's menu.
    #[test]
    fn a_right_click_on_a_mark_opens_its_menu_and_each_item_does_what_it_says() {
        let mut marks = Marks::default();
        let text = Mark { shape: Shape::Text(file((300.0, 200.0)), "note".into(), 1.0), ink: INKS[0] };
        marks.add(0, None, boxed((200.0, 100.0), (260.0, 160.0)), 1, 1);
        marks.add(0, None, text.clone(), 1, 1);
        marks.add(0, None, Mark { shape: Shape::Circle(file((380.0, 150.0)), file((440.0, 210.0))), ink: INKS[0] }, 1, 1);
        marks.add(0, None, Mark { shape: Shape::Pen(vec![file((380.0, 250.0)), file((420.0, 260.0)), file((460.0, 250.0))]), ink: INKS[0] }, 1, 1);
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        let pass = |ui: &mut Ui, steps: &[Step]| drive_with(ui, Model { marks: Some(&marks), ..model() }, steps);
        // the menu's rows start 13 in (its line and padding), 24 high; then 10 clear, the inks' heading 16 high, 4
        // clear, and the inks, a swatch each
        let row = |ui: &Ui, n: f32| {
            let menu = ui.markup.menu_rect.expect("the menu is drawn");
            (menu.left() + 40.0, menu.top() + 13.0 + 24.0 * n + 12.0)
        };
        let swatch = |ui: &Ui, rows: f32, n: f32| {
            let menu = ui.markup.menu_rect.expect("the menu is drawn");
            (menu.left() + 13.0 + n * (SWATCH + TOOLS_GAP) + SWATCH / 2.0, menu.top() + 13.0 + 24.0 * rows + 30.0 + 12.0)
        };
        assert!(pass(&mut ui, &right_click((200.0, 130.0))).is_empty());
        assert!(ui.markup.menu.is_some() && ui.commands.is_none() && ui.mark_selected(), "the box's menu, not the player's");
        assert_eq!(ui.group_selected(), Some(0), "and its group is selected with it");
        let at = row(&ui, 0.0);
        assert_eq!(pass(&mut ui, &click(at)), vec![Action::Change(0, 0, None)], "delete");
        assert!(ui.markup.menu.is_none());
        // the four share a group, so each has "own range" under what else its menu holds: it asks for a group of the
        // mark's own, and the mark is then the last of the frame's, in the clip's second group, both still selected
        pass(&mut ui, &right_click((200.0, 130.0)));
        let at = row(&ui, 1.0);
        assert_eq!(pass(&mut ui, &click(at)[..3]), vec![Action::OwnRange(0, 0)], "own range");
        assert_eq!((ui.markup.selected.as_ref().map(|(frame, index, _)| (*frame, *index)), ui.markup.group), (Some((0, 3)), Some((1, (0, 0)))));
        ui.escape();
        pass(&mut ui, &right_click((200.0, 130.0)));
        let at = swatch(&ui, 2.0, 1.0);
        let colored = pass(&mut ui, &click(at));
        assert_eq!(colored, vec![Action::Change(0, 0, Some(Mark { ink: INKS[1], ..boxed((200.0, 100.0), (260.0, 160.0)) }))], "an ink");
        // text has "edit text" under delete
        pass(&mut ui, &right_click((310.0, 205.0)));
        let at = row(&ui, 1.0);
        assert!(pass(&mut ui, &click(at)).is_empty());
        assert!(ui.typing(), "the field has the keyboard");
        assert_eq!(ui.markup.writing.as_ref().map(|w| w.text.as_str()), Some("note"));
        ui.char('!');
        ui.key(Key::Enter, Mods::default());
        let kept = pass(&mut ui, &[(310.0, 205.0, None)]);
        assert_eq!(kept, vec![Action::Change(0, 1, Some(Mark { shape: Shape::Text(file((300.0, 200.0)), "note!".into(), 1.0), ink: INKS[0] }))]);
        // the menu's height says its rows: the box's, the circle's and the pen's are alike, the text's a row taller
        let height = |ui: &mut Ui, at: (f32, f32)| {
            drive_with(ui, Model { marks: Some(&marks), ..model() }, &right_click(at));
            let height = ui.markup.menu_rect.expect("the menu is drawn").height();
            ui.escape();
            height
        };
        let boxed_height = height(&mut ui, (200.0, 130.0));
        assert_eq!(height(&mut ui, (310.0, 205.0)), boxed_height + 24.0, "text");
        assert_eq!(height(&mut ui, (410.0, 150.0)), boxed_height, "a circle");
        assert_eq!(height(&mut ui, (420.0, 259.0)), boxed_height, "a pen's line");
        for (at, index) in [((410.0, 150.0), 2), ((420.0, 259.0), 3)] {
            pass(&mut ui, &right_click(at));
            let at = row(&ui, 0.0);
            assert_eq!(pass(&mut ui, &click(at)), vec![Action::Change(0, index, None)], "delete, for mark {index}");
        }
        // a mark alone in its group has its own range already: no such row
        let mut alone = Marks::default();
        alone.add(0, None, boxed((200.0, 100.0), (260.0, 160.0)), 1, 1);
        drive_with(&mut ui, Model { marks: Some(&alone), ..model() }, &right_click((200.0, 130.0)));
        assert_eq!(ui.markup.menu_rect.expect("the menu is drawn").height(), boxed_height - 24.0);
        ui.escape();
        // off every mark, the player's own
        pass(&mut ui, &right_click((560.0, 200.0)));
        assert!(ui.markup.menu.is_none() && ui.commands.is_some());
    }

    /// a clip of 150 frames on frame 82, with these notes. its timebar is as wide as the window: frame f is 640 × f
    /// / 149 along it, and a group's bar is 9 to 11 up from the window's bottom edge, a second lane's 12 to 14
    fn clip(marks: &Marks) -> Model<'_> {
        Model { frame: 82, frames: 150, marks: Some(marks), ..model() }
    }

    fn along(frame: usize) -> f32 {
        640.0 * frame as f32 / 149.0
    }

    /// a group of one box, over these frames
    fn grouped(frames: (usize, usize)) -> crate::markup::Group {
        crate::markup::Group { frames, marks: vec![boxed((200.0, 100.0), (260.0, 160.0))] }
    }

    /// a press on a group's bar on the timebar selects the group and goes nowhere in the clip; a drag on an end of
    /// the bar moves that end to the frame under the pointer, and on its middle the whole range as far as the pointer
    /// went; the release asks for the range, once. a press on the track under it goes there as ever, and the group
    /// stays selected; escape lets go of it. all of it with markup mode off or on.
    #[test]
    fn a_groups_bar_is_selected_by_a_press_and_dragged_by_its_ends_and_its_middle() {
        for markup in [false, true] {
            let mut marks = Marks::of(vec![grouped((55, 129))], false);
            let mut ui = Ui::new((640, 360), 1.0, false, None, None);
            if markup {
                ui.toggle_markup();
            }
            // the pointer on the bar brings the controls up: a pass for them to be asked for, one for them to be there
            let up = [(along(90), 350.0, None), (along(90), 350.0, None)];
            drive_with(&mut ui, clip(&marks), &up);
            assert_eq!(ui.group_selected(), None);
            assert_eq!(drive_with(&mut ui, clip(&marks), &click((along(90), 350.0))), vec![], "a press on the bar is no seek");
            assert_eq!(ui.group_selected(), Some(0));
            for (from, to, range) in [(129, 100, (55, 100)), (55, 40, (40, 100)), (70, 80, (50, 110)), (110, 149, (50, 149)), (50, 120, (120, 149))] {
                // up to the release: the player sets the range before the next pass
                let asked = drive_with(&mut ui, clip(&marks), &drag((along(from), 350.0), (along(to), 350.0))[..5]);
                assert_eq!(asked, vec![Action::Range(0, range)], "markup {markup}: a drag from frame {from} to {to}");
                marks.set_range(0, range, 150);
                drive_with(&mut ui, clip(&marks), &up[..1]);
                assert_eq!(ui.markup.group, Some((0, range)), "still selected, as it is now");
            }
            let seeks = drive_with(&mut ui, clip(&marks), &click((along(10), 357.0)));
            assert!(!seeks.is_empty() && seeks.iter().all(|asked| *asked == Action::Seek(10)), "the track is the timebar's: {seeks:?}");
            assert_eq!(ui.group_selected(), Some(0));
            // the groups changed under it (an undo): it is let go
            assert!(marks.undo());
            drive_with(&mut ui, clip(&marks), &up[..1]);
            assert_eq!(ui.group_selected(), None);
            drive_with(&mut ui, clip(&marks), &click((along(120), 350.0)));
            assert!(ui.group_selected() == Some(0) && ui.escape() && ui.group_selected().is_none(), "escape lets go of it");
            assert_eq!(ui.markup(), markup, "and leaves markup mode as it was");
        }
    }

    /// a group's bar and a marker of the trim at one place are each taken in their own rows (the owner's ask for the
    /// markers, 2026-11-27): the bars stand over the markers, and no bar reaches the track. a group that ends on the
    /// trim's out frame and passes over its in: in the bar's lane a press at the out is the group's end and a press
    /// over the in marker its middle, which moves the whole range; on the track under them both presses are the
    /// marker's. the pointer is the two-way arrow on an end and on a marker, not on the bar's middle
    #[test]
    fn a_groups_bar_and_a_trim_marker_at_one_place_are_each_taken_in_their_own_rows() {
        let marks = Marks::of(vec![grouped((40, 129))], false);
        let trimmed = || Model { trim: (Some(55), Some(129)), ..clip(&marks) };
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        let (lane, track) = (350.0, 356.0);
        for (from, to, row, asked) in [(129, 120, lane, Action::Range(0, (40, 120))), (129, 120, track, Action::Trim(Some(55), Some(120))), (55, 45, lane, Action::Range(0, (30, 119))), (55, 45, track, Action::Trim(Some(45), Some(129)))] {
            assert_eq!(drive_with(&mut ui, trimmed(), &drag((along(from), row), (along(to), row))), vec![asked], "a drag from frame {from} to {to}, {row} down");
        }
        for (frame, row, arrow) in [(129, lane, true), (129, track, true), (55, lane, false), (55, track, true), (90, lane, false), (90, track, false)] {
            drive_with(&mut ui, trimmed(), &[(along(frame), row, None)]);
            ui.pointer_move(along(frame), row);
            let mut out = ui.run(&super::super::Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(trimmed()), error: None, opening: None, list: None, browsing: false, fullscreen: false, toast: None, settings: &super::super::tests::DEFAULTS });
            out.textures.clear();
            assert_eq!(out.resize, arrow, "the pointer on frame {frame}, {row} down");
        }
    }

    /// the bars as drawn: 2 high and 1 over the track at its thickest, from a frame before the group's in to a frame
    /// past its out as the timebar's head is 2 wide, in the track's lit gray; a group that overlaps another a lane
    /// higher; the one selected white, with a stem at each end.
    #[test]
    fn a_group_is_a_thin_bar_over_the_track_and_one_that_overlaps_a_lane_higher() {
        let marks = Marks::of(vec![grouped((55, 129)), grouped((100, 140))], false);
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.ctx.tessellation_options_mut(|options| options.feathering = false);
        drive_with(&mut ui, clip(&marks), &[(320.0, 100.0, None)]);
        std::thread::sleep(std::time::Duration::from_millis(200));
        // the corners of what is drawn in a color above the track's own rows, each once
        let drawn = |ui: &mut Ui| {
            ui.pointer_move(320.0, 100.0);
            let mut out = ui.run(&super::super::Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(clip(&marks)), error: None, opening: None, list: None, browsing: false, fullscreen: false, toast: None, settings: &super::super::tests::DEFAULTS });
            out.textures.clear();
            let meshes = out.primitives.iter().filter_map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(mesh) => Some(mesh),
                _ => None,
            });
            let corners: Vec<(Color32, (i32, i32))> = meshes.flat_map(|mesh| &mesh.vertices).filter(|v| (340.0..352.0).contains(&v.pos.y)).map(|v| (v.color, (v.pos.x.round() as i32, v.pos.y.round() as i32))).collect();
            move |color: Color32| {
                let mut of: Vec<(i32, i32)> = corners.iter().filter(|(c, _)| *c == color).map(|(_, at)| *at).collect();
                of.sort();
                of.dedup();
                of
            }
        };
        let plain = drawn(&mut ui);
        assert_eq!(plain(super::super::FILL), [(235, 349), (235, 351), (429, 346), (429, 348), (555, 349), (555, 351), (602, 346), (602, 348)]);
        assert_eq!(plain(Color32::WHITE), [], "nothing selected, and the head starts lower");
        ui.markup.group = Some((1, (100, 140)));
        let selected = drawn(&mut ui);
        assert_eq!(selected(super::super::FILL), [(235, 349), (235, 351), (555, 349), (555, 351)]);
        assert_eq!(selected(Color32::WHITE), [(429, 346), (429, 348), (431, 346), (600, 346), (602, 346), (602, 348)], "its bar, and the top of a stem at each end");
    }

    /// a mark drawn is its group's: the group it went into is selected once it is there, a group the next mark joins
    /// while its range holds the frame, whichever group is newer. drawing does not let go of the group, and the chip
    /// says its range as the trim's says its points.
    #[test]
    fn a_mark_drawn_selects_its_group_and_the_next_joins_the_group_selected() {
        let mut marks = Marks::of(vec![grouped((0, 100)), grouped((60, 120))], false);
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        let drawn = drive_with(&mut ui, clip(&marks), &drag((300.0, 100.0), (360.0, 160.0)));
        let [Action::Mark(82, mark)] = &drawn[..] else { panic!("{drawn:?}") };
        // as the player does it: into the group selected, or the newest that covers the frame
        marks.add(82, ui.group_selected(), mark.clone(), 180, 150);
        assert_eq!(marks.groups()[1].marks.len(), 2, "no group was selected: the newest that covers frame 82");
        drive_with(&mut ui, clip(&marks), &[(300.0, 200.0, None)]);
        assert_eq!(ui.markup.group, Some((1, (60, 120))), "the group it went into is selected");
        // the older group selected, on its bar: the next mark joins it, and it stays selected through the drawing
        ui.markup.group = Some((0, (0, 100)));
        let drawn = drive_with(&mut ui, clip(&marks), &drag((300.0, 200.0), (360.0, 260.0)));
        let [Action::Mark(82, mark)] = &drawn[..] else { panic!("{drawn:?}") };
        marks.add(82, ui.group_selected(), mark.clone(), 180, 150);
        assert_eq!((marks.groups()[0].marks.len(), marks.groups()[1].marks.len()), (2, 2));
        drive_with(&mut ui, clip(&marks), &[(300.0, 200.0, None)]);
        assert_eq!(ui.group_selected(), Some(0));
        let said: String = ui.markup.group_chip(&clip(&marks), false).expect("a chip").into_iter().map(|(words, _)| words).collect();
        assert_eq!(said, "group · 000000 to 000100 · 101 f");
        // shift with [ and ] are the group's in markup mode, and the trim's outside it
        assert_eq!(ui.group_for_keys(), Some(0));
        ui.group_ranged(marks.set_end(0, true, 90).expect("a group"));
        drive_with(&mut ui, clip(&marks), &[(300.0, 200.0, None)]);
        assert_eq!(ui.markup.group, Some((0, (0, 90))), "its out set at a frame, it stays selected");
        ui.toggle_markup();
        assert_eq!((ui.group_selected(), ui.group_for_keys()), (Some(0), None));
    }

    /// notes read from a file that says they were made on another version of the clip's file are shown, and labeled.
    #[test]
    fn notes_made_on_an_earlier_version_of_the_file_say_so() {
        let said = |marks: &Marks| earlier_chip(&clip(marks)).map(|parts| parts.into_iter().map(|(words, _)| words).collect::<String>());
        assert_eq!(said(&Marks::of(vec![grouped((0, 9))], true)).as_deref(), Some("notes made on an earlier version of this file"));
        assert_eq!((said(&Marks::of(vec![grouped((0, 9))], false)), said(&Marks::of(Vec::new(), true)), earlier_chip(&model())), (None, None, None));
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

    /// shift keeps a box square and a circle round (the owner's ask, 2026-11-25): the larger of the drag's two
    /// extents both ways, toward where the drag went. without it the drag is drawn as it went.
    #[test]
    fn shift_keeps_a_box_square_and_a_circle_round() {
        let from = (300.0, 180.0);
        for tool in [Tool::Rectangle, Tool::Circle] {
            for (dx, dy) in [(60.0, 24.0), (-60.0, 24.0), (60.0, -24.0), (-24.0, -60.0)] {
                let to = (from.0 + dx, from.1 + dy);
                for shift in [false, true] {
                    let mut ui = Ui::new((640, 360), 1.0, false, None, None);
                    ui.toggle_markup();
                    ui.markup.tool = tool;
                    ui.keys_held(Mods { shift, ..Mods::default() });
                    let kept = drive(&mut ui, &drag(from, to));
                    let side = dx.abs().max(dy.abs());
                    let want = if shift { (from.0 + side * dx.signum(), from.1 + side * dy.signum()) } else { to };
                    let shape = match &kept[..] {
                        [Action::Mark(0, mark)] => &mark.shape,
                        _ => panic!("{tool:?} shift {shift} to {to:?}: {kept:?}"),
                    };
                    let (Shape::Rectangle(a, b) | Shape::Circle(a, b)) = shape else { panic!("{shape:?}") };
                    assert_eq!(matches!(shape, Shape::Circle(..)), tool == Tool::Circle);
                    assert!(near(*a, file(from)) && near(*b, file(want)), "{tool:?} shift {shift} to {to:?}: {shape:?}, wanted the far corner at {:?}", file(want));
                }
            }
        }
    }

    /// shift is read in every pass, so pressing it or letting it go in the middle of a drag changes the box at once,
    /// the pointer still; the mark kept is the one last shown. square is square on screen: with the picture turned a
    /// quarter and the file's pixels drawn twice as wide as high, the box is as wide as high in the window, not in the
    /// file.
    #[test]
    fn shift_is_read_as_it_changes_and_square_is_square_on_screen() {
        // a 64 x 32 file turned a quarter: 32 x 64 as shown, in 360 x 360, so a pixel is 11.25 across and 5.625 down
        let turned = Look { turns: 1, ..Look::default() };
        let clip = || Model { shown_size: (32, 64), turned, ..model() };
        let place = Placing { picture: clip().picture, look: turned, size: (64, 32) };
        let (from, to) = ((250.0, 150.0), (330.0, 180.0));
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        let corners = |ui: &Ui| match &ui.markup.drawing {
            Some((_, Mark { shape: Shape::Rectangle(a, b), .. })) => (place.to_window(*a), place.to_window(*b)),
            other => panic!("drawing {other:?}"),
        };
        let mut steps = drag(from, to);
        let release = steps.split_off(4);
        drive_with(&mut ui, clip(), &steps);
        ui.keys_held(Mods { shift: true, ..Mods::default() });
        drive_with(&mut ui, clip(), &[(to.0, to.1, None)]);
        let (a, b) = corners(&ui);
        assert!((a.0 - from.0).abs() < 1e-3 && (a.1 - from.1).abs() < 1e-3, "the box starts where the drag did: {a:?}");
        assert!((b.0 - a.0 - 80.0).abs() < 1e-3 && (b.1 - a.1 - 80.0).abs() < 1e-3, "square in the window as shift goes down: {a:?} to {b:?}");
        ui.keys_held(Mods::default());
        drive_with(&mut ui, clip(), &[(to.0, to.1, None)]);
        let (_, b) = corners(&ui);
        assert!((b.0 - to.0).abs() < 1e-3 && (b.1 - to.1).abs() < 1e-3, "back with the pointer as it comes up: {b:?}");
        ui.keys_held(Mods { shift: true, ..Mods::default() });
        drive_with(&mut ui, clip(), &[(to.0, to.1, None)]);
        let shown = ui.markup.drawing.clone().map(|(_, mark)| mark);
        let kept = drive_with(&mut ui, clip(), &release);
        assert_eq!(kept, vec![Action::Mark(0, shown.expect("a box being drawn"))]);
    }

    /// a pen's line is kept smoothed by the settings' amount, its ends where the pointer went down and came up; with
    /// smoothing off it is kept as drawn.
    #[test]
    fn a_pens_line_is_kept_smoothed_by_the_settings() {
        // a jagged line, 4 across and 3 up or down a step
        let steps: Vec<(f32, f32)> = (0..30).map(|i| (200.0 + i as f32 * 4.0, 180.0 + if i % 2 == 0 { -3.0 } else { 3.0 })).collect();
        let (first, last) = (steps[0], steps[steps.len() - 1]);
        let path: Vec<Step> = [(first.0, first.1, None), (first.0, first.1, Some((Left, true)))].into_iter().chain(steps.iter().map(|p| (p.0, p.1, None))).chain([(last.0, last.1, Some((Left, false))), (last.0, last.1, None)]).collect();
        let drawn: Vec<Point> = steps.iter().map(|&p| file(p)).collect();
        // how far the line strays from its middle, away from its ends
        let stray = |points: &[Point]| points[5..points.len() - 5].iter().map(|p| (p.1 - file((0.0, 180.0)).1).abs()).fold(0.0, f32::max);
        for strength in [0.0, crate::settings::Settings::default().pen_smoothing] {
            let settings = crate::settings::Settings { pen_smoothing: strength, ..Default::default() };
            let screen = super::super::Screen { tabs: vec!["clip"], current: 0, places: Vec::new(), clip: Some(model()), error: None, opening: None, list: None, browsing: false, fullscreen: false, toast: None, settings: &settings };
            let mut ui = Ui::new((640, 360), 1.0, false, None, None);
            ui.toggle_markup();
            ui.markup.tool = Tool::Pen;
            let kept = drive_on(&mut ui, &screen, &path);
            let [Action::Mark(0, Mark { shape: Shape::Pen(points), .. })] = &kept[..] else { panic!("{kept:?}") };
            assert_eq!(points.len(), drawn.len(), "a point for each the pointer passed");
            assert!(near(points[0], drawn[0]) && near(points[points.len() - 1], drawn[drawn.len() - 1]), "the ends hold at {strength}: {points:?}");
            if strength == 0.0 {
                assert!(points.iter().zip(&drawn).all(|(a, b)| near(*a, *b)), "off, as drawn: {points:?}");
            } else {
                assert!(stray(points) < 0.5 * stray(&drawn), "smoothed: strays {} where it was drawn straying {}", stray(points), stray(&drawn));
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
        assert!(matches!(&kept[..], [Action::Mark(0, Mark { shape: Shape::Text(at, words, _), .. })] if near(*at, file(first)) && words == "hi"), "{kept:?}");
        assert!(ui.markup.writing.is_none() && ui.markup.on && !ui.typing());
        // typed, then a click elsewhere on the picture: kept, and a new field there
        drive(&mut ui, &click(first));
        "ok".chars().for_each(|c| ui.char(c));
        drive(&mut ui, &[(first.0, first.1, None)]);
        let kept = drive(&mut ui, &click(second));
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert!(matches!(&kept[0], Action::Mark(0, Mark { shape: Shape::Text(at, words, _), .. }) if near(*at, file(first)) && words == "ok"), "{kept:?}");
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

    /// the row of tools, every ink in it, fits the narrowest window the player opens (640 wide) with 8 to spare each
    /// side, where it stands as it always does: in the middle, under the strip's place.
    #[test]
    fn the_row_of_tools_fits_the_narrowest_window() {
        let mut ui = Ui::new((640, 360), 1.0, false, None, None);
        ui.toggle_markup();
        drive(&mut ui, &[(320.0, 200.0, None)]);
        let row = ui.markup.row.expect("the row of tools is up");
        assert!(row.left() >= 8.0 && row.right() <= 632.0, "{row:?}");
        assert!((row.center().x - 320.0).abs() < 0.01 && row.top() == 42.0, "{row:?}");
    }

    /// a click on the row of tools picks a tool or an ink, or does what undo, redo, clear, save image and done say
    /// (their keys, or the screenshot), and draws nothing.
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
        let press = |ui: &mut Ui, along: f32| {
            let row = ui.markup.row.expect("the row of tools is up");
            let x = if along < 0.0 { row.right() + along } else { row.left() + along };
            drive(ui, &click((x, row.top() + 4.0 + ITEM_HIGH / 2.0)))
        };
        // inside the line and the padding (4): the tools 32 apart, select first, a rule 11, the ten inks 20 apart, a
        // rule, undo, redo, clear, a rule, then the words
        assert_eq!(press(&mut ui, 4.0 + 15.0), vec![]);
        assert_eq!(ui.markup.tool, Tool::Select);
        assert_eq!(press(&mut ui, 4.0 + 4.0 * 32.0 + 15.0), vec![]);
        assert_eq!(press(&mut ui, 4.0 + 5.0 * 32.0 + 11.0 + 20.0 + 9.0), vec![]);
        assert_eq!(press(&mut ui, 4.0 + 382.0 + 15.0), vec![Action::Key(Key::Z, keys::CTRL)]);
        assert_eq!(press(&mut ui, 4.0 + 382.0 + 32.0 + 15.0), vec![Action::Key(Key::Z, keys::CTRL_SHIFT)]);
        assert_eq!(press(&mut ui, 4.0 + 382.0 + 64.0 + 15.0), vec![Action::ClearMarks]);
        assert_eq!(press(&mut ui, 4.0 + 382.0 + 96.0 + 11.0 + 20.0), vec![Action::Shot(Shot::Frame)]);
        assert_eq!(press(&mut ui, -14.0), vec![Action::Key(Key::E, keys::NONE)]);
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
