//! the panels, settings and help: drawn in the window over whatever is up, as the design's section 11 has them
//! (docs/design/settings.html, figs. 33 to 40). a sidebar of pages, then the page: its title, its rows, a line at its
//! foot. a change takes effect at once (`Action::Settings`), and the panel says what is fixed rather than offer it.

use super::brand::{self, Art};
use super::icons::{self, Icon};
use super::widgets::{self, CHOSEN, ON_CHOSEN};
use super::{dotted, font, keys, text_top, Action, Screen, Ui, ACCENT, DIM, FG, FILL, HOVER, LINE, MONO, MONO_MEDIUM, SANS, SOLID, STRIP, TRACK};
use crate::settings::{Chroma, Loop, Opens, Settings};
use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, FontId, Galley, Pos2, Rect, Sense, Vec2};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const COMMIT: &str = env!("EYEPIECE_COMMIT");
const BUILT: &str = env!("EYEPIECE_BUILT");
const PLATFORM: &str = if cfg!(windows) { "windows" } else { "macos" };
const ARCH: &str = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
pub const TRIDANT: &str = "https://tridant.io";
const SOURCE: &str = "https://github.com/tridant-io/eyepiece";
const ISSUES: &str = "https://github.com/tridant-io/eyepiece/issues";
const RELEASES: &str = "https://github.com/tridant-io/eyepiece/releases";
/// the license's text, when no copy stands beside the program
const LICENSE: &str = "https://spdx.org/licenses/CPAL-1.0.html";
/// what eyepiece is, in the owner's words (2026-11-24)
const WHAT: &str = "a video review player for video professionals: extremely fast, lightweight, and only what you need.";

/// the program's icon on about: twice the design's 64 (the owner's word, 2026-11-24)
const ICON: f32 = 128.0;
/// the panel at the design's scale; its sidebar's width, and its page's head and foot
const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 500.0;
const SIDE: f32 = 176.0;
const HEAD: f32 = 44.0;
const FOOT: f32 = 40.0;
/// the picture behind a panel, darkened to 60%
const SHADE: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 153);
const SIDE_FILL: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
/// the sidebar's page that is up, rgba(255,255,255,.08)
const PAGE_ON: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
/// a group's name over its rows; the rule between two rows, rgba(255,255,255,.07)
const GROUP: Color32 = Color32::from_rgb(0x77, 0x77, 0x77);
const RULE: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
const FOOT_INK: Color32 = Color32::from_rgb(0x8a, 0x8a, 0x8a);
/// a button that cannot be pressed now
const OFF: Color32 = Color32::from_rgb(0x5f, 0x5f, 0x5f);
/// "fixed": its outline and its letters
const TAG_LINE: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);
const TAG_INK: Color32 = Color32::from_rgb(0x88, 0x88, 0x88);
/// the notices' text box; a match found in it, rgba(255,255,255,.22); the rule of = drawn as a line, at .14
const PAPER: Color32 = Color32::from_rgb(0x0f, 0x0f, 0x0f);
const FOUND: Color32 = Color32::from_rgba_premultiplied(56, 56, 56, 56);
const TEXT_RULE: Color32 = Color32::from_rgba_premultiplied(36, 36, 36, 36);
/// the brand kit's night ink: the wordmark on about, the one brand color in the window
const NIGHT: Color32 = Color32::from_rgb(0xe2, 0xe8, 0xf4);
/// the guide color's swatch ring, rgba(255,255,255,.35)
const RING: Color32 = Color32::from_rgba_premultiplied(89, 89, 89, 89);
/// the grays offered behind the picture, after black (the design's proposals)
const BEHIND: [[u8; 3]; 3] = [[0, 0, 0], [0x26, 0x26, 0x26], [0x80, 0x80, 0x80]];
/// a line of the notices, 11 at 1.55
const NOTICE_LINE: f32 = 11.0 * 1.55;

/// which panel
#[derive(Clone, Copy, Debug, PartialEq, Hash)]
pub enum Which {
    Settings,
    Help,
}

const SETTINGS: [&str; 7] = ["general", "about", "playback", "display", "browser", "screenshots", "open-source notices"];
const HELP: [&str; 2] = ["keys", "what eyepiece does"];
/// settings' about page, which the mark on the strip opens
pub const ABOUT: usize = 1;

pub struct Panel {
    pub which: Which,
    pub(super) page: usize,
    /// where it was drawn last: a press outside it closes it
    pub(super) rect: Rect,
    /// custom was chosen behind the picture: its field shows
    custom: bool,
    /// the notices, read when their page first opens
    notices: Option<Notices>,
    /// what is known of the preview cache, measured off the drawing thread when the browser page opens
    cache: CacheFacts,
}

/// the preview cache's folder and the bytes its own files take, or why they are not known
type Measured = Result<(PathBuf, u64), String>;

enum CacheFacts {
    Unasked,
    Asked(mpsc::Receiver<Measured>),
    Known(Measured),
}

impl Panel {
    pub fn new(which: Which, page: usize) -> Panel {
        Panel { which, page, rect: Rect::NOTHING, custom: false, notices: None, cache: CacheFacts::Unasked }
    }

    fn pages(&self) -> &'static [&'static str] {
        match self.which {
            Which::Settings => &SETTINGS,
            Which::Help => &HELP,
        }
    }

    /// the page before or after, as far as the first and the last.
    pub fn turn(&mut self, by: isize) {
        self.page = (self.page as isize + by).clamp(0, self.pages().len() as isize - 1) as usize;
    }
}

/// the panel's place: 760 x 500 in the middle of the window, or the window less 16 at each side when it is smaller,
/// and never over the strip, whose buttons stay to be seen lit
fn place(full: Rect) -> Rect {
    let size = Vec2::new(WIDTH.min(full.width() - 32.0), HEIGHT.min(full.height() - STRIP - 32.0)).max(Vec2::splat(1.0));
    let top = (full.center().y - size.y / 2.0).max(full.top() + STRIP + 16.0);
    Rect::from_min_size(Pos2::new(full.center().x - size.x / 2.0, top).round(), size)
}

impl Ui {
    /// the panel that is up, over whatever is up: the picture darkened under the strip, which stays, and the panel in
    /// the middle. a press outside it closes it and does nothing else, since the panel's layer, over the whole
    /// window, takes it. what its controls change is asked for as new settings.
    pub(super) fn panel_over(&mut self, ui: &mut egui::Ui, full: Rect, screen: &Screen, actions: &mut Vec<Action>) {
        let Some(mut panel) = self.panel.take() else { return };
        if ui.input(|i| i.pointer.any_pressed()) && panel.rect.is_positive() && !ui.ctx().pointer_latest_pos().is_some_and(|p| panel.rect.contains(p)) {
            return;
        }
        let mut settings = screen.settings.clone();
        let mut close = false;
        let shown = egui::Area::new(egui::Id::unique("panel")).order(egui::Order::Foreground).fixed_pos(full.min).show(ui.ctx(), |ui| {
            ui.allocate_exact_size(full.size(), Sense::click());
            ui.painter().rect_filled(Rect::from_min_max(Pos2::new(full.left(), full.top() + STRIP), full.max), 0.0, SHADE);
            panel.rect = place(full);
            close = self.panel_drawn(ui, &mut panel, &mut settings, actions);
        });
        // over the menus and fields of what is under it, which are areas of the same order
        ui.ctx().move_to_top(shown.response.layer_id);
        if settings != *screen.settings {
            // what the controls keep themselves follows at once
            (self.pinned, self.timecode) = (settings.pin_controls, settings.timecode);
            if settings.browser_recent != screen.settings.browser_recent {
                self.browser.set_recent(settings.browser_recent.clone());
            }
            actions.push(Action::Settings(Box::new(settings)));
        }
        if !close {
            self.panel = Some(panel);
        }
    }

