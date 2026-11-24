//! every command that has a key, in one table. the table of keys (`?`, fig. 21), the right-click menu and the tooltips
//! on the strip's and the bar's buttons are all read from it, so they say the same. it holds only keys that work:
//! markup (E) and a folder's previous and next clip (page up and down) come into it when they are built.

use super::{font, text_top, widgets, DIM, FG, HOVER, MONO, MONO_MEDIUM, SANS};
use crate::event::{Key, Mods};
use egui::{Align2, Color32, Pos2, Rect, Sense, Vec2};

pub const NONE: Mods = Mods { shift: false, alt: false, ctrl: false };
pub const SHIFT: Mods = Mods { shift: true, alt: false, ctrl: false };
pub const CTRL: Mods = Mods { shift: false, alt: false, ctrl: true };
const ALT: Mods = Mods { shift: false, alt: true, ctrl: false };
const CTRL_SHIFT: Mods = Mods { shift: true, alt: false, ctrl: true };

/// one entry of the table.
struct Command {
    /// what it does, as the right-click menu and a tooltip say it. None keeps it out of the menu: another key for a
    /// command that is there, what the pointer does, and esc
    name: Option<&'static str>,
    /// its key, as shown
    key: &'static str,
    /// the key that does it, which the menu sends. None for what only the pointer does
    send: Option<(Key, Mods)>,
    /// what its line of the table of keys says. entries one after another that say the same share a line, their keys
    /// side by side. None leaves it to the menu
    line: Option<&'static str>,
}

/// a command on the table of keys and in the menu
const fn both(line: &'static str, name: &'static str, key: &'static str, k: Key, mods: Mods) -> Command {
    Command { name: Some(name), key, send: Some((k, mods)), line: Some(line) }
}

/// a command in the menu that the table leaves out
const fn menu_only(name: &'static str, key: &'static str, k: Key, mods: Mods) -> Command {
    Command { name: Some(name), key, send: Some((k, mods)), line: None }
}

/// another key for a command, or esc: on the table only
const fn also(line: &'static str, key: &'static str, k: Key, mods: Mods) -> Command {
    Command { name: None, key, send: Some((k, mods)), line: Some(line) }
}

/// what the table names and the menu cannot send: what the pointer does, and the arrows taken together
const fn table_only(line: &'static str, key: &'static str) -> Command {
    Command { name: None, key, send: None, line: Some(line) }
}

