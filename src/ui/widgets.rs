//! what the menus are made of, drawn to the design: a heading with its keys, a row of choices, a slider with a field.

use super::{font, ACCENT, DIM, FG, HOVER, LINE, MENU, MONO_MEDIUM, SANS, SOLID, TRACK};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, Pos2, Rect, Sense, Vec2};

const CHOSEN: Color32 = Color32::from_rgb(0xec, 0xec, 0xec);
const ON_CHOSEN: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);

/// a menu, standing on `left_bottom`. returns where it was drawn.
pub fn menu(ui: &egui::Ui, name: &str, left_bottom: Pos2, alpha: f32, add: impl FnOnce(&mut egui::Ui)) -> Rect {
    let frame = egui::Frame::new()
        .fill(SOLID)
        .stroke((1.0, LINE))
        .inner_margin(12.0)
        .shadow(egui::Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(128) });
    let area = egui::Area::new(egui::Id::unique(name)).order(egui::Order::Foreground).fixed_pos(left_bottom).pivot(Align2::LEFT_BOTTOM);
    let shown = area.show(ui.ctx(), |ui| {
        ui.set_opacity(alpha);
        frame.show(ui, |ui| {
            // the design's width counts the border and the padding
            ui.set_width(MENU - 26.0);
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 11.0);
            add(ui);
        });
    });
    shown.response.rect
}

/// a section's name, small and spaced out, with the keys that do the same at the right.
pub fn heading(ui: &mut egui::Ui, text: &str, keys: &[&str]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
    let painter = ui.painter();
    let mut job = LayoutJob::default();
    job.append(&text.to_uppercase(), 0.0, TextFormat { font_id: font(MONO_MEDIUM, 10.0), color: DIM, extra_letter_spacing: 1.6, ..Default::default() });
    let galley = painter.layout_job(job);
    painter.galley(Pos2::new(rect.left(), rect.center().y - galley.size().y / 2.0), galley, DIM);
    let mut right = rect.right();
    for key in keys.iter().rev() {
        let galley = painter.layout_no_wrap(key.to_string(), font(MONO_MEDIUM, 10.0), DIM);
        let cap = Rect::from_min_size(Pos2::new(right - galley.size().x - 10.0, rect.top()), Vec2::new(galley.size().x + 10.0, 16.0));
        painter.rect_stroke(cap, 3.0, (1.0, LINE), egui::StrokeKind::Inside);
        painter.galley(cap.center() - galley.size() / 2.0, galley, DIM);
        right = cap.left() - 5.0;
    }
}

/// a row of choices, one of them lit. returns the one clicked.
pub fn segments(ui: &mut egui::Ui, name: &str, labels: &[&str], on: Option<usize>) -> Option<usize> {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
    let painter = ui.painter().clone();
    painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
    let inner = rect.shrink(1.0);
    let width = inner.width() / labels.len() as f32;
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let cell = Rect::from_min_size(Pos2::new(inner.left() + width * i as f32, inner.top()), Vec2::new(width, inner.height()));
        let response = ui.interact(cell, ui.scope_id().with((name, i)), Sense::click());
        if on == Some(i) {
            painter.rect_filled(cell, 0.0, CHOSEN);
        } else if response.hovered() {
            painter.rect_filled(cell, 0.0, HOVER);
        }
        if i > 0 {
            painter.rect_filled(Rect::from_min_size(cell.min, Vec2::new(1.0, cell.height())), 0.0, LINE);
        }
        painter.text(cell.center(), Align2::CENTER_CENTER, label, font(MONO_MEDIUM, 11.0), if on == Some(i) { ON_CHOSEN } else { DIM });
        if response.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// what happened to a slider and its field in this pass.
pub struct Typing {
    /// the slider was moved to here, 0 to 1
    pub slid: Option<f32>,
    /// enter was pressed in the field
    pub entered: bool,
    /// the field is being typed into
    pub focused: bool,
}

/// a slider standing at `at` (0 to 1), and beside it a field holding `text` with its unit after it. the accent color
/// lives here: on the slider, and on the field's edge while it is typed into.
pub fn slider_and_field(ui: &mut egui::Ui, name: &str, at: f32, text: &mut String, unit: &str) -> Typing {
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
    let painter = ui.painter().clone();
    let unit = painter.layout_no_wrap(unit.to_string(), font(MONO_MEDIUM, 11.5), DIM);
    // the field: a border, 7 of padding, 46 for the number, 4, the unit, 7, a border
    let field = Rect::from_min_max(Pos2::new(row.right() - (8.0 + 46.0 + 4.0 + unit.size().x + 8.0), row.top()), row.max);
    let number = Rect::from_min_size(field.min + Vec2::new(8.0, 1.0), Vec2::new(46.0, 24.0));

    const KNOB: f32 = 6.0;
    let slider = Rect::from_min_max(row.min, Pos2::new(field.left() - 10.0, row.bottom()));
    let response = ui.interact(slider, ui.scope_id().with(name), Sense::click_and_drag());
    let (low, high) = (slider.left() + KNOB, slider.right() - KNOB);
    let slid = response.interact_pointer_pos().filter(|_| response.is_pointer_button_down_on() || response.clicked()).map(|p| ((p.x - low) / (high - low)).clamp(0.0, 1.0));
    let knob = Pos2::new(low + (high - low) * slid.unwrap_or(at), slider.center().y);
    let track = Rect::from_min_max(Pos2::new(slider.left(), knob.y - 2.0), Pos2::new(slider.right(), knob.y + 2.0));
    painter.rect_filled(track, 2.0, TRACK);
    painter.rect_filled(Rect::from_min_max(track.min, Pos2::new(knob.x, track.bottom())), 2.0, ACCENT);
    painter.circle_filled(knob, KNOB, ACCENT);

    // the number is egui's own text field, bare, inside our border
    let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(number).layout(egui::Layout::right_to_left(Align::Center)));
    let edit = inside.add(egui::TextEdit::singleline(text).frame(egui::Frame::NONE).font(font(MONO_MEDIUM, 11.5)).text_color(FG).align(Align2::RIGHT_CENTER).margin(egui::Margin::ZERO).desired_width(46.0).clip_text(true));
    text.retain(|c| c.is_ascii_digit() || c == '.');
    let focused = edit.has_focus();
    painter.rect_stroke(field, 0.0, (1.0, if focused { ACCENT } else { LINE }), egui::StrokeKind::Inside);
    painter.galley(Pos2::new(number.right() + 4.0, field.center().y - unit.size().y / 2.0), unit, DIM);
    Typing { slid, entered: edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)), focused }
}

/// a few words of explanation under a control.
pub fn note(ui: &mut egui::Ui, text: &str) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    job.append(text, 0.0, TextFormat { font_id: font(SANS, 11.5), color: DIM, line_height: Some(11.5 * 1.4), ..Default::default() });
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    ui.painter().galley(rect.min, galley, DIM);
}
