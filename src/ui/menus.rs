//! the menus that open over the bar, laid out as the design has them.

use super::widgets::{self, ROW};
use super::{cut_middle, font, text_top, Action, Extras, Facts, FolderList, Menu, Model, Shot, ACCENT, AMBER, DIM, FG, HOVER, MONO, MONO_MEDIUM};
use crate::event::{Key, Mods};
use crate::save::BIN;
use crate::settings::Loop;
use crate::view::{Channel, Guides, Look, LADDER};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, Pos2, Rect, Sense};

const WIDTH: f32 = 268.0;
const SHOT_WIDTH: f32 = 308.0;
const TRIM_WIDTH: f32 = 308.0;
const FACTS_WIDTH: f32 = 372.0;
const LIST_WIDTH: f32 = 456.0;
/// the ends of the speed slider (the transport holds speed to the same)
const SLOWEST: f64 = 0.1;
const FASTEST: f64 = 8.0;

/// the clips a tab plays, with its left top corner on `corner`: each with its size, rate and frames, the one that is
/// up marked. a size or a rate that differs from the first clip's is amber, which is how the odd one in a folder is
/// found (the design's fig. 14). a click on a row goes to that clip. it is no taller than `room`. returns where it was
/// drawn.
pub fn list(ui: &mut egui::Ui, corner: Pos2, alpha: f32, room: f32, list: &FolderList, actions: &mut Vec<Action>) -> Rect {
    // the columns after the name, from the right: frames, rate, size, with 10 between
    const COLUMNS: [f32; 3] = [56.0, 104.0, 84.0];
    widgets::menu(ui, "list-menu", corner, Align2::LEFT_TOP, LIST_WIDTH, alpha, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        widgets::heading_note(ui, "this tab plays", &format!("{} · {} clips", list.title, list.items.len()));
        // the heading and the menu's own edges take 60 of the room
        let most = (room - 60.0).max(46.0);
        // both limits are given: a menu is as tall as it was last drawn, and a list that grew would stay that short
        egui::ScrollArea::vertical().max_height(most).min_scrolled_height(most).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, item) in list.items.iter().enumerate() {
                let row = widgets::row(ui, 23.0);
                let response = ui.interact(row, ui.scope_id().with(("clip", i)), Sense::click());
                let painter = ui.painter();
                if i == list.at {
                    painter.rect_filled(row, 0.0, Color32::from_white_alpha(23));
                } else if response.hovered() {
                    painter.rect_filled(row, 0.0, HOVER);
                }
                let odd = |differs: bool| if differs { AMBER } else { DIM };
                let size = item.size.map_or_else(|| "…".to_string(), |(w, h)| format!("{w} × {h}"));
                let rate = item.rate.map_or("…", |r| r.trim_end_matches(" fps"));
                let frames = item.frames.map_or_else(|| "…".to_string(), |f| format!("{f} f"));
                let cells = [
                    (frames, DIM),
                    (rate.to_string(), odd(item.odd_rate)),
                    (size, odd(item.odd_size)),
                ];
                let mut right = row.right() - 6.0;
                for ((words, color), width) in cells.into_iter().zip(COLUMNS) {
                    let galley = painter.layout_no_wrap(words, font(MONO, 11.0), color);
                    painter.galley(Pos2::new(right - width, text_top(painter, font(MONO, 11.0), row.center().y)), galley, color);
                    right -= width + 10.0;
                }
                let name = cut_middle(painter, item.name, font(MONO_MEDIUM, 11.0), FG, right - row.left() - 6.0);
                painter.galley(Pos2::new(row.left() + 6.0, text_top(painter, font(MONO_MEDIUM, 11.0), row.center().y)), name, FG);
                if response.clicked() {
                    actions.push(Action::GoClip(i));
                }
            }
        });
    })
}

/// draws `menu` standing with its right bottom corner on `corner`. returns where it was drawn.
pub fn show(ui: &mut egui::Ui, menu: Menu, corner: Pos2, alpha: f32, model: &Model, actions: &mut Vec<Action>) -> Rect {
    let (name, width) = match menu {
        // the folder's list is not the clip's: `list` draws it
        Menu::List => return Rect::NOTHING,
        Menu::Loop => ("loop-menu", WIDTH),
        Menu::Speed => ("speed-menu", WIDTH),
        Menu::Zoom => ("zoom-menu", WIDTH),
        Menu::View => ("view-menu", WIDTH),
        Menu::Shot => ("shot-menu", SHOT_WIDTH),
        // the trim's keeps its switch and its prompt between passes: `trim` draws it
        Menu::Trim => return Rect::NOTHING,
    };
    widgets::menu(ui, name, corner, Align2::RIGHT_BOTTOM, width, alpha, |ui| match menu {
        Menu::List | Menu::Trim => {}
        Menu::Loop => looping(ui, model, actions),
        Menu::Speed => speed(ui, model, actions),
        Menu::Zoom => zoom(ui, model, actions),
        Menu::View => view(ui, model, actions),
        Menu::Shot => shot(ui, model, actions),
    })
}

