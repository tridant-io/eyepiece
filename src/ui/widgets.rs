//! what the menus are made of, drawn to the design: a heading with its keys, a row of choices, a slider, a field, a
//! switch. the accent color lives here and nowhere else: on sliders, switches and the field being typed into.

use super::{font, text_top, ACCENT, AMBER, DIM, FG, HOVER, LINE, MONO, MONO_MEDIUM, SANS, SOLID, TRACK};
use crate::media::meta::{Row, Tone};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, Pos2, Rect, Sense, Vec2};

pub const CHOSEN: Color32 = Color32::from_rgb(0xec, 0xec, 0xec);
pub const ON_CHOSEN: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);
/// a row that holds a field
pub const ROW: f32 = 26.0;

/// a menu `width` wide, with the corner `pivot` names on `corner`. returns where it was drawn.
pub fn menu(ui: &egui::Ui, name: &str, corner: Pos2, pivot: Align2, width: f32, alpha: f32, add: impl FnOnce(&mut egui::Ui)) -> Rect {
    let frame = egui::Frame::new()
        .fill(SOLID)
        .stroke((1.0, LINE))
        .inner_margin(12.0)
        .shadow(egui::Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(128) });
    let area = egui::Area::new(egui::Id::unique(name)).order(egui::Order::Foreground).fixed_pos(corner).pivot(pivot);
    let shown = area.show(ui.ctx(), |ui| {
        ui.set_opacity(alpha);
        frame.show(ui, |ui| {
            // the design's width counts the border and the padding
            ui.set_width(width - 26.0);
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 11.0);
            add(ui);
        });
    });
    shown.response.rect
}

/// the next row of a menu, this high.
pub fn row(ui: &mut egui::Ui, height: f32) -> Rect {
    ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover()).0
}

/// a key's name in its small outlined cap, with its right middle (or, `from_left`, its left middle) at `at`.
/// returns the cap.
pub fn key_cap(ui: &egui::Ui, at: Pos2, key: &str, from_left: bool, color: Color32) -> Rect {
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(key.to_string(), font(MONO_MEDIUM, 10.0), color);
    let size = Vec2::new(galley.size().x + 10.0, 16.0);
    let cap = Rect::from_min_size(Pos2::new(if from_left { at.x } else { at.x - size.x }, at.y - 8.0), size);
    painter.rect_stroke(cap, 3.0, (1.0, if color == DIM { LINE } else { color.gamma_multiply(0.4) }), egui::StrokeKind::Inside);
    painter.galley(Pos2::new(cap.center().x - galley.size().x / 2.0, text_top(painter, font(MONO_MEDIUM, 10.0), cap.center().y)), galley, color);
    cap
}

/// a section's name, small and spaced out, with the keys that do the same at the right.
pub fn heading(ui: &mut egui::Ui, text: &str, keys: &[&str]) {
    let rect = row(ui, 16.0);
    let mut job = LayoutJob::default();
    job.append(&text.to_uppercase(), 0.0, TextFormat { font_id: font(MONO_MEDIUM, 10.0), color: DIM, extra_letter_spacing: 1.6, ..Default::default() });
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(Pos2::new(rect.left(), text_top(ui.painter(), font(MONO_MEDIUM, 10.0), rect.center().y)), galley, DIM);
    let mut right = rect.right();
    for key in keys.iter().rev() {
        right = key_cap(ui, Pos2::new(right, rect.center().y), key, false, DIM).left() - 5.0;
    }
}

/// a section's name with a few dim words at its right.
pub fn heading_note(ui: &mut egui::Ui, text: &str, note: &str) {
    heading(ui, text, &[]);
    let rect = ui.min_rect();
    let last = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 16.0), rect.max);
    let galley = ui.painter().layout_no_wrap(note.to_string(), font(MONO_MEDIUM, 10.0), DIM);
    ui.painter().galley(Pos2::new(last.right() - galley.size().x, text_top(ui.painter(), font(MONO_MEDIUM, 10.0), last.center().y)), galley, DIM);
}

/// a section's name with a small outlined button at its right. true when the button is clicked.
pub fn heading_button(ui: &mut egui::Ui, text: &str, button: &str) -> bool {
    heading(ui, text, &[]);
    let rect = ui.min_rect();
    let last = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 16.0), rect.max);
    let galley = ui.painter().layout_no_wrap(button.to_string(), font(SANS, 11.5), FG);
    let frame = Rect::from_min_max(Pos2::new(last.right() - galley.size().x - 14.0, last.center().y - 9.0), Pos2::new(last.right(), last.center().y + 9.0));
    let response = ui.interact(frame, ui.scope_id().with((text, button)), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(frame, 0.0, HOVER);
    }
    ui.painter().rect_stroke(frame, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
    ui.painter().galley(Pos2::new(frame.left() + 7.0, text_top(ui.painter(), font(SANS, 11.5), frame.center().y)), galley, FG);
    response.clicked()
}