/// the table: the menu's sections, in the order of the design's table of keys. the design's words, except where the
/// build differs: - and = zoom (the owner's keys) and speed takes shift, a double click is fullscreen (the owner's
/// change), and looping is off or one until folders play. the left and right arrows and ↔ are not in the type, so
/// those arrows are named in words and the readout switches "frames / timecode".
const SECTIONS: &[(&str, &[Command])] = &[
    (
        "play",
        &[
            both("play / pause", "play / pause", "space", Key::Space, NONE),
            both("previous / next frame", "previous frame", "F", Key::F, NONE),
            both("previous / next frame", "next frame", "G", Key::G, NONE),
            also("previous / next frame", "left", Key::Left, NONE),
            also("previous / next frame", "right", Key::Right, NONE),
            both("first / last frame", "first frame", "shift F", Key::F, SHIFT),
            both("first / last frame", "last frame", "shift G", Key::G, SHIFT),
            also("first / last frame", "home", Key::Home, NONE),
            also("first / last frame", "end", Key::End, NONE),
            menu_only("ten frames back", "shift left", Key::Left, SHIFT),
            menu_only("ten frames forward", "shift right", Key::Right, SHIFT),
            both("reverse · stop · forward", "reverse", "J", Key::J, NONE),
            both("reverse · stop · forward", "stop", "K", Key::K, NONE),
            both("reverse · stop · forward", "forward", "L", Key::L, NONE),
            table_only("scrub", "wheel"),
        ],
    ),
    (
        "zoom",
        &[
            table_only("zoom at the pointer", "shift wheel"),
            both("fit · 1:1 · 2×", "fit", "0", Key::Digit0, NONE),
            both("fit · 1:1 · 2×", "1:1", "1", Key::Digit1, NONE),
            both("fit · 1:1 · 2×", "2×", "2", Key::Digit2, NONE),
            both("zoom out / in", "zoom out", "-", Key::Minus, NONE),
            both("zoom out / in", "zoom in", "=", Key::Equals, NONE),
            table_only("fullscreen", "double-click"),
            table_only("pan", "alt arrows"),
            menu_only("pan left", "alt left", Key::Left, ALT),
            menu_only("pan right", "alt right", Key::Right, ALT),
            menu_only("pan up", "alt ↑", Key::Up, ALT),
            menu_only("pan down", "alt ↓", Key::Down, ALT),
        ],
    ),
    (
        "tabs",
        &[
            both("open", "open", "O", Key::O, NONE),
            both("new tab", "new tab", "N", Key::N, NONE),
            both("switch tabs", "previous tab", "[", Key::BracketLeft, NONE),
            both("switch tabs", "next tab", "]", Key::BracketRight, NONE),
            menu_only("close tab", "ctrl W", Key::W, CTRL),
            menu_only("reopen closed tab", "ctrl shift T", Key::T, CTRL_SHIFT),
        ],
    ),
    (
        "loop and speed",
        &[
            both("loop off · one", "loop off · one", "R", Key::R, NONE),
            both("loop analysis", "loop analysis", "A", Key::A, NONE),
            both("slower / faster", "slower", "shift -", Key::Minus, SHIFT),
            both("slower / faster", "faster", "shift =", Key::Equals, SHIFT),
            menu_only("1× speed", "backspace", Key::Backspace, NONE),
        ],
    ),
    (
        "view",
        &[
            both("frames / timecode", "frames / timecode", "T", Key::T, NONE),
            both("pin metadata", "pin metadata", "I", Key::I, NONE),
            both("grid · center lines", "grid", "D", Key::D, NONE),
            both("grid · center lines", "center lines", "C", Key::C, NONE),
            both("mirror · flip · rotate", "mirror", "H", Key::H, NONE),
            both("mirror · flip · rotate", "flip", "V", Key::V, NONE),
            both("mirror · flip · rotate", "rotate", "Q", Key::Q, NONE),
        ],
    ),
    (
        "screenshot",
        &[
            both("screenshot", "screenshot", "S", Key::S, NONE),
            both("screenshot", "screenshot of the view", "shift S", Key::S, SHIFT),
            menu_only("copy frame", "ctrl C", Key::C, CTRL),
        ],
    ),
    (
        "sound",
        &[
            both("mute · volume", "mute", "M", Key::M, NONE),
            both("mute · volume", "volume up", "↑", Key::Up, NONE),
            both("mute · volume", "volume down", "↓", Key::Down, NONE),
        ],
    ),
    (
        "window",
        &[
            both("fullscreen", "fullscreen", "P", Key::P, NONE),
            both("fullscreen", "fullscreen on the next display", "shift P", Key::P, SHIFT),
            table_only("every command", "right-click"),
            both("this table", "the keys", "?", Key::Slash, SHIFT),
            also("leave fullscreen, close", "esc", Key::Escape, NONE),
        ],
    ),
];

// the table of keys, as the design draws it: black at 86% over the picture, two columns filled row by row, 34 apart,
// lines 11 high at 1.3 with 5 between them, a line's keys taking 96 at least
const SHADE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 219);
const LINE: f32 = 11.0 * 1.3;
const LINE_GAP: f32 = 5.0;
const COLUMN_GAP: f32 = 34.0;
const KEYS_WIDE: f32 = 96.0;
// the right-click menu: as wide as the bar's menus, a row 24 high
const MENU_WIDTH: f32 = 268.0;
const MENU_ROW: f32 = 24.0;

fn commands() -> impl Iterator<Item = &'static Command> {
    SECTIONS.iter().flat_map(|(_, commands)| commands.iter())
}

/// the table of keys, line by line: the entries on each line, and what it says.
fn grouped() -> Vec<(Vec<&'static Command>, &'static str)> {
    let mut lines: Vec<(Vec<&'static Command>, &'static str)> = Vec::new();
    for c in commands() {
        let Some(said) = c.line else { continue };
        match lines.last_mut() {
            Some((on, words)) if *words == said => on.push(c),
            _ => lines.push((vec![c], said)),
        }
    }
    lines
}

/// a line's keys side by side, what is held said once: "shift F G", "shift - =".
fn joined(on: &[&Command]) -> String {
    let held = |key: &'static str| key.rsplit_once(' ').map(|(held, _)| held);
    let mut text = String::new();
    for (i, c) in on.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        match c.key.rsplit_once(' ') {
            Some((held_here, last)) if i > 0 && held(on[i - 1].key) == Some(held_here) => text.push_str(last),
            _ => text.push_str(c.key),
        }
    }
    text
}

/// the table of keys: each line's keys as shown, and what they do.
fn lines() -> Vec<(String, &'static str)> {
    grouped().into_iter().map(|(on, words)| (joined(&on), words)).collect()
}

/// a command by its key, for a button's tooltip: what it does, and the key.
pub fn named(key: Key, mods: Mods) -> Option<(String, String)> {
    commands().filter(|c| c.send == Some((key, mods))).find_map(|c| Some((c.name?.to_string(), c.key.to_string())))
}