    /// the panel at `panel.rect`: its sidebar, its page's head, body and foot. true when its close mark was pressed.
    fn panel_drawn(&mut self, ui: &mut egui::Ui, panel: &mut Panel, s: &mut Settings, actions: &mut Vec<Action>) -> bool {
        let rect = panel.rect;
        let painter = ui.painter().clone();
        painter.add(egui::Shadow { offset: [0, 10], blur: 28, spread: 0, color: Color32::from_black_alpha(128) }.as_shape(rect, 0.0));
        painter.rect_filled(rect, 0.0, SOLID);
        // the sidebar: the panel's name, its pages, and the version at its foot
        let side = Rect::from_min_max(rect.min, Pos2::new(rect.left() + SIDE, rect.bottom()));
        painter.rect_filled(side, 0.0, SIDE_FILL);
        painter.rect_filled(Rect::from_min_max(Pos2::new(side.right() - 1.0, side.top()), side.max), 0.0, LINE);
        spaced(&painter, Pos2::new(side.left() + 14.0, side.top() + 14.0), if panel.which == Which::Help { "help" } else { "settings" }, GROUP);
        let mut y = side.top() + 14.0 + 9.5 + 8.0;
        for (i, name) in panel.pages().iter().enumerate() {
            // a page's name: 12.5 at 1.3, 6 above and below, 1 between
            let item = Rect::from_min_size(Pos2::new(side.left(), y), Vec2::new(SIDE - 1.0, 28.0));
            if ui.interact(item, ui.scope_id().with(("panel-page", i)), Sense::click()).clicked() {
                panel.page = i;
            }
            let on = i == panel.page;
            if on {
                painter.rect_filled(item, 0.0, PAGE_ON);
            }
            let color = if on { FG } else { DIM };
            painter.galley(Pos2::new(item.left() + 14.0, text_top(&painter, font(SANS, 12.5), item.center().y)), painter.layout_no_wrap(name.to_string(), font(SANS, 12.5), color), color);
            y += 29.0;
        }
        painter.text(Pos2::new(side.left() + 14.0, side.bottom() - 12.0), Align2::LEFT_BOTTOM, format!("eyepiece {VERSION}"), font(MONO, 10.5), DIM);

        // the page's head: its title, its keys, its close mark
        let page = Rect::from_min_max(Pos2::new(side.right(), rect.top()), rect.max);
        let head = Rect::from_min_size(page.min, Vec2::new(page.width(), HEAD));
        let title = painter.layout_no_wrap(panel.pages()[panel.page].to_string(), font("sans-semibold", 14.0), FG);
        painter.galley(Pos2::new(head.left() + 20.0, text_top(&painter, font("sans-semibold", 14.0), head.center().y)), title, FG);
        let mark = Rect::from_center_size(Pos2::new(head.right() - 8.0 - 14.0, head.center().y), Vec2::splat(28.0));
        let closed = ui.interact(mark, ui.scope_id().with("panel-close"), Sense::click()).clicked();
        icons::draw(&painter, mark, Icon::Close, DIM);
        let mut right = mark.left() - 8.0;
        for key in if panel.which == Which::Help { &["?", "esc"][..] } else { &["esc"] }.iter().rev() {
            right = widgets::key_cap(ui, Pos2::new(right, head.center().y), key, false, DIM).left() - 8.0;
        }
        painter.rect_filled(Rect::from_min_size(Pos2::new(head.left(), head.bottom() - 1.0), Vec2::new(head.width(), 1.0)), 0.0, LINE);

        // about alone has no foot: it has nothing to reset, and saying so says nothing
        let footless = panel.which == Which::Settings && panel.page == ABOUT;
        let foot = Rect::from_min_max(Pos2::new(page.left(), page.bottom() - if footless { 0.0 } else { FOOT }), page.max);
        let body = Rect::from_min_max(Pos2::new(page.left() + 20.0, head.bottom() + 2.0), Pos2::new(page.right() - 20.0, foot.top() - 8.0));
        let (reset, note) = self.page(ui, panel, body, s, actions);
        if footless {
            return closed;
        }

        // the foot: "reset this page" where the page has one, then what is fixed or where the page comes from
        painter.rect_filled(Rect::from_min_size(foot.min, Vec2::new(foot.width(), 1.0)), 0.0, LINE);
        let mut left = foot.left() + 20.0;
        if reset {
            let words = painter.layout_no_wrap("reset this page".to_string(), font(SANS, 12.0), DIM);
            let at = Pos2::new(left, text_top(&painter, font(SANS, 12.0), foot.center().y));
            let hit = Rect::from_min_size(Pos2::new(at.x, foot.center().y - 9.0), Vec2::new(words.size().x, 18.0));
            painter.rect_filled(Rect::from_min_size(Pos2::new(at.x, (foot.center().y + 6.0).round()), Vec2::new(words.size().x, 1.0)), 0.0, Color32::from_white_alpha(71));
            painter.galley(at, words, DIM);
            if ui.interact(hit, ui.scope_id().with(("panel-reset", panel.page)), Sense::click()).clicked() {
                self.reset(panel, s);
            }
            left = foot.right() - 20.0 - painter.layout_no_wrap(note.to_string(), font(SANS, 11.5), FOOT_INK).size().x;
        }
        painter.galley(Pos2::new(left, text_top(&painter, font(SANS, 11.5), foot.center().y)), painter.layout_no_wrap(note.to_string(), font(SANS, 11.5), FOOT_INK), FOOT_INK);
        closed
    }

    /// the page's body, in `body`. returns whether the page can be reset, and what its foot says.
    fn page(&mut self, ui: &mut egui::Ui, panel: &mut Panel, body: Rect, s: &mut Settings, actions: &mut Vec<Action>) -> (bool, &'static str) {
        let id = (panel.which, panel.page);
        match id {
            (Which::Settings, 0) => {
                scrolled(ui, body, id, |ui| general(ui, s, actions));
                (true, "changes take effect at once")
            }
            (Which::Settings, 1) => {
                let check_at_start = s.update_check;
                scrolled(ui, body, id, |ui| self.about(ui, body.height(), check_at_start, actions));
                (false, "")
            }
            (Which::Settings, 2) => {
                scrolled(ui, body, id, |ui| playback(ui, s));
                (true, "space, and a click on the picture, play and pause: fixed")
            }
            (Which::Settings, 3) => {
                scrolled(ui, body, id, |ui| display(ui, panel, s));
                (true, "past 100% a pixel is a square, never smoothed: fixed")
            }
            (Which::Settings, 4) => {
                let shows = self.browser.folder().to_path_buf();
                scrolled(ui, body, id, |ui| browser(ui, panel, s, shows));
                (true, "reset keeps the recent folders and the cache")
            }
            (Which::Settings, 5) => {
                // never a folder of media: screenshots are not written beside it
                let shows = Some(self.browser.folder().to_path_buf()).filter(|_| self.browser.clear_of_media());
                scrolled(ui, body, id, |ui| screenshots(ui, s, shows, actions));
                (true, "the screenshot menu has the same three switches")
            }
            (Which::Settings, _) => {
                notices(ui, panel, body, actions);
                (false, "THIRD-PARTY-NOTICES.txt, then ffmpeg-LICENSE.txt, as installed beside the program")
            }
            (Which::Help, 0) => {
                scrolled(ui, body, id, keys_page);
                (false, "the same list fills the right-click menu and the buttons' hints")
            }
            (Which::Help, _) => {
                scrolled(ui, body, id, |ui| what(ui, actions));
                (false, "the readme on github says the same at length")
            }
        }
    }

    /// puts the page's settings back to their defaults. the browser page keeps the recent folders and the cache.
    fn reset(&mut self, panel: &mut Panel, s: &mut Settings) {
        let d = Settings::default();
        match panel.page {
            0 => {
                (s.play_on_open, s.browser_opens_in, s.one_window, s.pin_controls) = (d.play_on_open, d.browser_opens_in, d.one_window, d.pin_controls);
                (s.controls_after, s.remember_size, s.update_check) = (d.controls_after, d.remember_size, d.update_check);
            }
            2 => {
                (s.loop_file, s.loop_folder, s.loop_analysis_seconds, s.timecode) = (d.loop_file, d.loop_folder, d.loop_analysis_seconds, d.timecode);
                (s.first_frame, s.volume, s.mute) = (d.first_frame, d.volume, d.mute);
            }
            3 => {
                (s.background, s.chroma, s.grid, s.tile, s.center, s.guide_color) = (d.background, d.chroma, d.grid, d.tile, d.center, d.guide_color);
                panel.custom = false;
            }
            4 => (s.browser_start, s.cache_limit) = (d.browser_start, d.cache_limit),
            5 => (s.screenshot_folder, s.screenshot_caption, s.screenshot_guides, s.screenshot_markup) = (d.screenshot_folder, d.screenshot_caption, d.screenshot_guides, d.screenshot_markup),
            _ => {}
        }
    }

