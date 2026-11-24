//! the menus that open over the bar, laid out as the design has them.

use super::widgets::{self, ROW};
use super::{font, Action, Menu, Model, DIM, MONO};
use crate::play::Loop;
use crate::view::{Channel, Guides, Look, LADDER};
use egui::{Pos2, Rect};

const WIDTH: f32 = 268.0;
/// the ends of the speed slider (the transport holds speed to the same)
const SLOWEST: f64 = 0.1;
const FASTEST: f64 = 8.0;

/// draws `menu` standing with its right bottom corner on `corner`. returns where it was drawn.
pub fn show(ui: &mut egui::Ui, menu: Menu, corner: Pos2, alpha: f32, model: &Model, actions: &mut Vec<Action>) -> Rect {
    let name = match menu {
        Menu::Loop => "loop-menu",
        Menu::Speed => "speed-menu",
        Menu::Zoom => "zoom-menu",
        Menu::View => "view-menu",
    };
    widgets::menu(ui, name, corner, WIDTH, alpha, |ui| match menu {
        Menu::Loop => looping(ui, model, actions),
        Menu::Speed => speed(ui, model, actions),
        Menu::Zoom => zoom(ui, model, actions),
        Menu::View => view(ui, model, actions),
    })
}

/// a slider and a field side by side. returns where the slider was moved to (0 to 1), and what was typed.
fn slider_and_field(ui: &mut egui::Ui, name: &str, at: f32, shown: &str, unit: &str) -> (Option<f32>, Option<f32>) {
    let row = widgets::row(ui, ROW);
    let (field, typed) = widgets::field(ui, name, row.right_center(), shown, unit, false);
    let slid = widgets::slider(ui, &format!("{name}-slider"), Rect::from_min_max(row.min, Pos2::new(field.left() - 10.0, row.bottom())), at);
    (slid, typed.and_then(|t| t.parse().ok()))
}

fn looping(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    widgets::heading(ui, "loop", &["R"]);
    // a folder's "all" comes with playing folders
    let modes = [("off", Loop::Off), ("one", Loop::One)];
    let on = modes.iter().position(|m| model.seam.is_none() && m.1 == model.looping);
    if let Some(i) = widgets::segments(ui, "loop-modes", &modes.map(|m| m.0), on) {
        actions.push(Action::Loop(modes[i].1));
    }
    let row = widgets::row(ui, ROW);
    let (field, typed) = widgets::field(ui, "seam-seconds", row.right_center(), &format!("{:.1}", model.seam_seconds), "s", false);
    if let Some(seconds) = typed.and_then(|t| t.parse::<f64>().ok()) {
        actions.push(Action::Seam(Some(seconds)));
    }
    if widgets::switch(ui, "seam", Rect::from_min_max(row.min, Pos2::new(field.left() - 9.0, row.bottom())), model.seam.is_some(), "loop analysis", "A", false) {
        actions.push(Action::Seam(model.seam.is_none().then_some(model.seam_seconds)));
    }
    widgets::note(ui, "plays the end into the start, over and over. the pause on black is the trip back, so the seam is the only cut you see.");
}

fn speed(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    widgets::heading(ui, "speed", &["shift -", "shift ="]);
    let stops = [("0.25×", 0.25), ("0.5×", 0.5), ("1×", 1.0), ("2×", 2.0), ("4×", 4.0)];
    let on = stops.iter().position(|s| s.1 == model.speed);
    if let Some(i) = widgets::segments(ui, "speed-stops", &stops.map(|s| s.0), on) {
        actions.push(Action::Speed(stops[i].1));
    }
    // the slider runs evenly in ratios, from the slowest speed to the fastest
    let at = ((model.speed / SLOWEST).ln() / (FASTEST / SLOWEST).ln()) as f32;
    let (slid, typed) = slider_and_field(ui, "speed", at, &format!("{:.2}", model.speed), "×");
    if let Some(to) = slid.map(|t| SLOWEST * (FASTEST / SLOWEST).powf(t as f64)).or(typed.map(f64::from)) {
        actions.push(Action::Speed(to));
    }
}