/// the metadata's rows: a dim name and its value, which wraps, with a rule between groups. amber with a mark is a
/// fault in the file; dim is what the file does not say, with the assumption in use.
pub fn rows(ui: &mut egui::Ui, groups: &[Vec<Row>]) {
    const NAME: f32 = 86.0;
    ui.spacing_mut().item_spacing.y = 5.0;
    let line = |text: &str, color: Color32, width: f32, words: bool| {
        let mut job = LayoutJob::default();
        job.wrap.max_width = width;
        // long paths have no spaces to break at
        job.wrap.break_anywhere = !text.contains(' ');
        let font_id = if words { font(SANS, 12.0) } else { font(MONO, 11.5) };
        job.append(text, 0.0, TextFormat { font_id, color, line_height: Some(11.5 * 1.35), ..Default::default() });
        job
    };
    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            let rule = row(ui, 5.0);
            ui.painter().rect_filled(Rect::from_min_size(Pos2::new(rule.left(), rule.top() + 2.0), Vec2::new(rule.width(), 1.0)), 0.0, LINE);
        }
        for fact in group {
            let (value, color) = match fact.tone {
                Tone::Plain => (fact.value.clone(), FG),
                Tone::Fault => (format!("! {}", fact.value), AMBER),
                Tone::Assumed => (fact.value.clone(), DIM),
            };
            let name = ui.painter().layout_job(line(fact.label, DIM, NAME, false));
            let value = ui.painter().layout_job(line(&value, color, ui.available_width() - NAME - 10.0, fact.tone != Tone::Plain));
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), name.size().y.max(value.size().y)), Sense::hover());
            ui.painter().galley(rect.min, name, DIM);
            ui.painter().galley(Pos2::new(rect.left() + NAME + 10.0, rect.top()), value, color);
        }
    }
}