    /// about (fig. 34): the program's icon, its name, version and build, what it is and whose, the links, the
    /// license, and the wordmark at the foot. `room` is the body's height, the foot standing at its end when it fits.
    fn about(&mut self, ui: &mut egui::Ui, room: f32, check_at_start: bool, actions: &mut Vec<Action>) {
        let width = ui.available_width();
        let painter = ui.painter().clone();
        ui.add_space(16.0);
        // the icon, and beside it what the program is: the column's lines with the room above each
        let column = width - ICON - 20.0;
        let mut version = LayoutJob::default();
        version.append(VERSION, 0.0, TextFormat { font_id: font(MONO_MEDIUM, 12.0), color: FG, ..Default::default() });
        version.append(&format!(" · {COMMIT}"), 0.0, TextFormat { font_id: font(MONO_MEDIUM, 12.0), color: DIM, ..Default::default() });
        let mut system = LayoutJob::default();
        for (words, color) in [("a ", DIM), ("tridant", FG), (" system", DIM)] {
            system.append(words, 0.0, TextFormat { font_id: font(SANS, 12.0), color, ..Default::default() });
        }
        let lines = [
            (painter.layout_no_wrap("eyepiece".to_string(), font("sans-semibold", 22.0), FG), 0.0),
            (painter.layout_job(version), 9.0),
            (painter.layout_no_wrap(format!("built {BUILT} · {PLATFORM} {ARCH}"), font(SANS, 12.0), DIM), 6.0),
            (painter.layout_job(wrapped(WHAT, font(SANS, 12.0), DIM, column, 1.35)), 6.0),
            (painter.layout_job(system), 9.0),
        ];
        let height: f32 = lines.iter().map(|(galley, above)| galley.size().y + above).sum();
        let (block, _) = ui.allocate_exact_size(Vec2::new(width, height.max(ICON)), Sense::hover());
        let (icon, pixels) = self.on_pixels(Pos2::new(block.left() + ICON / 2.0, block.center().y), Vec2::splat(ICON));
        painter.image(self.pictures.get(ui.ctx(), Art::Icon, pixels), icon, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let mut top = block.center().y - height / 2.0;
        let left = block.left() + ICON + 20.0;
        let mut whose = Rect::NOTHING;
        for (galley, above) in lines {
            top += above;
            whose = Rect::from_min_size(Pos2::new(left, top), galley.size());
            top += galley.size().y;
            painter.galley(whose.min, galley, FG);
        }
        // "a tridant system", the brand's line, with the small star after it: a press opens tridant.io
        let (star, pixels) = self.on_pixels(Pos2::new(whose.right() + 6.0 + 5.5, whose.center().y), Vec2::splat(11.0));
        brand::draw(&painter, self.pictures.get(ui.ctx(), Art::Star, pixels), star, 0.0, DIM);
        if ui.interact(whose.union(star), ui.scope_id().with("about-system"), Sense::click()).clicked() {
            actions.push(Action::OpenUrl(TRIDANT.to_string()));
        }
        // the update line (fig. 34), between what the program is and its links
        ui.add_space(14.0);
        self.update_line(ui, check_at_start, actions);
        ui.add_space(12.0);
        if let Some(url) = links(ui, "about-links", &[("tridant.io", TRIDANT), ("source on github", SOURCE), ("report a problem", ISSUES), ("release notes", RELEASES)]) {
            actions.push(Action::OpenUrl(url.to_string()));
        }
        ui.add_space(12.0);
        if sentence(ui, "about-license", "eyepiece is under the common public attribution license 1.0. ", "read it", " · its parts' own licenses are under open-source notices.") {
            actions.push(Action::OpenUrl(license()));
        }
        // the wordmark alone at the foot, apart from the line that names tridant, and the copyright at the right
        let used = ui.min_rect().height();
        ui.add_space((room - used - 12.0 - 12.0).max(12.0));
        let (foot, _) = ui.allocate_exact_size(Vec2::new(width, 12.0), Sense::hover());
        let (mark, pixels) = self.on_pixels(Pos2::new(foot.left() + 34.5, foot.center().y), Vec2::new(69.0, 12.0));
        brand::draw(&painter, self.pictures.get(ui.ctx(), Art::Wordmark, pixels), mark, 0.0, NIGHT);
        painter.text(foot.right_bottom(), Align2::RIGHT_BOTTOM, "© 2026 Tridant, Inc.", font(SANS, 11.5), DIM);
        ui.add_space(12.0);
    }
}

/// what "read it" opens: the license beside the program, when there is one, or its text on spdx's site
fn license() -> String {
    let beside = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("LICENSE")));
    beside.filter(|file| file.is_file()).map_or(LICENSE.to_string(), |file| file.to_string_lossy().into_owned())
}

/// `add` in a column that scrolls when the window is too short for it, its place kept for each page
fn scrolled<R>(ui: &mut egui::Ui, body: Rect, id: impl std::hash::Hash + std::fmt::Debug, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(Align::Min)));
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .max_height(body.height())
        .show(&mut inner, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            add(ui)
        })
        .inner
}

/// general (fig. 33)
fn general(ui: &mut egui::Ui, s: &mut Settings, actions: &mut Vec<Action>) {
    let mut rows = Rows::default();
    toggle(rows.row(ui, "autoplay", "play a file when it opens", words("off: a file opens paused on its first frame"), &[Part::Switch(s.play_on_open)]), &mut s.play_on_open);
    let opens = [Opens::ThisTab, Opens::NewTab];
    if let Some((_, Got::Chose(i))) = rows.row(ui, "opens", "open a file from the browser in", None, &[Part::Choices(&["this tab", "a new tab"], opens.iter().position(|o| *o == s.browser_opens_in))]) {
        s.browser_opens_in = opens[i];
    }
    toggle(rows.row(ui, "one-window", "one window", words("a second launch opens its file in a tab here"), &[Part::Switch(s.one_window)]), &mut s.one_window);
    toggle(rows.row(ui, "pin", "keep the controls up", words("the pin at the bar's left does the same"), &[Part::Switch(s.pin_controls)]), &mut s.pin_controls);
    if let Some(to) = typed(rows.row(ui, "after", "controls go after", words("once the pointer rests off them"), &[Part::Field(format!("{:.1}", s.controls_after), "s", 46.0, number)])) {
        s.controls_after = to.clamp(1.0, 10.0);
    }
    toggle(rows.row(ui, "size", "remember the window's size", None, &[Part::Switch(s.remember_size)]), &mut s.remember_size);
    toggle(rows.row(ui, "updates", "check for updates when eyepiece starts", words("one request to github, at most once a day. nothing else is sent."), &[Part::Switch(s.update_check)]), &mut s.update_check);
    let file = crate::settings::file();
    let shown = file.as_ref().map_or_else(|| "the system names no settings folder".to_string(), |file| file.display().to_string());
    if rows.row(ui, "file", "the settings file", Some(Note::Path(&shown)), &[Part::Button("reveal", file.is_some())]).is_some() {
        actions.extend(file.map(Action::Reveal));
    }
}

/// playback (fig. 37)
fn playback(ui: &mut egui::Ui, s: &mut Settings) {
    let mut rows = Rows::default();
    let loops = [Loop::Off, Loop::One, Loop::All];
    rows.group(ui, "loop");
    if let Some((_, Got::Chose(i))) = rows.row(ui, "file-loop", "loop for a file", None, &[Part::Choices(&["off", "one"], loops.iter().position(|l| *l == s.loop_file))]) {
        s.loop_file = loops[i];
    }
    if let Some((_, Got::Chose(i))) = rows.row(ui, "folder-loop", "loop for a played folder", None, &[Part::Choices(&["off", "one", "all"], loops.iter().position(|l| *l == s.loop_folder))]) {
        s.loop_folder = loops[i];
    }
    if let Some(to) = typed(rows.row(ui, "seam", "loop analysis", words("seconds each side of the seam, 0.5 to 10"), &[Part::Field(format!("{:.1}", s.loop_analysis_seconds), "s", 46.0, number)])) {
        s.loop_analysis_seconds = to.clamp(0.5, 10.0);
    }
    rows.group(ui, "readout");
    if let Some((_, Got::Chose(i))) = rows.row(ui, "count", "count in", words("T, or a click on the readout, switches it in a tab"), &[Part::Choices(&["frames", "timecode"], Some(s.timecode as usize))]) {
        s.timecode = i == 1;
    }
    if let Some((_, Got::Chose(i))) = rows.row(ui, "first", "frame numbers start at", words("the readout, the title, captions and screenshot names"), &[Part::Choices(&["0", "1"], Some(s.first_frame))]) {
        s.first_frame = i;
    }
    rows.group(ui, "sound");
    // the slider and the field set it as the bar's slider does: all the way down is muted
    let volume = match rows.row(ui, "volume", "volume", None, &[Part::Slider(s.volume), Part::Field(format!("{}", (s.volume * 100.0).round()), "%", 32.0, number)]) {
        Some((_, Got::Slid(to))) => Some(to),
        Some((_, Got::Typed(to))) => to.parse::<f32>().ok().map(|percent| percent.clamp(0.0, 100.0) / 100.0),
        _ => None,
    };
    if let Some(to) = volume {
        s.volume = (to * 100.0).round() / 100.0;
        s.mute = s.volume <= 0.0;
    }
    toggle(rows.row(ui, "mute", "mute", None, &[Part::Switch(s.mute)]), &mut s.mute);
}