fn zoom(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    widgets::heading(ui, "zoom", &["0", "1", "2", "-", "="]);
    let stops: [(&str, Option<f32>); 5] = [("fit", None), ("1:1", Some(1.0)), ("2×", Some(2.0)), ("4×", Some(4.0)), ("8×", Some(8.0))];
    let on = stops.iter().position(|(_, z)| match z {
        None => !model.zoomed,
        Some(z) => model.zoomed && (model.zoom - z).abs() < 1e-4,
    });
    if let Some(i) = widgets::segments(ui, "zoom-stops", &stops.map(|s| s.0), on) {
        actions.push(Action::Zoom(stops[i].1));
    }
    // the slider runs evenly in doublings, from the least zoom to the most
    let (low, high) = (LADDER[0], LADDER[LADDER.len() - 1]);
    let percent = if model.zoom >= 1.0 { format!("{}", (model.zoom * 100.0).round()) } else { format!("{}", (model.zoom * 1000.0).round() / 10.0) };
    let (slid, typed) = slider_and_field(ui, "zoom", (model.zoom / low).ln() / (high / low).ln(), &percent, "%");
    if let Some(to) = slid.map(|t| low * (high / low).powf(t)).or(typed.map(|p| p / 100.0)) {
        actions.push(Action::Zoom(Some(to.clamp(low, high))));
    }
    widgets::note(ui, "100% is one video pixel on one display pixel. from there up, pixels are drawn as squares with no smoothing.");
}

fn view(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    let (guides, look) = (model.guides, model.look);
    widgets::heading(ui, "view", &[]);
    // the grid, with its tile's width and height in video pixels. typing a size turns the grid on
    let row = widgets::row(ui, ROW);
    let (height, typed_h) = widgets::field(ui, "tile-h", row.right_center(), &guides.tile.1.to_string(), "px", true);
    let times = ui.painter().layout_no_wrap("×".to_string(), font(MONO, 11.0), DIM);
    let times_at = Pos2::new(height.left() - 5.0 - times.size().x, row.center().y - times.size().y / 2.0);
    ui.painter().galley(times_at, times, DIM);
    let (width, typed_w) = widgets::field(ui, "tile-w", Pos2::new(times_at.x - 5.0, row.center().y), &guides.tile.0.to_string(), "", true);
    let side = |typed: Option<String>| typed.and_then(|t| t.parse::<f32>().ok()).map(|v| (v.round() as u32).clamp(8, 4096));
    if let Some(tile) = match (side(typed_w), side(typed_h)) {
        (None, None) => None,
        (w, h) => Some((w.unwrap_or(guides.tile.0), h.unwrap_or(guides.tile.1))),
    } {
        actions.push(Action::Guides(Guides { grid: true, tile, ..guides }));
    }
    if widgets::switch(ui, "grid", Rect::from_min_max(row.min, Pos2::new(width.left() - 9.0, row.bottom())), guides.grid, "grid", "D", false) {
        actions.push(Action::Guides(Guides { grid: !guides.grid, ..guides }));
    }
    let row = widgets::row(ui, 18.0);
    if widgets::switch(ui, "center", row, guides.center, "center lines and circle", "C", true) {
        actions.push(Action::Guides(Guides { center: !guides.center, ..guides }));
    }
    let channels = [("rgb", Channel::Rgb), ("r", Channel::Red), ("g", Channel::Green), ("b", Channel::Blue), ("alpha", Channel::Alpha)];
    if let Some(i) = widgets::segments(ui, "channels", &channels.map(|c| c.0), channels.iter().position(|c| c.1 == look.channel)) {
        actions.push(Action::Look(Look { channel: channels[i].1, ..look }));
    }
    match widgets::buttons(ui, "turns", &[("mirror", "H", look.mirror), ("flip", "V", look.flip), ("rotate", "Q", look.turns != 0)]) {
        Some(0) => actions.push(Action::Look(Look { mirror: !look.mirror, ..look })),
        Some(1) => actions.push(Action::Look(Look { flip: !look.flip, ..look })),
        Some(_) => actions.push(Action::Look(Look { turns: (look.turns + 1) % 4, ..look })),
        None => {}
    }
}