/// the metadata panel, with the corner `pivot` names on `corner`, no taller than `room`. returns where it was drawn.
pub fn facts(ui: &mut egui::Ui, corner: Pos2, pivot: Align2, alpha: f32, room: f32, model: &Model, actions: &mut Vec<Action>) -> Rect {
    widgets::menu(ui, "facts", corner, pivot, FACTS_WIDTH, alpha, |ui| {
        ui.spacing_mut().item_spacing.y = 9.0;
        let copy = widgets::heading_button(ui, "metadata", "copy all");
        match &model.facts {
            Facts::Reading => widgets::note(ui, "reading the file…"),
            Facts::Failed(why) => widgets::note(ui, why),
            Facts::Ready(meta) => {
                if copy {
                    actions.push(Action::Copy(meta.text()));
                }
                // the heading and the menu's own edges take 60 of the room. both limits are given: a panel is as tall
                // as it was last drawn, and a list that grew (the facts arriving) would otherwise stay that short
                let most = (room - 60.0).max(40.0);
                egui::ScrollArea::vertical().max_height(most).min_scrolled_height(most).show(ui, |ui| {
                    widgets::rows(ui, &meta.groups);
                    // what the file cannot say of itself: how this player decodes it and brings its chroma up
                    use crate::media::meta::{Row, Tone};
                    let mut ours = Vec::new();
                    if !model.decoded.is_empty() {
                        ours.push(Row { label: "decode", value: model.decoded.to_string(), tone: Tone::Plain });
                    }
                    if let Some((words, assumed)) = &model.chroma {
                        ours.push(Row { label: "chroma", value: words.clone(), tone: if *assumed { Tone::Assumed } else { Tone::Plain } });
                    }
                    if !ours.is_empty() {
                        widgets::rows(ui, &[Vec::new(), ours]);
                    }
                });
            }
        }
    })
}

/// the trim's menu (the owner's button, 2026-11-26), standing over its button: what the trim keeps, as its chip says
/// it; how a copy of it is cut, exactly or from the key frame before; and saving it as a new file beside the clip, or
/// in the clip's place behind a prompt. `sound` is its keep sound switch and `confirming` whether its prompt is up.
/// returns where it was drawn.
pub fn trim(ui: &mut egui::Ui, corner: Pos2, alpha: f32, model: &Model, timecode: bool, (sound, confirming): (&mut bool, &mut bool), actions: &mut Vec<Action>) -> Rect {
    widgets::menu(ui, "trim-menu", corner, Align2::RIGHT_BOTTOM, TRIM_WIDTH, alpha, |ui| {
        if *confirming {
            widgets::heading(ui, "replace the original", &[]);
            widgets::line(ui, &format!("the original goes to the {BIN}, and the new file takes its name."), FG);
            widgets::note(ui, "the new file is written and checked first. if it is not what was asked, both are kept.");
            match widgets::confirm(ui, "trim-replace", "cancel", "replace") {
                Some(1) => {
                    actions.push(Action::Save { trim: true, sound: *sound, replace: true });
                    *confirming = false;
                }
                Some(_) => *confirming = false,
                None => {}
            }
            return;
        }
        widgets::heading(ui, "trim", &["shift [", "shift ]"]);
        // what it keeps, as its chip says it
        if let Some(parts) = super::trim_chip(model, timecode) {
            let mut job = LayoutJob::default();
            for (i, (text, color)) in parts.iter().enumerate() {
                let text = if i == 0 { text.trim_start_matches("trim · ") } else { text };
                job.append(text, 0.0, TextFormat { font_id: font(MONO_MEDIUM, 11.0), color: *color, ..Default::default() });
            }
            let galley = ui.painter().layout_job(job);
            let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
            ui.painter().galley(rect.min, galley, FG);
        }
        // a sequence's frames are files of their own: a copy of them is not made yet
        if model.sequence {
            return widgets::note(ui, "image sequences are not saved yet.");
        }
        if let Some((name, part)) = model.saving {
            widgets::line(ui, &format!("saving {name} · {}%", (part * 100.0).floor()), FG);
            if widgets::buttons(ui, "trim-stop", &[("stop", "", false)]).is_some() {
                actions.push(Action::StopSaving);
            }
            return;
        }
        let Some((from, kept)) = super::trimmed(model) else { return };
        let cut = match model.cut {
            None => return widgets::note(ui, "reading where its key frames are…"),
            Some(Err(why)) => return widgets::note(ui, why),
            Some(Ok(cut)) => cut,
        };
        // how it is cut: a copy starts at a key frame, and with frames stored out of order may end a frame or two on
        let at = |frame| super::stamp(frame, model.first, model.clock, timecode).1;
        let exact = (cut.first, cut.last) == (from, from + kept - 1);
        let how = match (exact, cut.first == from) {
            (true, _) => "copy · exact".to_string(),
            (false, false) => format!("copy · starts at key frame {} · {} frames", at(cut.first), cut.frames()),
            (false, true) => format!("copy · ends at {} · {} frames", at(cut.last), cut.frames()),
        };
        widgets::line(ui, &how, FG);
        if !exact {
            widgets::note(ui, "an exact cut of a long-gop file needs an encode, which is not built yet.");
        }
        // what of the clip's tracks the copy leaves out, said before it is saved
        if let Some(left) = &cut.left {
            widgets::note(ui, &format!("{left}."));
        }
        if cut.sound > 0 {
            let row = widgets::row(ui, 15.6);
            if widgets::switch(ui, "trim-sound", row, *sound, "keep sound", "", false) {
                *sound = !*sound;
            }
        }
        // a new file beside the clip is the default. in its place is offered only where the original can go to a bin
        let mut choices = vec![("save new file", "", true)];
        if model.no_replace.is_none() {
            choices.push(("replace original…", "", false));
        }
        match widgets::buttons(ui, "trim-save", &choices) {
            Some(0) => actions.push(Action::Save { trim: true, sound: *sound, replace: false }),
            Some(_) => *confirming = true,
            None => {}
        }
        if let Some(why) = model.no_replace {
            widgets::note(ui, &format!("replace original is not offered: {why}."));
        }
    })
}