/// display (fig. 37), less "transparent parts", which is not built
fn display(ui: &mut egui::Ui, panel: &mut Panel, s: &mut Settings) {
    let mut rows = Rows::default();
    rows.group(ui, "around and under the picture");
    let preset = BEHIND.iter().position(|gray| *gray == s.background).filter(|_| !panel.custom);
    let mut parts = vec![Part::Choices(&["black", "dark gray", "mid gray", "custom"], Some(preset.unwrap_or(3)))];
    if preset.is_none() {
        parts.push(Part::Field(hex_of(s.background), "", 50.0, hex));
    }
    match rows.row(ui, "behind", "behind the picture", words("the bars round a picture of another shape"), &parts) {
        Some((0, Got::Chose(3))) => panel.custom = true,
        Some((0, Got::Chose(i))) => (s.background, panel.custom) = (BEHIND[i], false),
        Some((1, Got::Typed(to))) => s.background = crate::settings::color(&to).unwrap_or(s.background),
        _ => {}
    }
    let chromas = [Chroma::Bilinear, Chroma::Nearest];
    if let Some((_, Got::Chose(i))) = rows.row(ui, "chroma", "chroma for 4:2:0 and 4:2:2", words("blended at the file's siting, or the nearest sample"), &[Part::Choices(&["blended", "nearest"], chromas.iter().position(|c| *c == s.chroma))]) {
        s.chroma = chromas[i];
    }
    rows.group(ui, "guides");
    // a tile's side as the view menu takes it
    let side = |typed: &str| typed.parse::<f32>().ok().map(|v| (v.round() as u32).clamp(8, 4096));
    let grid = [Part::Field(s.tile.0.to_string(), "", 32.0, number), Part::Words("×".to_string()), Part::Field(s.tile.1.to_string(), "px", 32.0, number), Part::Cap("D"), Part::Switch(s.grid)];
    match rows.row(ui, "grid", "grid", words("a tile's size in the clip's pixels"), &grid) {
        Some((0, Got::Typed(to))) => s.tile.0 = side(&to).unwrap_or(s.tile.0),
        Some((2, Got::Typed(to))) => s.tile.1 = side(&to).unwrap_or(s.tile.1),
        Some((4, _)) => s.grid = !s.grid,
        _ => {}
    }
    if let Some((1, _)) = rows.row(ui, "center", "center lines and circle", None, &[Part::Cap("C"), Part::Switch(s.center)]) {
        s.center = !s.center;
    }
    if let Some((1, Got::Typed(to))) = rows.row(ui, "guide-color", "guide color", None, &[Part::Swatch(s.guide_color), Part::Field(hex_of(s.guide_color), "", 50.0, hex)]) {
        s.guide_color = crate::settings::color(&to).unwrap_or(s.guide_color);
    }
}

/// browser (fig. 38). there is no folder picker: the fixed folder is the one the browser shows, `shows`
fn browser(ui: &mut egui::Ui, panel: &mut Panel, s: &mut Settings, shows: PathBuf) {
    let mut rows = Rows::default();
    let showing = !shows.as_os_str().is_empty();
    rows.group(ui, "start");
    match rows.row(ui, "start", "start in", words("the first run opens on downloads"), &[Part::Choices(&["where it was last", "a fixed folder"], Some(s.browser_start.is_some() as usize))]) {
        Some((_, Got::Chose(0))) => s.browser_start = None,
        Some((_, Got::Chose(_))) if s.browser_start.is_none() && showing => s.browser_start = Some(shows.clone()),
        _ => {}
    }
    let fixed = s.browser_start.clone().unwrap_or_else(|| shows.clone()).display().to_string();
    if rows.row(ui, "fixed", "the fixed folder", Some(Note::Path(&fixed)), &[Part::Button("use the folder the browser shows", s.browser_start.is_some() && showing)]).is_some() {
        s.browser_start = Some(shows);
    }
    if rows.row(ui, "recent", "recent folders", words("the last 5 the browser opened files from"), &[Part::Button("clear recent folders", !s.browser_recent.is_empty())]).is_some() {
        s.browser_recent.clear();
    }
    rows.group(ui, "network folders");
    rows.fact(ui, "look-only", "nothing but the listing is read until you open a file or ask for previews");
    rows.group(ui, "preview cache");
    // measured off the drawing thread, which looks again until the answer is in
    if let CacheFacts::Unasked = panel.cache {
        panel.cache = CacheFacts::Asked(measure(false));
    }
    if let CacheFacts::Asked(answer) = &panel.cache {
        match answer.try_recv() {
            Ok(found) => panel.cache = CacheFacts::Known(found),
            Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(Duration::from_millis(40)),
            Err(mpsc::TryRecvError::Disconnected) => panel.cache = CacheFacts::Known(Err("the cache could not be measured".to_string())),
        }
    }
    const GB: f64 = (1u64 << 30) as f64;
    let (note, in_use) = match &panel.cache {
        CacheFacts::Known(Ok((root, used))) => (root.display().to_string(), Some(*used)),
        CacheFacts::Known(Err(why)) => (why.clone(), None),
        _ => ("measuring…".to_string(), None),
    };
    let limit = s.cache_limit as f64 * GB;
    let amount = in_use.map_or_else(|| "…".to_string(), |used| format!("{:.2} of {} GB", used as f64 / GB, s.cache_limit));
    let note = if in_use.is_some() { Note::Path(&note) } else { Note::Words(&note) };
    rows.row(ui, "cache", "posters and previews", Some(note), &[Part::Meter(in_use.map_or(0.0, |used| (used as f64 / limit) as f32)), Part::Words(amount)]);
    let clearing = matches!(panel.cache, CacheFacts::Asked(_));
    match rows.row(ui, "limit", "limit", words("oldest out first. clearing takes its files one by one"), &[Part::Field(s.cache_limit.to_string(), "GB", 32.0, number), Part::Button("clear the cache", !clearing)]) {
        Some((0, Got::Typed(to))) => s.cache_limit = to.parse::<f32>().map_or(s.cache_limit, |gb| gb.clamp(1.0, 64.0)),
        Some((1, _)) => panel.cache = CacheFacts::Asked(measure(true)),
        _ => {}
    }
}

/// the preview cache's folder and what its files take, measured on a thread of its own, after clearing it when
/// asked: clearing takes its own files one at a time, as eviction does
fn measure(clear: bool) -> mpsc::Receiver<Measured> {
    let (tell, answer) = mpsc::channel();
    std::thread::spawn(move || {
        let found = match crate::preview::Cache::open() {
            None => Err("the system names no cache folder".to_string()),
            Some(cache) => {
                let cleared = if clear { cache.clear().map_err(|e| format!("the cache was not all cleared: {e}")) } else { Ok(()) };
                cleared.and_then(|_| Ok((cache.root().to_path_buf(), cache.used().map_err(|e| e.to_string())?)))
            }
        };
        let _ = tell.send(found);
    });
    answer
}

/// screenshots (fig. 38). there is no folder picker: the folder can be the one the browser shows, `shows`
fn screenshots(ui: &mut egui::Ui, s: &mut Settings, shows: Option<PathBuf>, actions: &mut Vec<Action>) {
    let mut rows = Rows::default();
    ui.add_space(6.0);
    let folder = s.screenshot_folder.clone().unwrap_or_else(crate::shot::folder);
    let shown = folder.display().to_string();
    match rows.row(ui, "shots", "folder", Some(Note::Path(&shown)), &[Part::Button("use the folder the browser shows", shows.is_some()), Part::Button("reveal", true)]) {
        Some((0, _)) => s.screenshot_folder = shows,
        Some((1, _)) => actions.push(Action::Reveal(folder)),
        _ => {}
    }
    rows.group(ui, "a saved picture carries");
    toggle(rows.row(ui, "caption", "a caption: name, frame, timecode", words("under the picture, so it covers no pixel"), &[Part::Switch(s.screenshot_caption)]), &mut s.screenshot_caption);
    toggle(rows.row(ui, "guides", "the guides, when they are showing", None, &[Part::Switch(s.screenshot_guides)]), &mut s.screenshot_guides);
    toggle(rows.row(ui, "markup", "the markup", None, &[Part::Switch(s.screenshot_markup)]), &mut s.screenshot_markup);
    rows.group(ui, "always");
    rows.fact(ui, "never beside the media, never over an earlier one", "clip.f000742.png, then clip.f000742 (2).png");
    rows.fact(ui, "16 bits a channel when the file is deeper than 8", "a copy to the clipboard is the frame alone, 8 bits");
}

