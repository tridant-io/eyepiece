//! every command that has a key, in one table. help's keys page (`?`, fig. 39), the right-click menu and the tooltips
//! on the strip's and the bar's buttons are all read from it, so they say the same. it holds only keys that work.

use super::{font, text_top, widgets, ACCENT, DIM, FG, HOVER, LINE, SANS};
use crate::event::{Key, Mods};
use egui::{Align2, Pos2, Rect, Sense};

pub const NONE: Mods = Mods { shift: false, alt: false, ctrl: false };
pub const SHIFT: Mods = Mods { shift: true, alt: false, ctrl: false };
pub const CTRL: Mods = Mods { shift: false, alt: false, ctrl: true };
const ALT: Mods = Mods { shift: false, alt: true, ctrl: false };
pub const CTRL_SHIFT: Mods = Mods { shift: true, alt: false, ctrl: true };

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
/// change). the left and right arrows and ↔ are not in the type, so
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
            both("previous / next clip", "previous clip of the folder", "ctrl left", Key::Left, CTRL),
            both("previous / next clip", "next clip of the folder", "ctrl right", Key::Right, CTRL),
            menu_only("close tab", "ctrl W", Key::W, CTRL),
            menu_only("reopen closed tab", "ctrl shift T", Key::T, CTRL_SHIFT),
        ],
    ),
    (
        "loop and speed",
        &[
            both("loop off · one · all", "loop off · one · all", "R", Key::R, NONE),
            both("loop analysis", "loop analysis", "A", Key::A, NONE),
            both("slower / faster", "slower", "shift -", Key::Minus, SHIFT),
            both("slower / faster", "faster", "shift =", Key::Equals, SHIFT),
            menu_only("1× speed", "backspace", Key::Backspace, NONE),
        ],
    ),
    (
        "trim",
        &[
            both("set in / set out", "set in", "shift [", Key::BracketLeft, SHIFT),
            both("set in / set out", "set out", "shift ]", Key::BracketRight, SHIFT),
            both("clear in and out", "clear in and out", "shift backspace", Key::Backspace, SHIFT),
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
            both("markup", "markup mode", "E", Key::E, NONE),
            menu_only("take back a change to the marks", "ctrl Z", Key::Z, CTRL),
            menu_only("bring back a change taken back", "ctrl shift Z", Key::Z, CTRL_SHIFT),
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
            both("help, on this page", "help", "?", Key::Slash, SHIFT),
            both("settings", "settings", "ctrl ,", Key::Comma, CTRL),
            also("leave fullscreen, close", "esc", Key::Escape, NONE),
        ],
    ),
];

// the right-click menu: as wide as the bar's menus, a row 24 high
/// two columns of the design's 268 menu, with a gap between
const MENU_WIDTH: f32 = 520.0;
const MENU_GAP: f32 = 16.0;
const MENU_ROW: f32 = 24.0;

fn commands() -> impl Iterator<Item = &'static Command> {
    SECTIONS.iter().flat_map(|(_, commands)| commands.iter())
}

/// the table of keys, line by line: the entries on each line, and what it says.
fn grouped() -> Vec<(Vec<&'static Command>, &'static str)> {
    grouped_of(commands())
}

/// `commands` line by line, as `grouped` lays out the whole table.
fn grouped_of(commands: impl Iterator<Item = &'static Command>) -> Vec<(Vec<&'static Command>, &'static str)> {
    let mut lines: Vec<(Vec<&'static Command>, &'static str)> = Vec::new();
    for c in commands {
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

/// the table of keys by section, for help's keys page: each section's name, and its lines' keys and words.
pub fn sections() -> Vec<(&'static str, Vec<(String, &'static str)>)> {
    SECTIONS.iter().map(|(name, commands)| (*name, grouped_of(commands.iter()).into_iter().map(|(on, words)| (joined(&on), words)).collect())).collect()
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

/// the right-click menu: a field that finds a command, then every command with its key beside it, in sections, in two
/// columns. its corner is at `corner` and it is kept inside `room` by its size when it was `last` drawn; it scrolls
/// when the room is too short. `opened` starts it at its top with the field empty and taking the keys, so typing at
/// once finds; `find` is what has been typed. enter runs the first command found. over a `still` it leaves out
/// what has to do with time, as the bar does, but for play and the loop over one a played folder has `held` for a
/// time. returns where it was drawn, and the key of the row chosen.
pub fn menu(ui: &egui::Ui, corner: Pos2, room: Rect, last: Rect, opened: bool, find: &mut String, still: bool, held: bool) -> (Rect, Option<(Key, Mods)>) {
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
        // the field: outlined in the line's color, and in the accent while it has the keys, as a menu's field is
        let (field, _) = ui.allocate_exact_size(egui::Vec2::new(ui.available_width(), 26.0), Sense::hover());
        let id = egui::Id::unique("commands-find");
        let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(field.shrink2(egui::Vec2::new(8.0, 5.0))));
        let edit = inside.add(egui::TextEdit::singleline(find).id(id).frame(egui::Frame::NONE).margin(egui::Margin::ZERO).font(font(SANS, 12.0)).text_color(FG).hint_text("find a command").desired_width(field.width() - 16.0));
        if opened {
            edit.request_focus();
        }
        ui.painter().rect_stroke(field, 0.0, (1.0, if edit.has_focus() { ACCENT } else { LINE }), egui::StrokeKind::Inside);
        let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(10.0);
        // what is typed finds a command by what it does or by its key
        let wanted = find.trim().to_lowercase();
        let found = |c: &Command| c.name.is_some() && c.send.is_some() && (wanted.is_empty() || c.key.to_lowercase().contains(&wanted) || c.name.is_some_and(|n| n.contains(&wanted)));
        let kept = |c: &Command| held && (c.send == Some((Key::Space, NONE)) || c.send == Some((Key::R, NONE)));
        let timeless = |section: &str, c: &Command| still && (matches!(section, "play" | "loop and speed" | "trim" | "sound") || c.send == Some((Key::T, NONE))) && !kept(c);
        let shown: Vec<(&str, Vec<&Command>)> = SECTIONS.iter().map(|(section, commands)| (*section, commands.iter().filter(|c| found(c) && !timeless(section, c)).collect::<Vec<_>>())).filter(|(_, commands)| !commands.is_empty()).collect();
        if entered {
            chosen = shown.first().and_then(|(_, commands)| commands.first()).and_then(|c| c.send);
        }
        // the sections in two columns, split where the left holds about half the rows; a heading counts as one
        let weight: Vec<usize> = shown.iter().map(|(_, commands)| commands.len() + 2).collect();
        let half = weight.iter().sum::<usize>().div_ceil(2);
        let mut split = 0;
        let mut taken = 0;
        while split < shown.len() && taken + weight[split] / 2 < half {
            taken += weight[split];
            split += 1;
        }
        // what was found is one list, as wide as the menu
        let split = if wanted.is_empty() { split } else { shown.len() };
        scroll.show(ui, |ui| {
            if shown.is_empty() {
                ui.label(egui::RichText::new("no command says that").font(font(SANS, 12.0)).color(DIM));
                return;
            }
            let column = if split < shown.len() { (ui.available_width() - MENU_GAP) / 2.0 } else { ui.available_width() };
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = MENU_GAP;
                for part in [&shown[..split], &shown[split..]].into_iter().filter(|part| !part.is_empty()) {
                    ui.vertical(|ui| {
                        ui.set_width(column);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, (section, commands)) in part.iter().enumerate() {
                            if i > 0 {
                                ui.add_space(10.0);
                            }
                            widgets::heading(ui, section, &[]);
                            ui.add_space(4.0);
                            for c in commands {
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
                }
            });
        });
    });
    (drawn, chosen)
}

