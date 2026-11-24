//! the file browser: what an empty tab shows, and what O lays over a clip. this is its first cut, one plain list of
//! the folder you are in, from its listing alone. the design's columns, grid and metadata column replace it.

use super::{font, DIM, FG, LINE, MONO, MONO_MEDIUM, SANS};
use crate::browse::{self, Entry, Kind};
use crate::event::Key;
use egui::{Color32, Pos2, Rect, Sense, Vec2};
use std::path::{Path, PathBuf};

const BACK: Color32 = Color32::from_rgb(0x0e, 0x0e, 0x0e);
const SELECTED: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26); // rgba(255,255,255,.1)
const HEAD: f32 = 36.0;
const LINE_HEIGHT: f32 = 26.0;

/// what was chosen in the browser
pub enum Chosen {
    /// open this file. a sequence is opened by its first file
    Open(PathBuf),
    /// back to the clip under the browser
    Back,
}

pub struct Browser {
    folder: PathBuf,
    /// the folder's entries, or why it could not be listed
    entries: Result<Vec<Entry>, String>,
    selected: usize,
    /// the first row in view
    top: usize,
    /// chosen by key since the last pass
    chosen: Option<Chosen>,
}

impl Browser {
    /// starts in `folder`; with none, in the first of the system's places (downloads).
    pub fn new(folder: Option<PathBuf>) -> Browser {
        let folder = folder.or_else(|| browse::places().into_iter().next().map(|p| p.path)).unwrap_or_default();
        let mut browser = Browser { folder: PathBuf::new(), entries: Ok(Vec::new()), selected: 0, top: 0, chosen: None };
        browser.go(folder);
        browser
    }

    /// the folder on screen: kept between runs.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn go(&mut self, folder: PathBuf) {
        self.entries = browse::list(&folder).map_err(|e| e.to_string());
        (self.folder, self.selected, self.top) = (folder, 0, 0);
    }

    /// a folder is entered, a clip is chosen.
    fn enter(&mut self, at: usize) {
        let Some(entry) = self.entries.as_ref().ok().and_then(|e| e.get(at)) else { return };
        match entry.kind {
            Kind::Folder => self.go(entry.path.clone()),
            Kind::Clip | Kind::Sequence { .. } => self.chosen = Some(Chosen::Open(entry.path.clone())),
            Kind::Other => {}
        }
    }

    pub fn key(&mut self, key: Key) {
        let last = self.entries.as_ref().map_or(0, |e| e.len()).saturating_sub(1);
        match key {
            Key::Down => self.selected = (self.selected + 1).min(last),
            Key::Up => self.selected = self.selected.saturating_sub(1),
            Key::Home => self.selected = 0,
            Key::End => self.selected = last,
            Key::Enter | Key::Right => self.enter(self.selected),
            Key::Backspace | Key::Left => {
                if let Some(parent) = self.folder.parent().map(Path::to_path_buf) {
                    self.go(parent);
                }
            }
            Key::Escape => self.chosen = Some(Chosen::Back),
            _ => {}
        }
    }

    /// a typed letter selects the first entry whose name starts with it.
    pub fn char(&mut self, c: char) {
        let typed = c.to_lowercase().to_string();
        if let Some(at) = self.entries.as_ref().ok().and_then(|e| e.iter().position(|e| e.name.to_lowercase().starts_with(&typed))) {
            self.selected = at;
        }
    }

    /// draws the browser over `area`. returns what was chosen, by key or by pointer.
    pub fn draw(&mut self, ui: &mut egui::Ui, area: Rect) -> Option<Chosen> {
        let painter = ui.painter().with_clip_rect(area);
        painter.rect_filled(area, 0.0, BACK);
        // a press here is not a press on a picture under the browser
        ui.interact(area, ui.scope_id().with("browser"), Sense::click_and_drag());

        // the folder's name and what it holds
        let head = Rect::from_min_size(area.min, Vec2::new(area.width(), HEAD));
        painter.rect_filled(Rect::from_min_max(Pos2::new(head.left(), head.bottom() - 1.0), head.max), 0.0, LINE);
        let name = self.folder.file_name().map_or_else(|| self.folder.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
        let name = painter.layout_no_wrap(name, font(MONO_MEDIUM, 11.5), FG);
        let holds = match &self.entries {
            Ok(entries) => format!("{} here", entries.len()),
            Err(why) => why.clone(),
        };
        painter.text(Pos2::new(head.left() + 14.0 + name.size().x + 10.0, head.center().y), egui::Align2::LEFT_CENTER, holds, font(MONO, 11.5), DIM);
        painter.galley(Pos2::new(head.left() + 14.0, head.center().y - name.size().y / 2.0), name, FG);

        // the rows that fit, with the selected one kept in view
        let rows = (((area.height() - HEAD) / LINE_HEIGHT).floor() as usize).max(1);
        self.top = self.top.min(self.selected).max((self.selected + 1).saturating_sub(rows));
        let mut entered = None;
        for (row, (at, entry)) in self.entries.iter().flatten().enumerate().skip(self.top).take(rows).enumerate() {
            let rect = Rect::from_min_size(Pos2::new(area.left(), head.bottom() + row as f32 * LINE_HEIGHT), Vec2::new(area.width(), LINE_HEIGHT));
            let response = ui.interact(rect, ui.scope_id().with(("browser-row", at)), Sense::click());
            if response.clicked() {
                self.selected = at;
            }
            if response.double_clicked() {
                entered = Some(at);
            }
            if at == self.selected {
                painter.rect_filled(rect, 0.0, SELECTED);
            }
            let color = match entry.kind {
                Kind::Other => DIM,
                _ if at == self.selected => Color32::WHITE,
                _ => Color32::from_rgb(0xc9, 0xc9, 0xc9),
            };
            let mark = if matches!(entry.kind, Kind::Folder) { "›" } else { "" };
            painter.text(Pos2::new(rect.left() + 14.0, rect.center().y), egui::Align2::LEFT_CENTER, &entry.name, font(SANS, 12.5), color);
            painter.text(Pos2::new(rect.right() - 14.0, rect.center().y), egui::Align2::RIGHT_CENTER, mark, font(SANS, 12.5), DIM);
        }
        if let Some(at) = entered {
            self.enter(at);
        }
        self.chosen.take()
    }
}