/// help's keys page (fig. 39): the table of keys in two columns by section, the left's keys 120 wide and the
/// right's 100, lines 17 high
fn keys_page(ui: &mut egui::Ui) {
    ui.add_space(12.0);
    let width = ui.available_width();
    let note = ui.painter().layout_job(wrapped("fixed in this version: none can be changed. on a mac, cmd stands for ctrl.", font(SANS, 11.0), DIM, width, 1.4));
    let (at, _) = ui.allocate_exact_size(note.size(), Sense::hover());
    ui.painter().galley(at.min, note, DIM);
    ui.add_space(2.0 + 4.0);
    let sections = keys::sections();
    let painter = ui.painter().clone();
    let top = ui.cursor().top();
    let mut bottom = top;
    for (column, (from, keys_wide)) in [(&["play", "zoom", "tabs", "sound"][..], (0.0, 120.0)), (&["loop and speed", "view", "screenshot", "window"][..], (274.0 + 20.0, 100.0))] {
        let left = ui.cursor().left() + from;
        let mut y = top;
        for (name, lines) in sections.iter().filter(|(name, _)| column.contains(name)) {
            y += 10.0;
            spaced(&painter, Pos2::new(left, y), name, GROUP);
            y += 9.5 + 4.0;
            for (keys, does) in lines {
                let middle = y + 8.5;
                painter.galley(Pos2::new(left, text_top(&painter, font(MONO_MEDIUM, 10.5), middle)), painter.layout_no_wrap(keys.clone(), font(MONO_MEDIUM, 10.5), FG), FG);
                painter.galley(Pos2::new(left + keys_wide, text_top(&painter, font(SANS, 11.5), middle)), painter.layout_no_wrap(does.to_string(), font(SANS, 11.5), DIM), DIM);
                y += 17.0;
            }
        }
        bottom = bottom.max(y);
    }
    ui.allocate_exact_size(Vec2::new(width, bottom - top + 8.0), Sense::hover());
}

/// help's "what eyepiece does" (fig. 40): what it is, its features in eight lines, the links, the license
fn what(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    const FEATURES: [(&str, &str); 8] = [
        ("opens what review needs", "hap, notchlc, prores, h.264, hevc and vp9, and numbered stills as one clip"),
        ("one true frame at a time", "steps, a timebar with pictures, frames or the file's own timecode"),
        ("true pixels", "1:1 and up drawn square, with the pixel and its values under the pointer"),
        ("what the file is", "a metadata panel read from the file; what it cannot say is labeled"),
        ("loops", "loop analysis plays the end into the start and measures the seam"),
        ("folders", "a browser with scrubbing previews; a folder played in order"),
        ("review", "guides, mirror and rotate, markup, and screenshots with a caption"),
        ("versions side by side", "two tabs of one size flip on the same frame, zoom and pan"),
    ];
    let width = ui.available_width();
    let painter = ui.painter().clone();
    ui.add_space(14.0);
    let intro = format!("{WHAT} it opens a clip at once, steps one true frame at a time, shows true pixels, and says exactly what the file is.");
    let intro = painter.layout_job(wrapped(&intro, font(SANS, 12.5), FG, width.min(500.0), 1.45));
    let (at, _) = ui.allocate_exact_size(Vec2::new(width, intro.size().y), Sense::hover());
    painter.galley(at.min, intro, FG);
    ui.add_space(16.0);
    let column = (width - 22.0) / 2.0;
    for pair in FEATURES.chunks(2) {
        let laid: Vec<_> = pair.iter().map(|(title, words)| (painter.layout_no_wrap(title.to_string(), font("sans-semibold", 12.0), FG), painter.layout_job(wrapped(words, font(SANS, 11.0), DIM, column, 1.4)))).collect();
        let height = laid.iter().map(|(title, words)| title.size().y + 3.0 + words.size().y).fold(0.0, f32::max);
        let (row, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        for (i, (title, words)) in laid.into_iter().enumerate() {
            let left = row.left() + i as f32 * (column + 22.0);
            let under = row.top() + title.size().y + 3.0;
            painter.galley(Pos2::new(left, row.top()), title, FG);
            painter.galley(Pos2::new(left, under), words, DIM);
        }
        ui.add_space(12.0);
    }
    if let Some(url) = links(ui, "what-links", &[("source on github", SOURCE), ("report a problem", ISSUES), ("release notes", RELEASES), ("tridant.io", TRIDANT)]) {
        actions.push(Action::OpenUrl(url.to_string()));
    }
    ui.add_space(12.0);
    if sentence(ui, "what-license", "eyepiece is under the common public attribution license 1.0. ", "read it", "") {
        actions.push(Action::OpenUrl(license()));
    }
    ui.add_space(12.0);
}

/// open-source notices (fig. 36): the notices and ffmpeg's license as installed beside the program, read when the
/// page first opens; a field that finds as it is typed in, with the match it is on lit; and "reveal the file"
fn notices(ui: &mut egui::Ui, panel: &mut Panel, body: Rect, actions: &mut Vec<Action>) {
    let notices = panel.notices.get_or_insert_with(Notices::read);
    let painter = ui.painter().clone();
    let find = Rect::from_min_size(Pos2::new(body.left(), body.top() + 12.0), Vec2::new(body.width(), widgets::ROW));
    // from the right: reveal the file, then how many are found, then the field in what is left
    let reveal = Part::Button("reveal the file", notices.file.is_some());
    let reveal_wide = reveal.width(&painter);
    let at = Rect::from_min_max(Pos2::new(find.right() - reveal_wide, find.top()), find.max);
    if reveal.draw(ui, "notices-reveal", at).is_some() {
        actions.extend(notices.file.clone().map(Action::Reveal));
    }
    let count = if notices.query.is_empty() { String::new() } else { format!("{} of {}", if notices.found.is_empty() { 0 } else { notices.at + 1 }, notices.found.len()) };
    let count = painter.layout_no_wrap(count, font(MONO, 11.0), DIM);
    let count_left = at.left() - 8.0 - count.size().x;
    painter.galley(Pos2::new(count_left, text_top(&painter, font(MONO, 11.0), find.center().y)), count, DIM);
    let field = Rect::from_min_max(find.min, Pos2::new(count_left - 8.0, find.bottom()));
    // the text is laid out by egui, which centers a line by its box: it is lifted by the difference, as `field` does
    let lift = text_top(&painter, font(MONO_MEDIUM, 11.5), 0.0) + painter.layout_no_wrap("0".to_string(), font(MONO_MEDIUM, 11.5), FG).size().y / 2.0;
    let mut inside = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(field.min + Vec2::new(8.0, 1.0 + lift), Vec2::new(field.width() - 16.0, widgets::ROW - 2.0))).layout(egui::Layout::left_to_right(Align::Center)));
    let edit = inside.add(egui::TextEdit::singleline(&mut notices.query).id(ui.scope_id().with("notices-find")).frame(egui::Frame::NONE).font(font(MONO_MEDIUM, 11.5)).text_color(FG).margin(egui::Margin::ZERO).desired_width(field.width() - 16.0).return_key(None));
    if edit.changed() {
        notices.find();
    }
    // enter goes on to the next match, shift + enter back to the one before
    if edit.has_focus() && !notices.found.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        let back = ui.input(|i| i.modifiers.shift);
        notices.at = (notices.at + if back { notices.found.len() - 1 } else { 1 }) % notices.found.len();
        notices.reveal = true;
    }
    painter.rect_stroke(field, 0.0, (1.0, if edit.has_focus() { ACCENT } else { LINE }), egui::StrokeKind::Inside);

    // the text, in its box, of which only what is in view is drawn
    let text = Rect::from_min_max(Pos2::new(body.left(), find.bottom() + 10.0), body.max);
    painter.rect_filled(text, 0.0, PAPER);
    painter.rect_stroke(text, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
    let inner = Rect::from_min_max(text.min + Vec2::new(11.0, 9.0), text.max - Vec2::new(14.0, 9.0));
    let laid = notices.laid(&painter, inner.width());
    let total = laid.last().map_or(0.0, |(top, item)| top + item.height());
    let current = notices.found.get(notices.at).copied();
    let reveal = std::mem::take(&mut notices.reveal);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    egui::ScrollArea::vertical().id_salt("notices").auto_shrink([false, false]).max_height(inner.height()).show_viewport(&mut child, |ui, view| {
        ui.set_height(total);
        let origin = ui.min_rect().min;
        for (i, (top, item)) in laid.iter().enumerate() {
            if top + item.height() < view.min.y || *top > view.max.y {
                continue;
            }
            match item {
                Laid::Rule => {
                    ui.painter().rect_filled(Rect::from_min_size(origin + Vec2::new(0.0, top + 2.0), Vec2::new(inner.width(), 1.0)), 0.0, TEXT_RULE);
                }
                Laid::Blank => {}
                Laid::Words(indent, galley) => ui.painter().galley(origin + Vec2::new(*indent, *top), galley.clone(), DIM),
            }
            // the match it is on, brought into view when it changes
            if let (true, Some((para, start, _)), Laid::Words(indent, galley)) = (reveal, current, item) {
                if para == i {
                    let Some(Para::Words { text, .. }) = notices.paras.get(para) else { continue };
                    let place = galley.pos_from_cursor(CCursor::new(text[..start].chars().count()));
                    ui.scroll_to_rect(place.translate(origin.to_vec2() + Vec2::new(*indent, *top)), Some(Align::Center));
                }
            }
        }
    });
}