/// the line of the table a key is on, for the tooltip of a button that stands for more than one command (a menu):
/// what the line says, and its keys.
pub fn line(key: Key, mods: Mods) -> Option<(String, String)> {
    grouped().into_iter().find(|(on, _)| on.iter().any(|c| c.send == Some((key, mods)))).map(|(on, words)| (words.to_string(), joined(&on)))
}

/// the table of keys over the picture: the picture darkened, and the table in the middle of the window. a press
/// anywhere is to take it away, so nothing under it takes the pointer while it is up.
pub fn table(ui: &egui::Ui, full: Rect) {
    let painter = ui.painter();
    painter.rect_filled(full, 0.0, SHADE);
    let (keys, words) = (font(MONO_MEDIUM, 11.0), font(MONO, 11.0));
    let laid: Vec<_> = lines().into_iter().map(|(k, w)| (painter.layout_no_wrap(k, keys.clone(), FG), painter.layout_no_wrap(w.to_string(), words.clone(), DIM))).collect();
    // a column's words start where its keys end (12 past the longest, which the design's keys never come near) and
    // it is as wide as its longest words
    let column = |at: usize| {
        let lines = || laid.iter().skip(at).step_by(2);
        let keys_wide = lines().map(|(k, _)| k.size().x + 12.0).fold(KEYS_WIDE, f32::max);
        (keys_wide, keys_wide + lines().map(|(_, w)| w.size().x).fold(0.0, f32::max))
    };
    let (left, right) = (column(0), column(1));
    let rows = laid.len().div_ceil(2) as f32;
    let corner = full.center() - Vec2::new((left.1 + COLUMN_GAP + right.1) / 2.0, (rows * (LINE + LINE_GAP) - LINE_GAP) / 2.0);
    for (i, (k, w)) in laid.into_iter().enumerate() {
        let (x, keys_wide) = if i % 2 == 0 { (corner.x, left.0) } else { (corner.x + left.1 + COLUMN_GAP, right.0) };
        let middle = corner.y + (i / 2) as f32 * (LINE + LINE_GAP) + LINE / 2.0;
        painter.galley(Pos2::new(x, text_top(painter, keys.clone(), middle)), k, FG);
        painter.galley(Pos2::new(x + keys_wide, text_top(painter, words.clone(), middle)), w, DIM);
    }
    egui::Area::new(egui::Id::unique("keys-table")).order(egui::Order::Middle).fixed_pos(full.min).show(ui.ctx(), |ui| {
        ui.allocate_exact_size(full.size(), Sense::click());
    });
}