#[cfg(test)]
mod tests {
    use super::super::MONO_MEDIUM;
    use super::*;

    /// a key's name, from the key itself: what the table shows must be what the menu sends.
    fn shown((key, mods): (Key, Mods)) -> String {
        let name = match key {
            Key::Space => "space".to_string(),
            Key::Escape => "esc".to_string(),
            Key::Backspace => "backspace".to_string(),
            Key::Home => "home".to_string(),
            Key::End => "end".to_string(),
            Key::PageUp => "page up".to_string(),
            Key::PageDown => "page down".to_string(),
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
            Key::Comma => ",".to_string(),
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
        assert_eq!(rows.len(), 55);
        assert_eq!(named(Key::BracketLeft, SHIFT), Some(("set in".to_string(), "shift [".to_string())));
        assert_eq!(named(Key::Backspace, SHIFT), Some(("clear in and out".to_string(), "shift backspace".to_string())));
        assert_eq!(named(Key::Z, CTRL_SHIFT), Some(("bring back a change taken back".to_string(), "ctrl shift Z".to_string())));
        for (name, key, send) in rows {
            assert_eq!(shown(send), key, "{name}");
            assert_eq!(named(send.0, send.1), Some((name.to_string(), key.to_string())));
        }
        assert!(commands().filter(|c| c.name.is_some()).all(|c| c.send.is_some()), "a row that sends nothing");
    }

    /// the design's table (fig. 21, set as help's keys page in fig. 39 with control + , new), its words put right
    /// where the build differs, and other keys for the same commands beside theirs.
    #[test]
    fn the_table_of_keys_is_the_designs_as_built() {
        let keys: Vec<String> = sections().into_iter().flat_map(|(_, lines)| lines.into_iter().map(|(keys, _)| keys)).collect();
        let wanted = [
            "space", "F G left right", "shift F G home end", "J K L", "wheel", "shift wheel", "0 1 2", "- =", "double-click", "alt arrows", "O", "N", "[ ]", "ctrl left right", "R", "A", "shift - =", "shift [ ]", "shift backspace", "T", "I", "D C", "H V Q", "S shift S", "E", "M ↑ ↓", "P shift P", "right-click", "?", "ctrl ,", "esc",
        ];
        assert_eq!(keys, wanted);
        assert_eq!(named(Key::Comma, CTRL), Some(("settings".to_string(), "ctrl ,".to_string())));
        assert_eq!(line(Key::Equals, SHIFT), Some(("slower / faster".to_string(), "shift - =".to_string())));
        assert_eq!(line(Key::M, NONE), Some(("mute · volume".to_string(), "M ↑ ↓".to_string())));
        // the trim's section stands after loop and speed, the other commands about time
        let names: Vec<&str> = sections().into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["play", "zoom", "tabs", "loop and speed", "trim", "view", "screenshot", "sound", "window"]);
    }

    /// the fonts carry every letter the table, the menu and the tooltips use, and every row fits the menu.
    #[test]
    fn every_name_and_key_can_be_drawn_and_every_row_fits_the_menu() {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for c in commands() {
                for (text, font_id) in [(c.key, font(MONO_MEDIUM, 10.0)), (c.key, font(MONO_MEDIUM, 11.0)), (c.name.unwrap_or(""), font(SANS, 12.0)), (c.name.unwrap_or(""), font(MONO_MEDIUM, 11.0)), (c.line.unwrap_or(""), font(SANS, 12.0))] {
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