/// what the notices page shows: the paragraphs of the files as installed, the file to reveal, and what is found
struct Notices {
    file: Option<PathBuf>,
    paras: Vec<Para>,
    query: String,
    /// each match: its paragraph, and where it starts and ends in it
    found: Vec<(usize, usize, usize)>,
    /// the match it is on, and whether that is to be brought into view
    at: usize,
    reveal: bool,
}

/// a paragraph of the notices: a rule of = or -, a blank line, or words, which are a heading or are indented
#[derive(Debug, PartialEq)]
enum Para {
    Rule,
    Blank,
    Words { text: String, heading: bool, indent: bool },
}

/// a paragraph as laid out: where it is drawn is kept beside it
enum Laid {
    Rule,
    Blank,
    Words(f32, Arc<Galley>),
}

impl Laid {
    fn height(&self) -> f32 {
        match self {
            Laid::Rule => 5.0,
            Laid::Blank => NOTICE_LINE,
            Laid::Words(_, galley) => galley.size().y,
        }
    }
}

impl Notices {
    /// THIRD-PARTY-NOTICES.txt, then ffmpeg-LICENSE.txt, from the program's own folder. a build run from the source
    /// tree has neither, and says where they come from.
    fn read() -> Notices {
        let folder = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.to_path_buf()));
        let read = |name: &str| folder.as_ref().and_then(|folder| std::fs::read_to_string(folder.join(name)).ok()).map(|text| text.trim_start_matches('\u{feff}').to_string());
        let (ours, ffmpeg) = (read("THIRD-PARTY-NOTICES.txt"), read("ffmpeg-LICENSE.txt"));
        let mut paras = ours.as_deref().map_or_else(Vec::new, |text| paragraphs(text, true));
        if let Some(text) = &ffmpeg {
            paras.extend([Para::Rule, Para::Words { text: "ffmpeg-LICENSE.txt".to_string(), heading: true, indent: false }]);
            paras.extend(paragraphs(text, false));
        }
        if paras.is_empty() {
            let text = "this build has no notices beside it: it runs from the source tree. scripts\\package.ps1 puts THIRD-PARTY-NOTICES.txt and ffmpeg-LICENSE.txt beside the program it packages, and they are read from there.";
            paras.push(Para::Words { text: text.to_string(), heading: false, indent: false });
        }
        let file = ours.and(folder.map(|folder| folder.join("THIRD-PARTY-NOTICES.txt")));
        Notices { file, paras, query: String::new(), found: Vec::new(), at: 0, reveal: false }
    }

    /// finds the query in every paragraph, whatever its case, and goes to the first match.
    fn find(&mut self) {
        let query = self.query.to_ascii_lowercase();
        (self.found, self.at, self.reveal) = (Vec::new(), 0, true);
        if query.is_empty() {
            return;
        }
        for (i, para) in self.paras.iter().enumerate() {
            let Para::Words { text, .. } = para else { continue };
            // ascii case only, so a match's place in the lowered text is its place in the text
            let lower = text.to_ascii_lowercase();
            self.found.extend(lower.match_indices(&query).map(|(at, _)| (i, at, at + query.len())));
        }
    }

    /// every paragraph laid out `width` wide, each with its top: egui keeps the layouts between passes
    fn laid(&self, painter: &egui::Painter, width: f32) -> Vec<(f32, Laid)> {
        let mut top = 0.0;
        self.paras
            .iter()
            .enumerate()
            .map(|(i, para)| {
                let laid = match para {
                    Para::Rule => Laid::Rule,
                    Para::Blank => Laid::Blank,
                    Para::Words { text, heading, indent } => {
                        let indent = if *indent { 12.0 } else { 0.0 };
                        let marks: Vec<(usize, usize, bool)> = self.found.iter().enumerate().filter(|(_, (p, ..))| *p == i).map(|(n, (_, a, b))| (*a, *b, n == self.at)).collect();
                        Laid::Words(indent, painter.layout_job(notice_job(text, *heading, width - indent, &marks)))
                    }
                };
                let at = top;
                top += laid.height();
                (at, laid)
            })
            .collect()
    }
}

/// a paragraph of the notices in its type: a document, so archivo, its headings and version strings mono. a match
/// found is lit, the one it is on in the white of a choice
fn notice_job(text: &str, heading: bool, width: f32, marks: &[(usize, usize, bool)]) -> LayoutJob {
    let mono: Vec<(usize, usize)> = if heading { vec![(0, text.len())] } else { words_of(text).filter(|(a, b)| version_like(&text[*a..*b])).collect() };
    let mut cuts: Vec<usize> = [0, text.len()].into_iter().chain(mono.iter().flat_map(|(a, b)| [*a, *b])).chain(marks.iter().flat_map(|(a, b, _)| [*a, *b])).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    for pair in cuts.windows(2) {
        let at = pair[0];
        let font_id = if mono.iter().any(|(a, b)| (*a..*b).contains(&at)) { font(if heading { MONO_MEDIUM } else { MONO }, 10.5) } else { font(SANS, 11.0) };
        let (color, background) = match marks.iter().find(|(a, b, _)| (*a..*b).contains(&at)) {
            Some((.., true)) => (ON_CHOSEN, CHOSEN),
            Some(_) => (FG, FOUND),
            None => (if heading { FG } else { DIM }, Color32::TRANSPARENT),
        };
        job.append(&text[at..pair[1]], 0.0, TextFormat { font_id, color, background, line_height: Some(NOTICE_LINE), ..Default::default() });
    }
    job
}

/// where each word of `text` starts and ends
fn words_of(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut start = None;
    text.char_indices().chain([(text.len(), ' ')]).filter_map(move |(i, c)| {
        if c.is_whitespace() {
            start.take().map(|from| (from, i))
        } else {
            start.get_or_insert(i);
            None
        }
    })
}

/// a version or a commit: "0.2.29", "n9.0.2-62-gac1c6e003f-20261123", "ac1c6e003f"
fn version_like(word: &str) -> bool {
    let word = word.trim_matches(|c: char| matches!(c, ',' | ';' | ':' | '(' | ')')).trim_end_matches('.');
    let figures = word.trim_start_matches(['v', 'n']);
    let version = figures.starts_with(|c: char| c.is_ascii_digit()) && figures.contains('.') && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    let commit = word.len() >= 7 && word.chars().all(|c| c.is_ascii_hexdigit()) && word.chars().any(|c| c.is_ascii_digit()) && word.chars().any(|c| c.is_ascii_alphabetic());
    version || commit
}

/// a text file's lines as paragraphs for the panel: its hard-wrapped lines joined to flow at the panel's width, a
/// rule of = or - drawn as a line, the line after a rule (and the first, with `heading_first`) a heading, and an
/// indented line, a list's or a table's, kept a line of its own
fn paragraphs(text: &str, heading_first: bool) -> Vec<Para> {
    let mut out = Vec::new();
    let (mut heading, mut open): (bool, Option<String>) = (heading_first, None);
    let flush = |open: &mut Option<String>, out: &mut Vec<Para>| {
        if let Some(text) = open.take() {
            out.push(Para::Words { text, heading: false, indent: false });
        }
    };
    for line in text.lines() {
        let line = line.trim_end();
        let bare = line.trim_start();
        if bare.len() >= 20 && (bare.bytes().all(|b| b == b'=') || bare.bytes().all(|b| b == b'-')) {
            flush(&mut open, &mut out);
            out.push(Para::Rule);
            heading = true;
        } else if bare.is_empty() {
            flush(&mut open, &mut out);
            if !matches!(out.last(), None | Some(Para::Blank | Para::Rule)) {
                out.push(Para::Blank);
            }
        } else if heading || bare.len() != line.len() {
            flush(&mut open, &mut out);
            // runs of spaces that lined up a monospaced file's columns, as two
            let squeezed = bare.split("  ").filter(|part| !part.trim().is_empty()).map(str::trim).collect::<Vec<_>>().join("  ");
            out.push(Para::Words { text: squeezed, heading, indent: !heading });
            heading = false;
        } else {
            // a line that ends a sentence or a label ends its paragraph, and one that begins with a label
            // ("version: ...") starts one of its own
            let label = line.split_once(": ").is_some_and(|(word, _)| !word.is_empty() && !word.contains(' '));
            if label || open.as_deref().is_some_and(|words| words.ends_with(['.', ':'])) {
                flush(&mut open, &mut out);
            }
            match &mut open {
                Some(words) => {
                    words.push(' ');
                    words.push_str(line);
                }
                None => open = Some(line.to_string()),
            }
        }
    }
    flush(&mut open, &mut out);
    out
}