/// the right-click menu: every command with its key beside it, in sections, its corner at `corner` and kept 8 in from
/// the window's edges by its size when it was `last` drawn; it scrolls when the window is too short. `opened` starts
/// it at its top. returns where it was drawn, and the key of the row chosen.
pub fn menu(ui: &egui::Ui, corner: Pos2, full: Rect, last: Rect, opened: bool) -> (Rect, Option<(Key, Mods)>) {
    let room = full.shrink(8.0);
    let at = Pos2::new(corner.x.min(room.right() - last.width()).max(room.left()), corner.y.min(room.bottom() - last.height()).max(room.top()));
    let mut chosen = None;
    let drawn = widgets::menu(ui, "commands", at, Align2::LEFT_TOP, MENU_WIDTH, 1.0, |ui| {
        // the menu's own border and padding take 26 of the room. both limits are given: a menu is as tall as it was
        // last drawn, and the first time is no taller than egui's default
        let most = room.height() - 26.0;
        let mut scroll = egui::ScrollArea::vertical().max_height(most).min_scrolled_height(most);
        if opened {
            scroll = scroll.vertical_scroll_offset(0.0);
        }
        scroll.show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, (section, commands)) in SECTIONS.iter().enumerate() {
                if i > 0 {
                    ui.add_space(10.0);
                }
                widgets::heading(ui, section, &[]);
                ui.add_space(4.0);
                for c in commands.iter() {
                    let (Some(name), Some(send)) = (c.name, c.send) else { continue };
                    let row = widgets::row(ui, MENU_ROW);
                    let response = ui.interact(row, ui.scope_id().with(c.key), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(row, 3.0, HOVER);
                    }
                    let galley = ui.painter().layout_no_wrap(name.to_string(), font(SANS, 12.0), FG);
                    ui.painter().galley(Pos2::new(row.left() + 6.0, text_top(ui.painter(), font(SANS, 12.0), row.center().y)), galley, FG);
                    widgets::key_cap(ui, Pos2::new(row.right() - 6.0, row.center().y), c.key, false, DIM);
                    if response.clicked() {
                        chosen = Some(send);
                    }
                }
            }
        });
    });
    (drawn, chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a key's name, from the key itself: what the table shows must be what the menu sends.
    fn shown((key, mods): (Key, Mods)) -> String {
        let name = match key {
            Key::Space => "space".to_string(),
            Key::Escape => "esc".to_string(),
            Key::Backspace => "backspace".to_string(),
            Key::Home => "home".to_string(),
            Key::End => "end".to_string(),
            Key::Left => "left".to_string(),
            Key::Right => "right".to_string(),
            Key::Up => "↑".to_string(),
            Key::Down => "↓".to_string(),
            Key::Digit0 => "0".to_string(),
            Key::Digit1 => "1".to_string(),
            Key::Digit2 => "2".to_string(),
            Key::Minus => "-".to_string(),
            Key::Equals => "=".to_string(),
            Key::BracketLeft => "[".to_string(),
            Key::BracketRight => "]".to_string(),
            // shift + slash is the question mark
            Key::Slash if mods == SHIFT => return "?".to_string(),
            letter => format!("{letter:?}"),
        };
        let held = [(mods.ctrl, "ctrl "), (mods.shift, "shift "), (mods.alt, "alt ")];
        held.iter().filter(|(on, _)| *on).map(|(_, word)| *word).collect::<String>() + &name
    }

    #[test]
    fn every_key_is_in_the_table_once_and_shown_as_it_is_sent() {
        let mut sent = Vec::new();
        let mut keys = Vec::new();
        for c in commands() {
            assert!(!keys.contains(&c.key), "{} is in the table twice", c.key);
            keys.push(c.key);
            if let Some(send) = c.send {
                assert!(!sent.contains(&send), "{send:?} is in the table twice");
                sent.push(send);
                assert_eq!(shown(send), c.key, "the row shows {} and sends {send:?}", c.key);
            }
        }
    }

    /// the menu offers every command that has a key and nothing else, and a row sends the key it shows.
    #[test]
    fn the_menus_rows_send_the_key_they_show() {
        let rows: Vec<_> = commands().filter_map(|c| Some((c.name?, c.key, c.send?))).collect();
        assert_eq!(rows.len(), 46);
        for (name, key, send) in rows {
            assert_eq!(shown(send), key, "{name}");
            assert_eq!(named(send.0, send.1), Some((name.to_string(), key.to_string())));
        }
        assert!(commands().filter(|c| c.name.is_some()).all(|c| c.send.is_some()), "a row that sends nothing");
    }

    /// the design's table (fig. 21) with what this build does not have left out (markup, a folder's clips), its
    /// words put right where the build differs, and other keys for the same commands beside theirs.
    #[test]
    fn the_table_of_keys_is_the_designs_as_built() {
        let keys: Vec<String> = lines().into_iter().map(|(keys, _)| keys).collect();
        let wanted = [
            "space", "F G left right", "shift F G home end", "J K L", "wheel", "shift wheel", "0 1 2", "- =", "double-click", "alt arrows", "O", "N", "[ ]", "R", "A", "shift - =", "T", "I", "D C", "H V Q", "S shift S", "M ↑ ↓", "P shift P", "right-click", "?", "esc",
        ];
        assert_eq!(keys, wanted);
        assert_eq!(line(Key::Equals, SHIFT), Some(("slower / faster".to_string(), "shift - =".to_string())));
        assert_eq!(line(Key::M, NONE), Some(("mute · volume".to_string(), "M ↑ ↓".to_string())));
    }

    /// the fonts carry every letter the table, the menu and the tooltips use, and every row fits the menu.
    #[test]
    fn every_name_and_key_can_be_drawn_and_every_row_fits_the_menu() {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for c in commands() {
                for (text, font_id) in [(c.key, font(MONO_MEDIUM, 10.0)), (c.key, font(MONO_MEDIUM, 11.0)), (c.name.unwrap_or(""), font(SANS, 12.0)), (c.name.unwrap_or(""), font(MONO_MEDIUM, 11.0)), (c.line.unwrap_or(""), font(MONO, 11.0))] {
                    assert!(ui.ctx().fonts_mut(|f| f.has_glyphs(&font_id, text)), "{font_id:?} cannot draw {text}");
                }
                if let Some(name) = c.name {
                    let name_wide = ui.painter().layout_no_wrap(name.to_string(), font(SANS, 12.0), FG).size().x;
                    let cap_wide = ui.painter().layout_no_wrap(c.key.to_string(), font(MONO_MEDIUM, 10.0), DIM).size().x + 10.0;
                    // the menu's border and padding are 26, the row's insets 6 each side, and 12 at least between
                    assert!(6.0 + name_wide + 12.0 + cap_wide + 6.0 <= MENU_WIDTH - 26.0, "{name} with {} does not fit", c.key);
                }
            }
        });
        out.textures_delta.clear();
    }
}