/// a row of choices, one of them lit. returns the one clicked.
pub fn segments(ui: &mut egui::Ui, name: &str, labels: &[&str], on: Option<usize>) -> Option<usize> {
    let rect = row(ui, 28.0);
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
        let color = if on == Some(i) { ON_CHOSEN } else { DIM };
        let galley = painter.layout_no_wrap(label.to_string(), font(MONO_MEDIUM, 11.0), color);
        painter.galley(Pos2::new(cell.center().x - galley.size().x / 2.0, text_top(&painter, font(MONO_MEDIUM, 11.0), cell.center().y)), galley, color);
        if response.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// outlined buttons side by side, each with its name and the key that does the same; lit while on. returns the one
/// clicked.
pub fn buttons(ui: &mut egui::Ui, name: &str, items: &[(&str, &str, bool)]) -> Option<usize> {
    let rect = row(ui, 30.0);
    let painter = ui.painter().clone();
    let (mut x, mut clicked) = (rect.left(), None);
    for (i, (label, key, on)) in items.iter().enumerate() {
        let color = if *on { ON_CHOSEN } else { FG };
        let galley = painter.layout_no_wrap(label.to_string(), font(SANS, 12.0), color);
        let key_width = if key.is_empty() { 0.0 } else { painter.layout_no_wrap(key.to_string(), font(MONO_MEDIUM, 10.0), color).size().x + 10.0 + 7.0 };
        let button = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(9.0 + galley.size().x + key_width + 9.0, rect.height()));
        let response = ui.interact(button, ui.scope_id().with((name, i)), Sense::click());
        if *on {
            painter.rect_filled(button, 0.0, CHOSEN);
        } else if response.hovered() {
            painter.rect_filled(button, 0.0, HOVER);
        }
        painter.rect_stroke(button, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
        let text_at = Pos2::new(button.left() + 9.0, text_top(&painter, font(SANS, 12.0), button.center().y));
        let text_right = text_at.x + galley.size().x;
        painter.galley(text_at, galley, color);
        if !key.is_empty() {
            key_cap(ui, Pos2::new(text_right + 7.0, button.center().y), key, true, if *on { ON_CHOSEN } else { DIM });
        }
        if response.clicked() {
            clicked = Some(i);
        }
        x = button.right() + 6.0;
    }
    clicked
}

/// a slider in `rect`, standing at `at` (0 to 1), in `color`: the accent in a menu, plain on the bar. returns where
/// it was moved to.
pub fn slider(ui: &mut egui::Ui, name: &str, rect: Rect, at: f32, color: Color32) -> Option<f32> {
    const KNOB: f32 = 6.0;
    let response = ui.interact(rect, ui.scope_id().with(name), Sense::click_and_drag());
    let (low, high) = (rect.left() + KNOB, rect.right() - KNOB);
    let slid = response.interact_pointer_pos().filter(|_| response.is_pointer_button_down_on() || response.clicked()).map(|p| ((p.x - low) / (high - low)).clamp(0.0, 1.0));
    let knob = Pos2::new(low + (high - low) * slid.unwrap_or(at).clamp(0.0, 1.0), rect.center().y);
    let track = Rect::from_min_max(Pos2::new(rect.left(), knob.y - 2.0), Pos2::new(rect.right(), knob.y + 2.0));
    let painter = ui.painter();
    painter.rect_filled(track, 2.0, TRACK);
    painter.rect_filled(Rect::from_min_max(track.min, Pos2::new(knob.x, track.bottom())), 2.0, color);
    painter.circle_filled(knob, KNOB, color);
    slid
}

/// a boxed number with its unit after it, its right middle at `right`. it shows `shown` until it is typed into.
/// returns the box, and what was typed once enter is pressed or the field is left (escape leaves it unchanged).
pub fn field(ui: &mut egui::Ui, name: &str, right: Pos2, shown: &str, unit: &str, small: bool) -> (Rect, Option<String>) {
    let (pad, width) = if small { (5.0, 32.0) } else { (7.0, 46.0) };
    let painter = ui.painter().clone();
    let unit = painter.layout_no_wrap(unit.to_string(), font(MONO_MEDIUM, 11.5), DIM);
    let gap = if unit.size().x > 0.0 { 4.0 } else { 0.0 };
    let size = Vec2::new(1.0 + pad + width + gap + unit.size().x + pad + 1.0, ROW);
    let rect = Rect::from_min_size(Pos2::new(right.x - size.x, right.y - size.y / 2.0), size);
    // the number is laid out by egui, which centers a line by its box: it is lifted by the difference
    let lift = text_top(&painter, font(MONO_MEDIUM, 11.5), 0.0) + painter.layout_no_wrap("0".to_string(), font(MONO_MEDIUM, 11.5), FG).size().y / 2.0;
    let number = Rect::from_min_size(rect.min + Vec2::new(1.0 + pad, 1.0 + lift), Vec2::new(width, ROW - 2.0));

    // what is being typed is kept between passes only while the field has the keyboard
    let id = ui.scope_id().with(name);
    let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.to_string());
    let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(number).layout(egui::Layout::right_to_left(Align::Center)));
    let edit = inside.add(egui::TextEdit::singleline(&mut text).id(id.with("edit")).frame(egui::Frame::NONE).font(font(MONO_MEDIUM, 11.5)).text_color(FG).align(Align2::RIGHT_CENTER).margin(egui::Margin::ZERO).desired_width(width).clip_text(true));
    text.retain(|c| c.is_ascii_digit() || c == '.');
    let entered = (edit.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape))).then(|| text.clone());
    // asked before the store below is opened: egui's own state is behind one lock
    let focused = edit.has_focus();
    ui.data_mut(|d| {
        if focused {
            d.insert_temp(id, text);
        } else {
            d.remove::<String>(id);
        }
    });
    painter.rect_stroke(rect, 0.0, (1.0, if focused { ACCENT } else { LINE }), egui::StrokeKind::Inside);
    painter.galley(Pos2::new(number.right() + gap, text_top(&painter, font(MONO_MEDIUM, 11.5), rect.center().y)), unit, DIM);
    (rect, entered)
}

/// a switch at the left of `rect` with its label, and the key that does the same: after the label, or `key_right`,
/// at the right end. all of `rect` takes the click. true when clicked.
pub fn switch(ui: &mut egui::Ui, name: &str, rect: Rect, on: bool, label: &str, key: &str, key_right: bool) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(name), Sense::click());
    let painter = ui.painter().clone();
    let color = if on { ACCENT } else { DIM };
    let frame = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - 7.0), Vec2::new(26.0, 14.0));
    painter.rect_stroke(frame, 0.0, (1.0, color), egui::StrokeKind::Inside);
    painter.rect_filled(Rect::from_min_size(frame.min + Vec2::new(if on { 15.0 } else { 3.0 }, 3.0), Vec2::splat(8.0)), 0.0, color);
    let galley = painter.layout_no_wrap(label.to_string(), font(SANS, 12.0), FG);
    let after = frame.right() + 9.0 + galley.size().x + 9.0;
    painter.galley(Pos2::new(frame.right() + 9.0, text_top(&painter, font(SANS, 12.0), rect.center().y)), galley, FG);
    if !key.is_empty() {
        key_cap(ui, Pos2::new(if key_right { rect.right() } else { after }, rect.center().y), key, !key_right, DIM);
    }
    response.clicked()
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