/// a row's note: a few words, or a path, which is mono
enum Note<'a> {
    Words(&'a str),
    Path(&'a str),
}

fn words(text: &str) -> Option<Note<'_>> {
    Some(Note::Words(text))
}

/// what stands at a row's right, from its left
enum Part<'a> {
    Switch(bool),
    /// choices side by side, one lit
    Choices(&'a [&'a str], Option<usize>),
    /// a field: what it shows, its unit, its text's width, and the characters it takes
    Field(String, &'a str, f32, fn(char) -> bool),
    /// a button, which can be pressed or not
    Button(&'a str, bool),
    /// a slider, 0 to 1
    Slider(f32),
    /// how full, 0 to 1
    Meter(f32),
    Cap(&'a str),
    /// a figure or two, dim
    Words(String),
    /// "fixed": what is stated, not offered
    Fixed,
    Swatch([u8; 3]),
}

/// what a part was asked for
enum Got {
    Pressed,
    Chose(usize),
    Typed(String),
    Slid(f32),
}

impl Part<'_> {
    fn width(&self, painter: &egui::Painter) -> f32 {
        let wide = |text: &str, font_id: FontId| painter.layout_no_wrap(text.to_string(), font_id, FG).size().x;
        match self {
            Part::Switch(_) => 26.0,
            Part::Choices(labels, _) => 2.0 + labels.iter().map(|label| wide(label, font(SANS, 12.0)) + 22.0).sum::<f32>() + labels.len().saturating_sub(1) as f32,
            Part::Field(_, unit, width, _) => widgets::field_width(painter, unit, (7.0, *width)),
            Part::Button(label, _) => wide(label, font(SANS, 12.0)) + 20.0,
            Part::Slider(_) => 132.0,
            Part::Meter(_) => 120.0,
            Part::Cap(key) => wide(key, font(MONO_MEDIUM, 10.0)) + 10.0,
            Part::Words(text) => wide(text, font(MONO, 11.0)),
            Part::Fixed => spaced_width(painter, "fixed") + 10.0,
            Part::Swatch(_) => 14.0,
        }
    }

    /// draws the part in `rect`, 26 high. `name` is its own among the page's.
    fn draw(&self, ui: &mut egui::Ui, name: &str, rect: Rect) -> Option<Got> {
        let painter = ui.painter().clone();
        let middle = rect.center().y;
        match self {
            Part::Switch(on) => widgets::switch(ui, name, rect, *on, "", "", false).then_some(Got::Pressed),
            Part::Choices(labels, on) => {
                painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
                let (mut x, mut chose) = (rect.left() + 1.0, None);
                for (i, label) in labels.iter().enumerate() {
                    let lit = *on == Some(i);
                    let color = if lit { ON_CHOSEN } else { DIM };
                    let galley = painter.layout_no_wrap(label.to_string(), font(SANS, 12.0), color);
                    let cell = Rect::from_min_size(Pos2::new(x, rect.top() + 1.0), Vec2::new(galley.size().x + 22.0, rect.height() - 2.0));
                    let response = ui.interact(cell, ui.scope_id().with((name, i)), Sense::click());
                    if lit {
                        painter.rect_filled(cell, 0.0, CHOSEN);
                    } else if response.hovered() {
                        painter.rect_filled(cell, 0.0, HOVER);
                    }
                    if i > 0 {
                        painter.rect_filled(Rect::from_min_size(Pos2::new(cell.left() - 1.0, cell.top()), Vec2::new(1.0, cell.height())), 0.0, LINE);
                    }
                    painter.galley(Pos2::new(cell.left() + 11.0, text_top(&painter, font(SANS, 12.0), middle)), galley, color);
                    if response.clicked() {
                        chose = Some(Got::Chose(i));
                    }
                    x = cell.right() + 1.0;
                }
                chose
            }
            Part::Field(shown, unit, width, keep) => widgets::field_with(ui, name, rect.right_center(), shown, unit, (7.0, *width), *keep).1.map(Got::Typed),
            Part::Button(label, on) => {
                let response = ui.interact(rect, ui.scope_id().with(name), if *on { Sense::click() } else { Sense::hover() });
                if *on && response.hovered() {
                    painter.rect_filled(rect, 0.0, HOVER);
                }
                painter.rect_stroke(rect, 0.0, (1.0, LINE), egui::StrokeKind::Inside);
                let color = if *on { FG } else { OFF };
                painter.galley(Pos2::new(rect.left() + 10.0, text_top(&painter, font(SANS, 12.0), middle)), painter.layout_no_wrap(label.to_string(), font(SANS, 12.0), color), color);
                (*on && response.clicked()).then_some(Got::Pressed)
            }
            Part::Slider(at) => widgets::slider(ui, name, Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 14.0)), *at, ACCENT).map(Got::Slid),
            Part::Meter(full) => {
                let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 4.0));
                painter.rect_filled(track, 0.0, TRACK);
                painter.rect_filled(Rect::from_min_size(track.min, Vec2::new(track.width() * full.clamp(0.0, 1.0), 4.0)), 0.0, FILL);
                None
            }
            Part::Cap(key) => {
                widgets::key_cap(ui, rect.right_center(), key, false, DIM);
                None
            }
            Part::Words(text) => {
                painter.galley(Pos2::new(rect.left(), text_top(&painter, font(MONO, 11.0), middle)), painter.layout_no_wrap(text.clone(), font(MONO, 11.0), DIM), DIM);
                None
            }
            Part::Fixed => {
                let tag = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 15.0));
                painter.rect_stroke(tag, 0.0, (1.0, TAG_LINE), egui::StrokeKind::Inside);
                let mut job = LayoutJob::default();
                job.append("FIXED", 0.0, TextFormat { font_id: font(MONO_MEDIUM, 9.0), color: TAG_INK, extra_letter_spacing: 0.9, ..Default::default() });
                painter.galley(Pos2::new(tag.left() + 5.0, text_top(&painter, font(MONO_MEDIUM, 9.0), middle)), painter.layout_job(job), TAG_INK);
                None
            }
            Part::Swatch([r, g, b]) => {
                let patch = Rect::from_center_size(rect.center(), Vec2::splat(14.0));
                painter.rect_filled(patch, 0.0, Color32::from_rgb(*r, *g, *b));
                painter.rect_stroke(patch, 0.0, (1.0, RING), egui::StrokeKind::Outside);
                None
            }
        }
    }
}

/// a page's rows, as the design draws them: a label and its note at the left, the parts at the right, a faint rule
/// between one row and the next, and a group's name over the rows it heads
#[derive(Default)]
struct Rows {
    after_row: bool,
}