fn shot(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    widgets::heading_note(ui, "screenshot", &format!("to {}", model.shots_to));
    match widgets::buttons(ui, "shots", &[("frame", "S", false), ("view", "shift S", false), ("copy", "ctrl C", false)]) {
        Some(0) => actions.push(Action::Shot(Shot::Frame)),
        Some(1) => actions.push(Action::Shot(Shot::View)),
        Some(_) => actions.push(Action::Shot(Shot::Copy)),
        None => {}
    }
    // what a saved picture carries besides the picture (fig. 18), kept between runs. a row is the design's line of
    // 12 at 1.3
    let extras = model.extras;
    for (name, on, label, to) in [
        ("caption", extras.caption, "caption: name, frame, timecode", Extras { caption: !extras.caption, ..extras }),
        ("guides", extras.guides, "guides, when showing", Extras { guides: !extras.guides, ..extras }),
        ("markup", extras.markup, "markup", Extras { markup: !extras.markup, ..extras }),
    ] {
        let row = widgets::row(ui, 15.6);
        if widgets::switch(ui, name, row, on, label, "", false) {
            actions.push(Action::Extras(to));
        }
    }
    if widgets::buttons(ui, "markup-mode", &[("markup mode", "E", false)]).is_some() {
        actions.push(Action::Key(Key::E, Mods::default()));
    }
}

/// a slider and a field side by side. returns where the slider was moved to (0 to 1), and what was typed.
fn slider_and_field(ui: &mut egui::Ui, name: &str, at: f32, shown: &str, unit: &str) -> (Option<f32>, Option<f32>) {
    let row = widgets::row(ui, ROW);
    let (field, typed) = widgets::field(ui, name, row.right_center(), shown, unit, false);
    let slid = widgets::slider(ui, &format!("{name}-slider"), Rect::from_min_max(row.min, Pos2::new(field.left() - 10.0, row.bottom())), at, ACCENT);
    (slid, typed.and_then(|t| t.parse().ok()))
}

fn looping(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    widgets::heading(ui, "loop", &["R"]);
    // "all" is a played folder's: its clips one after another, round and round
    let all = [("off", Loop::Off), ("one", Loop::One), ("all", Loop::All)];
    let modes = &all[..if model.folder { 3 } else { 2 }];
    let on = modes.iter().position(|m| model.seam.is_none() && m.1 == model.looping);
    if let Some(i) = widgets::segments(ui, "loop-modes", &modes.iter().map(|m| m.0).collect::<Vec<_>>(), on) {
        actions.push(Action::Loop(modes[i].1));
    }
    // a held still has no end and start to play into each other
    if !super::framed(model) {
        return;
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