impl Rows {
    fn group(&mut self, ui: &mut egui::Ui, name: &str) {
        ui.add_space(14.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.5), Sense::hover());
        spaced(ui.painter(), rect.min, name, GROUP);
        ui.add_space(2.0);
        self.after_row = false;
    }

    /// a row. returns the part asked for, by its place, and what it was asked for.
    fn row(&mut self, ui: &mut egui::Ui, name: &str, label: &str, note: Option<Note>, parts: &[Part]) -> Option<(usize, Got)> {
        self.line(ui, name, (label, FG), note, parts)
    }

    /// what is stated, not offered: dim, with "fixed" at the right
    fn fact(&mut self, ui: &mut egui::Ui, label: &str, note: &str) {
        self.line(ui, label, (label, DIM), words(note), &[Part::Fixed]);
    }

    fn line(&mut self, ui: &mut egui::Ui, name: &str, (label, ink): (&str, Color32), note: Option<Note>, parts: &[Part]) -> Option<(usize, Got)> {
        let width = ui.available_width();
        let painter = ui.painter().clone();
        let widths: Vec<f32> = parts.iter().map(|part| part.width(&painter)).collect();
        let controls = widths.iter().sum::<f32>() + 6.0 * widths.len().saturating_sub(1) as f32;
        let room = (width - controls - 16.0).max(80.0);
        let label = painter.layout_job(wrapped(label, font(SANS, 12.5), ink, room, 1.3));
        let note = note.map(|note| match note {
            Note::Words(text) => painter.layout_job(wrapped(text, font(SANS, 11.0), DIM, room, 1.4)),
            Note::Path(path) => {
                let mut job = wrapped(path, font(MONO, 11.0), DIM, room, 1.4);
                // a path has no spaces to break at
                job.wrap.break_anywhere = true;
                painter.layout_job(job)
            }
        });
        let words = label.size().y + note.as_ref().map_or(0.0, |note| 3.0 + note.size().y);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, words.max(26.0) + 14.0), Sense::hover());
        if std::mem::replace(&mut self.after_row, true) {
            painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(width, 1.0)), 0.0, RULE);
        }
        let top = rect.center().y - words / 2.0;
        let label_height = label.size().y;
        painter.galley(Pos2::new(rect.left(), top), label, ink);
        if let Some(note) = note {
            painter.galley(Pos2::new(rect.left(), top + label_height + 3.0), note, DIM);
        }
        let (mut x, mut got) = (rect.right() - controls, None);
        for (i, (part, wide)) in parts.iter().zip(widths).enumerate() {
            if let Some(asked) = part.draw(ui, &format!("{name}-{i}"), Rect::from_min_size(Pos2::new(x, rect.center().y - 13.0), Vec2::new(wide, 26.0))) {
                got = Some((i, asked));
            }
            x += wide + 6.0;
        }
        got
    }
}

/// a switch's row pressed: it turns over
fn toggle(got: Option<(usize, Got)>, on: &mut bool) {
    if got.is_some() {
        *on = !*on;
    }
}

/// a number typed into a row's field
fn typed(got: Option<(usize, Got)>) -> Option<f32> {
    match got {
        Some((_, Got::Typed(text))) => text.parse().ok(),
        _ => None,
    }
}

fn number(c: char) -> bool {
    c.is_ascii_digit() || c == '.'
}

fn hex(c: char) -> bool {
    c.is_ascii_hexdigit() || c == '#'
}

fn hex_of([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// words wrapped to `width`, their lines `line` times their size apart
fn wrapped(text: &str, font_id: FontId, color: Color32, width: f32, line: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let size = font_id.size;
    job.append(text, 0.0, TextFormat { font_id, color, line_height: Some(size * line), ..Default::default() });
    job
}

/// a name in small spaced capitals, mono, with its left top at `at`: a sidebar's heading, a group's name
fn spaced(painter: &egui::Painter, at: Pos2, text: &str, color: Color32) {
    let mut job = LayoutJob::default();
    job.append(&text.to_uppercase(), 0.0, TextFormat { font_id: font(MONO_MEDIUM, 9.5), color, extra_letter_spacing: 9.5 * 0.16, ..Default::default() });
    painter.galley(Pos2::new(at.x, text_top(painter, font(MONO_MEDIUM, 9.5), at.y + 4.75)), painter.layout_job(job), color);
}

fn spaced_width(painter: &egui::Painter, text: &str) -> f32 {
    let mut job = LayoutJob::default();
    job.append(&text.to_uppercase(), 0.0, TextFormat { font_id: font(MONO_MEDIUM, 9.0), color: TAG_INK, extra_letter_spacing: 0.9, ..Default::default() });
    painter.layout_job(job).size().x
}

/// outlined buttons in a row, 6 apart, each a link: returns the page of the one pressed
fn links(ui: &mut egui::Ui, name: &str, links: &[(&str, &'static str)]) -> Option<&'static str> {
    let painter = ui.painter().clone();
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), widgets::ROW), Sense::hover());
    let (mut x, mut pressed) = (row.left(), None);
    for (i, (label, url)) in links.iter().enumerate() {
        let button = Part::Button(label, true);
        let wide = button.width(&painter);
        if button.draw(ui, &format!("{name}-{i}"), Rect::from_min_size(Pos2::new(x, row.top()), Vec2::new(wide, row.height()))).is_some() {
            pressed = Some(*url);
        }
        x += wide + 6.0;
    }
    pressed
}

/// dim words with a link among them, wrapped to the room: true when the link is pressed. the link is in the
/// foreground's ink, underlined dotted
fn sentence(ui: &mut egui::Ui, name: &str, before: &str, link: &str, after: &str) -> bool {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    for (words, color) in [(before, DIM), (link, FG), (after, DIM)] {
        job.append(words, 0.0, TextFormat { font_id: font(SANS, 12.0), color, line_height: Some(12.0 * 1.45), ..Default::default() });
    }
    let galley = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
    // the link's words where they were laid, on the line they start on
    let start = before.chars().count();
    let first = galley.layout_from_cursor(CCursor::new(start));
    let end = galley.layout_from_cursor(CCursor::new(start + link.chars().count()));
    let row = &galley.rows[first.row];
    let glyph = &row.row.glyphs[first.column.0];
    let right = if end.row == first.row { galley.pos_from_cursor(CCursor::new(start + link.chars().count())).min.x } else { row.rect().right() };
    let left = rect.left() + glyph.pos.x + row.pos.x;
    let words = Rect::from_min_max(Pos2::new(left, rect.top() + row.pos.y), Pos2::new(rect.left() + right, rect.top() + row.pos.y + row.height()));
    let under = rect.top() + row.pos.y + glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y;
    ui.painter().galley(rect.min, galley.clone(), DIM);
    dotted_under(ui.painter(), left, words.width(), under, FG);
    ui.interact(words, ui.scope_id().with(name), Sense::click()).clicked()
}

/// the dotted line under a link whose letters' foot is at `under`
fn dotted_under(painter: &egui::Painter, left: f32, width: f32, under: f32, color: Color32) {
    // `dotted` measures the foot from a figure laid at a place: here it is known
    let figure = painter.layout_no_wrap("0".to_string(), font(SANS, 12.0), color);
    let glyph = &figure.rows[0].row.glyphs[0];
    dotted(painter, font(SANS, 12.0), Pos2::new(left, under - (glyph.pos.y + glyph.uv_rect.offset.y + glyph.uv_rect.size.y)), width, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a file's hard-wrapped lines flow again, up to a line that ends a sentence or a label, or one that starts with
    /// a label; a rule is a line and the line after it a heading; indented lines keep their own lines, their columns'
    /// spaces squeezed; blank lines stand between paragraphs, one for many.
    #[test]
    fn the_notices_are_set_as_paragraphs() {
        let text = "third-party notices: eyepiece 0.1.0\n\neyepiece contains open source software. each part\nis under its own license:\n\n\n  1. ffmpeg:      LGPL\n  2. 78 rust crates\n====================\n1. ffmpeg\n\nversion: n9.0.2-62-gac1c6e003f, the build made by\nhttps://github.com/BtbN/FFmpeg-Builds\nsource: https://git.ffmpeg.org at commit ac1c6e003f.\nthe end";
        let words = |text: &str, heading, indent| Para::Words { text: text.to_string(), heading, indent };
        assert_eq!(
            paragraphs(text, true),
            [
                words("third-party notices: eyepiece 0.1.0", true, false),
                Para::Blank,
                words("eyepiece contains open source software. each part is under its own license:", false, false),
                Para::Blank,
                words("1. ffmpeg:  LGPL", false, true),
                words("2. 78 rust crates", false, true),
                Para::Rule,
                words("1. ffmpeg", true, false),
                Para::Blank,
                words("version: n9.0.2-62-gac1c6e003f, the build made by https://github.com/BtbN/FFmpeg-Builds", false, false),
                words("source: https://git.ffmpeg.org at commit ac1c6e003f.", false, false),
                words("the end", false, false),
            ]
        );
        // versions and commits are mono, the rest is not
        let mono: Vec<&str> = ["0.2.29", "n9.0.2-62-gac1c6e003f,", "ac1c6e003f.", "9.0", "1.", "Apache-2.0", "https://ffmpeg.org", "license", "deadline"].into_iter().filter(|w| version_like(w)).collect();
        assert_eq!(mono, ["0.2.29", "n9.0.2-62-gac1c6e003f,", "ac1c6e003f.", "9.0"]);
    }

    /// the find field finds a word in any case in every paragraph, and the count follows it.
    #[test]
    fn the_notices_find_in_any_case() {
        let mut notices = Notices { file: None, paras: paragraphs("ffmpeg is LGPL.\n\n  lgpl-shared\n", false), query: "LgPl".to_string(), found: Vec::new(), at: 0, reveal: false };
        notices.find();
        assert_eq!(notices.found, [(0, 10, 14), (2, 0, 4)]);
        notices.query.clear();
        notices.find();
        assert!(notices.found.is_empty());
    }
}
