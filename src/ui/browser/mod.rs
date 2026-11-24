//! the file browser: what an empty tab shows, and what O lays over a clip (the design's figs. 27 to 29). at the left
//! the places and volumes, then the path as finder-style columns, the folder shown in the main area with its videos
//! and sequences as cards, and at the right the selected card's metadata. folders are read from their listing alone,
//! off the drawing thread; on a network volume nothing more is read until asked (adr 0004).

mod jobs;
mod nav;

use super::{cut_middle, font, text_top, AMBER, DIM, FG, HOVER, LINE, MONO, MONO_MEDIUM, SANS};
use crate::browse::{self, Entry, Kind, Place, Volume};
use crate::event::{Key, Mods};
use crate::media::meta::{Meta, Row, Tone};
use crate::preview::make::{self, Pictures, Want};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, FontId, Galley, Painter, Pos2, Rect, Sense, StrokeKind, Vec2};
use jobs::Jobs;
use nav::{is_card, Listing, Nav};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---- the design's colors for the browser, beside the ones every control uses
const BACK: Color32 = Color32::from_rgb(0x0e, 0x0e, 0x0e);
/// the sidebar and the metadata column
const PANEL: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
/// a card with no picture, and the file type written on it
const PLAIN: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
const PLAIN_TEXT: Color32 = Color32::from_rgb(0x5f, 0x5f, 0x5f);
/// a selected row, rgba(255,255,255,.1), and a selected place, .08
const ROW_ON: Color32 = Color32::from_rgba_premultiplied(26, 26, 26, 26);
const PLACE_ON: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
const ROW_TEXT: Color32 = Color32::from_rgb(0xc9, 0xc9, 0xc9);
/// a file that is not a video
const ROW_DIM: Color32 = Color32::from_rgb(0x6d, 0x6d, 0x6d);
/// the sidebar's section names and a folder's arrow
const GRAY: Color32 = Color32::from_rgb(0x77, 0x77, 0x77);
const FOLDER_EDGE: Color32 = Color32::from_rgb(0x8a, 0x8a, 0x8a);
const FILE_EDGE: Color32 = Color32::from_rgb(0x66, 0x66, 0x66);
/// the "net" tag on a network volume
const TAG_EDGE: Color32 = Color32::from_rgb(0x55, 0x55, 0x55);
const TAG: Color32 = Color32::from_rgb(0x88, 0x88, 0x88);
/// the marks on a card, on rgba(0,0,0,.75)
const BADGE: Color32 = Color32::from_rgb(0xdd, 0xdd, 0xdd);
const BADGE_BACK: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 191);
/// the one filled button, open
const STRONG: Color32 = Color32::from_rgb(0xec, 0xec, 0xec);
const ON_STRONG: Color32 = Color32::from_rgb(0x11, 0x11, 0x11);

// ---- the design's measurements (fig. 28)
const SIDE: f32 = 150.0;
const COLUMN: f32 = 168.0;
const META: f32 = 316.0;
/// the main area keeps this much before another column is drawn; the metadata column comes before the columns
const MAIN_MIN: f32 = 360.0;
/// cards are at least this wide: as many go across as fit
const CARD_MIN: f32 = 160.0;
/// a row of the sidebar and the columns: 12.5 type at 1.3, and 5 above and below
const ROW: f32 = 26.25;
const PATH_LINE: f32 = 36.0;
/// a card's name and its line under it: 11.5 and 10.5 type at 1.25
const NAME: f32 = 14.375;
const SPEC: f32 = 13.125;
/// a line of the metadata column: 11.5 type at 1.35
const META_LINE: f32 = 15.525;
/// what a notch of the wheel scrolls
const NOTCH: f32 = 3.0 * ROW;
/// how long the selection rests on a card before its file is read, in seconds
const REST: f64 = 0.15;
/// while an answer is awaited, the browser looks again this often: nothing else wakes the window for it
const POLL: Duration = Duration::from_millis(40);

/// what was chosen in the browser
pub enum Chosen {
    /// open this file. a sequence is opened by its first file
    Open(PathBuf),
    /// play this folder
    PlayFolder(PathBuf),
    /// back to the clip under the browser
    Back,
}

pub struct Browser {
    /// where it starts, and the maker of its cards' pictures: held until it is first shown, so a launch with a
    /// file reads no folder
    start: Option<PathBuf>,
    pictures: Option<Pictures>,
    state: Option<State>,
}

impl Browser {
    /// starts in `folder`; with none, in the first of the system's places (downloads). without `pictures` the
    /// cards stay plain.
    pub fn new(folder: Option<PathBuf>, pictures: Option<Pictures>) -> Browser {
        Browser { start: folder, pictures, state: None }
    }

    /// the browser is not on screen: no pictures are wanted.
    pub fn away(&mut self) {
        if let Some(state) = &mut self.state {
            state.want(Vec::new());
        }
    }

    /// the folder the main area shows: kept between runs.
    pub fn folder(&self) -> &Path {
        match &self.state {
            Some(state) => &state.nav.main().folder,
            None => self.start.as_deref().unwrap_or(Path::new("")),
        }
    }

    fn state(&mut self) -> &mut State {
        let (start, pictures) = (&mut self.start, &mut self.pictures);
        self.state.get_or_insert_with(|| State::new(start.take(), pictures.take()))
    }

    pub fn key(&mut self, key: Key, mods: Mods) {
        let state = self.state();
        state.take_answers();
        if let Some(chosen) = state.nav.key(key, mods, state.per_row) {
            state.choose(chosen);
        }
    }

    /// a typed character: letters select by name.
    pub fn char(&mut self, c: char) {
        let state = self.state();
        state.take_answers();
        state.nav.char(c, Instant::now());
    }

    /// the wheel, in notches (away from the user is positive). it scrolls what is under the pointer.
    pub fn wheel(&mut self, notches: i32) {
        self.state().wheel += notches;
    }

    /// draws the browser over `area`. returns what was chosen, by key or by pointer.
    pub fn draw(&mut self, ui: &mut egui::Ui, area: Rect) -> Option<Chosen> {
        self.state().draw(ui, area)
    }
}

struct State {
    nav: Nav,
    /// the sidebar: the places, then the volumes, each with the kind of volume it is on
    places: Vec<(Place, Volume)>,
    lists: Jobs<Listing>,
    metas: Jobs<Result<Meta, String>>,
    /// listings read before the browser was last away: shown while they are read again
    stale: HashSet<PathBuf>,
    /// the metadata read: the selected card's, and that of the files asked about
    read: HashMap<PathBuf, Result<Meta, String>>,
    /// the files whose metadata was asked for, or that were opened from here: on a network volume, read
    asked: HashSet<PathBuf>,
    /// the unread file the selection is on, and since when
    resting: Option<(PathBuf, f64)>,
    /// the maker of the cards' pictures, and what it was last asked for
    pictures: Option<Pictures>,
    wanted: Vec<Want>,
    /// the pictures of the cards in the folder shown, as far as they have come
    cards: HashMap<PathBuf, Card>,
    /// the card the pointer is on, and since when: resting there asks for all of its previews
    hover: Option<(PathBuf, f64)>,
    /// wheel notches not yet applied; how far the sidebar, the grid and the metadata column are scrolled, and how
    /// tall the sidebar's and the metadata column's contents were when last drawn
    wheel: i32,
    side_scroll: f32,
    side_height: f32,
    grid_scroll: f32,
    meta_scroll: f32,
    meta_height: f32,
    /// the folder the grid was last drawn for, and the card last brought into view in it
    grid_folder: PathBuf,
    revealed: Option<PathBuf>,
    /// cards across the grid when it was last drawn: up and down move by it
    per_row: usize,
    /// what a double click asks for, should the next press make one: the last press's, as that press may have moved
    /// what is under the pointer (a selected card brings in the metadata column)
    twice: Option<Click>,
    chosen: Option<Chosen>,
    /// the pass it was last drawn in
    drawn: u64,
}

/// a press in the browser: what it asks for, done once everything is drawn, and what a double click on the same thing
/// asks for
type Press = (Click, Option<Click>);

enum Click {
    Place(PathBuf),
    /// an entry of the drawn column at this place on the path; true for a folder
    Row(usize, PathBuf, bool),
    Enter,
    Card(PathBuf),
    Open(PathBuf),
    PlayFolder,
    Read(PathBuf),
}

/// where the parts go: the sidebar, the columns that fit (the deepest ones), the main area, the metadata column
struct Layout {
    side: Rect,
    /// the first column on the path that is drawn, and where each drawn one goes
    first: usize,
    columns: Vec<Rect>,
    main: Rect,
    meta: Option<Rect>,
}

impl Layout {
    /// `shown` columns on the path; `card`: one is selected, so the metadata column is wanted.
    fn new(area: Rect, shown: usize, card: bool) -> Layout {
        let meta = card && area.width() - SIDE - COLUMN - META >= MAIN_MIN;
        let room = area.width() - SIDE - if meta { META } else { 0.0 };
        let fit = (((room - MAIN_MIN) / COLUMN).floor().max(1.0) as usize).min(shown);
        let x = |offset: f32| area.left() + offset;
        let span = |from: f32, to: f32| Rect::from_min_max(Pos2::new(from, area.top()), Pos2::new(to.max(from), area.bottom()));
        let main = x(SIDE + fit as f32 * COLUMN);
        Layout {
            side: span(area.left(), x(SIDE)),
            first: shown - fit,
            columns: (0..fit).map(|i| span(x(SIDE + i as f32 * COLUMN), x(SIDE + (i + 1) as f32 * COLUMN))).collect(),
            main: span(main, if meta { area.right() - META } else { area.right() }),
            meta: meta.then(|| span(area.right() - META, area.right())),
        }
    }
}

/// a card's pictures: the frame each is of, in order
struct Card {
    /// the file's size and time when they were asked for: a changed file starts over
    stamp: (u64, SystemTime),
    clip_frames: usize,
    pictures: Vec<(usize, egui::TextureHandle)>,
    /// the poster has been answered for, and all of the previews: they are here, or not to be had
    poster: bool,
    all: bool,
}

impl Card {
    /// the picture to show: the one nearest the place in the clip the pointer scrubs to (0 to 1), else the poster.
    fn shown(&self, scrub: Option<f32>) -> Option<&(usize, egui::TextureHandle)> {
        let wanted = match scrub {
            Some(at) => (at * self.clip_frames.saturating_sub(1) as f32).round() as usize,
            None => make::poster(self.clip_frames),
        };
        self.pictures.iter().min_by_key(|(frame, _)| frame.abs_diff(wanted))
    }
}

/// what the folder listing says of a file, which names its pictures
fn stamp(entry: &Entry) -> (u64, SystemTime) {
    (entry.size.unwrap_or(0), entry.modified.unwrap_or(UNIX_EPOCH))
}

impl State {
    fn new(start: Option<PathBuf>, pictures: Option<Pictures>) -> State {
        let places = browse::places();
        let folder = start.or_else(|| places.first().map(|p| p.path.clone())).unwrap_or_default();
        let roots: Vec<PathBuf> = places.iter().map(|p| p.path.clone()).collect();
        State {
            nav: Nav::new(&folder, &roots),
            places: places
                .into_iter()
                .map(|place| {
                    let volume = browse::volume_of(&place.path);
                    (place, volume)
                })
                .collect(),
            lists: Jobs::new(),
            metas: Jobs::new(),
            stale: HashSet::new(),
            read: HashMap::new(),
            asked: HashSet::new(),
            resting: None,
            pictures,
            wanted: Vec::new(),
            cards: HashMap::new(),
            hover: None,
            wheel: 0,
            side_scroll: 0.0,
            side_height: 0.0,
            grid_scroll: 0.0,
            meta_scroll: 0.0,
            meta_height: 0.0,
            grid_folder: PathBuf::new(),
            revealed: None,
            per_row: 1,
            twice: None,
            chosen: None,
            drawn: 0,
        }
    }

    /// what the host is asked for. a file opened from here counts as asked about.
    fn choose(&mut self, chosen: Chosen) {
        if let Chosen::Open(file) = &chosen {
            self.asked.insert(file.clone());
        }
        self.chosen = Some(chosen);
    }

    /// tells the maker what is wanted now, when that has changed.
    fn want(&mut self, wants: Vec<Want>) {
        if let Some(pictures) = self.pictures.as_ref().filter(|_| wants != self.wanted) {
            pictures.want(wants.clone());
            self.wanted = wants;
        }
    }

    /// takes in the pictures that have come.
    fn take_pictures(&mut self, ctx: &egui::Context) {
        while let Some(got) = self.pictures.as_ref().and_then(Pictures::take) {
            let stamp = (got.want.size, got.want.modified);
            let card = self.cards.entry(got.want.path.clone()).or_insert_with(|| Card { stamp, clip_frames: 0, pictures: Vec::new(), poster: false, all: false });
            if card.stamp != stamp {
                *card = Card { stamp, clip_frames: 0, pictures: Vec::new(), poster: false, all: false };
            }
            for (frame, pixels) in got.pictures {
                card.clip_frames = got.clip_frames;
                let image = egui::ColorImage::from_rgb([got.width as usize, got.height as usize], &pixels);
                let texture = ctx.load_texture(format!("card {} {frame}", got.want.path.display()), image, egui::TextureOptions::LINEAR);
                match card.pictures.binary_search_by_key(&frame, |(of, _)| *of) {
                    Ok(at) => card.pictures[at].1 = texture,
                    Err(at) => card.pictures.insert(at, (frame, texture)),
                }
            }
            card.poster |= got.done;
            card.all |= got.done && got.want.all;
        }
    }

    fn take_answers(&mut self) {
        for (folder, listing) in self.lists.answers() {
            self.stale.remove(&folder);
            self.nav.listings.insert(folder, listing);
        }
        self.read.extend(self.metas.answers());
        self.nav.settle();
    }

    fn draw(&mut self, ui: &mut egui::Ui, area: Rect) -> Option<Chosen> {
        // away since the last pass (a clip was up): what is on screen may have changed, so it is listed again
        let pass = ui.ctx().cumulative_pass_nr();
        if pass > self.drawn + 1 {
            self.stale = self.nav.listings.keys().cloned().collect();
        }
        self.drawn = pass;
        self.take_answers();
        ui.painter().with_clip_rect(area).rect_filled(area, 0.0, BACK);
        // a press here is not a press on a picture under the browser
        ui.interact(area, ui.scope_id().with("browser"), Sense::click_and_drag());

        let layout = Layout::new(area, self.nav.shown().len(), self.nav.card().is_some());
        self.scroll(ui, &layout);
        let mut clicks = Vec::new();
        self.side(ui, layout.side, &mut clicks);
        for (i, rect) in layout.columns.iter().enumerate() {
            self.column(ui, layout.first + i, *rect, &mut clicks);
        }
        self.main_area(ui, layout.main, &mut clicks);
        if let Some(rect) = layout.meta {
            self.metadata(ui, rect, &mut clicks);
        }
        // a double click acts on what its first press was on, not on what its second one lands on. a press on nothing
        // leaves nothing for one
        let (single, double) = ui.input(|i| {
            let inside = i.pointer.interact_pos().is_some_and(|p| area.contains(p));
            (inside && i.pointer.primary_clicked(), inside && i.pointer.button_double_clicked(egui::PointerButton::Primary))
        });
        let clicks: Vec<Click> = if double {
            self.twice.take().into_iter().collect()
        } else {
            if single {
                self.twice = clicks.last_mut().and_then(|(_, twice)| twice.take());
            }
            clicks.into_iter().map(|(click, _)| click).collect()
        };
        // what a press changed is drawn at once, not with the next thing the window says
        if !clicks.is_empty() {
            ui.ctx().request_repaint();
        }
        for click in clicks {
            self.click(click);
        }
        if let Some(wait) = self.ask(layout.first, ui.input(|i| i.time)) {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(wait));
        }
        if self.lists.busy() || self.metas.busy() {
            ui.ctx().request_repaint_after(POLL);
        }
        self.chosen.take()
    }

    fn click(&mut self, click: Click) {
        match click {
            Click::Place(place) => self.nav.go(&place),
            Click::Row(at, entry, folder) => {
                self.nav.select(at, entry, folder);
                self.nav.focus = nav::Focus::Columns;
            }
            Click::Enter => self.nav.enter(),
            Click::Card(card) => self.nav.choose_card(card),
            Click::Open(file) => self.choose(Chosen::Open(file)),
            Click::PlayFolder => self.choose(Chosen::PlayFolder(self.nav.main().folder.clone())),
            Click::Read(file) => {
                self.asked.insert(file);
            }
        }
    }

    /// asks for what is on screen and not read yet, and lets go of what is not on screen any more. returns how long
    /// until it should be called again, when a file is waiting for the selection to rest on it.
    fn ask(&mut self, first: usize, now: f64) -> Option<f64> {
        let needed = self.nav.needed(first);
        self.lists.keep_only(|folder| needed.iter().any(|n| n == folder));
        for folder in needed {
            if (!self.nav.listings.contains_key(&folder) || self.stale.contains(&folder)) && !self.lists.waiting(&folder) {
                self.lists.ask(folder, list);
            }
        }
        // a folder off the path is listed again when it comes back
        let nav::Nav { listings, path, .. } = &mut self.nav;
        listings.retain(|folder, _| path.iter().any(|c| &c.folder == folder));
        self.stale.retain(|folder| listings.contains_key(folder));

        // what was read is kept for the files asked about, so going back to one does not open it again
        let file = self.nav.metadata(&self.asked);
        self.read.retain(|f, _| self.asked.contains(f) || file.as_ref() == Some(f));
        self.metas.keep_only(|f| Some(f) == file.as_deref());
        // a selection only passing over a card (an arrow held down) reads nothing: the read starts where it rests.
        // a file asked for by name is read at once
        let file = file.filter(|f| !self.read.contains_key(f) && !self.metas.waiting(f))?;
        let wait = if self.asked.contains(&file) { 0.0 } else { rest(&mut self.resting, &file, now) };
        if wait > 0.0 {
            return Some(wait);
        }
        self.metas.ask(file, read_meta);
        None
    }

    /// the wheel scrolls what is under the pointer: the sidebar, a column, the grid or the metadata column. each keeps
    /// itself in range when it is drawn.
    fn scroll(&mut self, ui: &egui::Ui, layout: &Layout) {
        let notches = std::mem::take(&mut self.wheel);
        let Some(at) = ui.ctx().pointer_latest_pos().filter(|_| notches != 0) else { return };
        let by = -notches as f32 * NOTCH;
        if layout.side.contains(at) {
            self.side_scroll += by;
        } else if let Some(i) = layout.columns.iter().position(|r| r.contains(at)) {
            self.nav.path[layout.first + i].scroll += by;
        } else if layout.main.contains(at) {
            self.grid_scroll += by;
        } else if layout.meta.is_some_and(|r| r.contains(at)) {
            self.meta_scroll += by;
        }
    }

    /// the sidebar: the places, then the volumes, a network one tagged.
    fn side(&mut self, ui: &egui::Ui, rect: Rect, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, PANEL);
        rule(&painter, rect.right() - 1.0, rect);
        self.side_scroll = self.side_scroll.clamp(0.0, (self.side_height - rect.height()).max(0.0));
        let width = rect.width() - 1.0;
        let mut y = rect.top() + 12.0 - self.side_scroll;
        let mut first = true;
        for drives in [false, true] {
            let places: Vec<&(Place, Volume)> = self.places.iter().filter(|(p, _)| p.path.parent().is_none() == drives).collect();
            if places.is_empty() {
                continue;
            }
            if !std::mem::take(&mut first) {
                y += 1.0 + 12.0;
            }
            let heading = spaced(&painter, if drives { "VOLUMES" } else { "PLACES" }, font(MONO_MEDIUM, 9.5), 1.52, GRAY);
            painter.galley(Pos2::new(rect.left() + 14.0, top_of(&painter, &heading, y + 4.75)), heading, GRAY);
            y += 9.5 + 5.0;
            for (i, (place, volume)) in places.into_iter().enumerate() {
                let row = Rect::from_min_size(Pos2::new(rect.left(), y + 1.0), Vec2::new(width, ROW));
                y = row.bottom();
                let on = place.path == self.nav.path[0].folder;
                if on {
                    painter.rect_filled(row, 0.0, PLACE_ON);
                }
                let mut room = width - 28.0;
                if *volume == Volume::Network {
                    let tag = spaced(&painter, "net", font(MONO_MEDIUM, 9.0), 0.9, TAG);
                    let size = Vec2::new(tag.size().x + 8.0, 15.0);
                    let mark = Rect::from_min_size(Pos2::new(row.right() - 14.0 - size.x, row.center().y - size.y / 2.0), size);
                    painter.rect_stroke(mark, 0.0, (1.0, TAG_EDGE), StrokeKind::Inside);
                    painter.galley(Pos2::new(mark.center().x - tag.size().x / 2.0, top_of(&painter, &tag, mark.center().y)), tag, TAG);
                    room -= size.x + 8.0;
                }
                text(&painter, row.left() + 14.0, row.center().y, &place.name, font(SANS, 12.5), if on { FG } else { DIM }, room);
                if ui.interact(row.intersect(rect), ui.scope_id().with(("brw-place", drives, i)), Sense::click()).clicked() {
                    clicks.push((Click::Place(place.path.clone()), None));
                }
            }
        }
        self.side_height = y + self.side_scroll - rect.top() + 12.0;
    }

    /// one folder on the path as a column: its entries, the selected one lit.
    fn column(&mut self, ui: &egui::Ui, at: usize, rect: Rect, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        rule(&painter, rect.right() - 1.0, rect);
        let inner = Rect::from_min_max(rect.min, Pos2::new(rect.right() - 1.0, rect.bottom()));
        let entries = match self.nav.listings.get(&self.nav.path[at].folder) {
            None => {
                text(&painter, inner.left() + 12.0, inner.top() + 8.0 + ROW / 2.0, "reading…", font(SANS, 12.5), DIM, inner.width() - 22.0);
                return;
            }
            Some(Err(why)) => {
                let galley = painter.layout(why.to_lowercase(), font(SANS, 12.5), DIM, inner.width() - 22.0);
                painter.galley(Pos2::new(inner.left() + 12.0, inner.top() + 13.0), galley, DIM);
                return;
            }
            Some(Ok(entries)) => entries,
        };
        let column = &self.nav.path[at];
        let selected = column.selected.as_ref().and_then(|s| entries.iter().position(|e| &e.path == s));
        let mut scroll = column.scroll;
        if let Some(i) = selected.filter(|_| column.reveal) {
            scroll = reveal(scroll, 8.0 + i as f32 * ROW, ROW, rect.height(), 8.0);
        }
        scroll = scroll.clamp(0.0, (16.0 + entries.len() as f32 * ROW - rect.height()).max(0.0));
        (self.nav.path[at].scroll, self.nav.path[at].reveal) = (scroll, false);

        let first = ((scroll - 8.0) / ROW).floor().max(0.0) as usize;
        for (i, entry) in entries.iter().enumerate().skip(first).take((rect.height() / ROW) as usize + 2) {
            let row = Rect::from_min_size(Pos2::new(inner.left(), inner.top() + 8.0 + i as f32 * ROW - scroll), Vec2::new(inner.width(), ROW));
            let on = selected == Some(i);
            if on {
                painter.rect_filled(row, 0.0, ROW_ON);
            }
            let folder = entry.kind == Kind::Folder;
            let color = match entry.kind {
                Kind::Other => ROW_DIM,
                _ if on => Color32::WHITE,
                _ => ROW_TEXT,
            };
            // a folder's mark is wide, a file's tall
            let middle = row.center().y;
            let (mark, edge) = if folder { (Rect::from_min_size(Pos2::new(row.left() + 12.0, middle - 4.5), Vec2::new(11.0, 9.0)), FOLDER_EDGE) } else { (Rect::from_min_size(Pos2::new(row.left() + 13.0, middle - 5.5), Vec2::new(9.0, 11.0)), FILE_EDGE) };
            painter.rect_stroke(mark, 0.0, (1.0, edge), StrokeKind::Inside);
            let mut room = row.width() - 31.0 - 10.0;
            if folder {
                let arrow = painter.layout_no_wrap("›".to_string(), font(SANS, 12.5), GRAY);
                room -= arrow.size().x + 8.0;
                painter.galley(Pos2::new(row.right() - 10.0 - arrow.size().x, top_of(&painter, &arrow, middle)), arrow, GRAY);
            }
            text(&painter, row.left() + 31.0, middle, &entry.name, font(SANS, 12.5), color, room);
            if ui.interact(row.intersect(inner), ui.scope_id().with(("brw-row", at, i)), Sense::click()).clicked() {
                let twice = if folder { Some(Click::Enter) } else { is_card(entry).then(|| Click::Open(entry.path.clone())) };
                clicks.push((Click::Row(at, entry.path.clone(), folder), twice));
            }
        }
    }

    /// the main area: the path line, the note on a network folder, and the grid of cards.
    fn main_area(&mut self, ui: &egui::Ui, rect: Rect, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        let folder = self.nav.main().folder.clone();
        // the path line: the folder's name and what it holds, and at the right the button that plays it
        let line = Rect::from_min_size(rect.min, Vec2::new(rect.width(), PATH_LINE));
        painter.rect_filled(Rect::from_min_max(Pos2::new(line.left(), line.bottom() - 1.0), line.max), 0.0, LINE);
        let middle = line.top() + (PATH_LINE - 1.0) / 2.0;
        let label = painter.layout_no_wrap("play folder".to_string(), font(MONO, 11.5), FG);
        let keys = painter.layout_no_wrap("shift + enter".to_string(), font(MONO_MEDIUM, 9.5), DIM);
        let size = Vec2::new(1.0 + 9.0 + label.size().x + 6.0 + keys.size().x + 8.0 + 9.0 + 1.0, 27.5);
        let play = Rect::from_min_size(Pos2::new(line.right() - 14.0 - size.x, middle - size.y / 2.0), size);
        if button(ui, &painter, play, "brw-play", label, false) {
            clicks.push((Click::PlayFolder, None));
        }
        let cap = Rect::from_min_size(Pos2::new(play.right() - 10.0 - keys.size().x - 8.0, middle - 6.75), Vec2::new(keys.size().x + 8.0, 13.5));
        painter.rect_stroke(cap, 3.0, (1.0, LINE), StrokeKind::Inside);
        painter.galley(Pos2::new(cap.center().x - keys.size().x / 2.0, top_of(&painter, &keys, cap.center().y)), keys, DIM);
        let name = folder.file_name().map_or_else(|| folder.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
        let (left, room) = (line.left() + 14.0, play.left() - 10.0 - line.left() - 14.0);
        let named = text(&painter, left, middle, &name, font(MONO_MEDIUM, 11.5), FG, room);
        let holds = match self.nav.listings.get(&folder) {
            None => "reading…".to_string(),
            Some(Err(_)) => String::new(),
            Some(Ok(entries)) => holds(entries),
        };
        text(&painter, left + named + 10.0, middle, &holds, font(MONO, 11.5), DIM, room - named - 10.0);

        // on a network volume, a note that nothing is opened without asking (fig. 29)
        let mut top = line.bottom();
        if self.nav.volume == Volume::Network {
            let read_here = self.asked.iter().any(|f| f.parent() == Some(folder.as_path()));
            let rest = if read_here { "only the files you asked for have been opened" } else { "nothing here has been opened" };
            let mut job = LayoutJob::default();
            job.wrap.max_width = rect.width() - 28.0 - 3.0 - 24.0;
            let format = |family: &str, color| TextFormat { font_id: font(family, 11.5), color, line_height: Some(11.5 * 1.5), ..Default::default() };
            job.append("network folder: look-only.", 0.0, format(MONO_MEDIUM, FG));
            job.append(&format!(" {rest}. names, sizes and dates come from the folder listing."), 0.0, format(MONO, DIM));
            let galley = painter.layout_job(job);
            let note = Rect::from_min_size(Pos2::new(rect.left() + 14.0, top + 14.0), Vec2::new(rect.width() - 28.0, galley.size().y + 22.0));
            painter.rect_stroke(note, 0.0, (1.0, LINE), StrokeKind::Inside);
            // fig. 29 draws this edge in amber. amber is kept for a fault in a file, and this is none
            painter.rect_filled(Rect::from_min_size(note.min, Vec2::new(2.0, note.height())), 0.0, GRAY);
            painter.galley(note.min + Vec2::new(2.0 + 12.0, 1.0 + 10.0), galley, DIM);
            top = note.bottom();
        }
        self.grid(ui, Rect::from_min_max(Pos2::new(rect.left(), top), rect.max), &folder, clicks);
    }

    /// the cards of the folder shown, as many across as fit; the selected one outlined.
    fn grid(&mut self, ui: &egui::Ui, rect: Rect, folder: &Path, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        self.take_pictures(ui.ctx());
        if self.grid_folder != folder {
            (self.grid_folder, self.grid_scroll, self.revealed) = (folder.to_path_buf(), 0.0, None);
        }
        let say = |words: &str| text(&painter, rect.left() + 14.0, rect.top() + 14.0 + NAME / 2.0, words, font(MONO, 11.5), DIM, rect.width() - 28.0);
        let entries = match self.nav.listings.get(folder) {
            None => {
                say("reading…");
                return self.want(Vec::new());
            }
            Some(Err(why)) => {
                let why = why.to_lowercase();
                self.want(Vec::new());
                return self.cannot_read(ui, rect, &why, clicks);
            }
            Some(Ok(entries)) => entries,
        };
        let cards: Vec<&Entry> = entries.iter().filter(|e| is_card(e)).collect();
        if cards.is_empty() {
            say("no video here");
            return self.want(Vec::new());
        }
        let inner = rect.width() - 28.0;
        let per_row = (((inner + 12.0) / (CARD_MIN + 12.0)).floor() as usize).max(1);
        let width = ((inner - 12.0 * (per_row - 1) as f32) / per_row as f32).floor().max(1.0);
        let thumb = (width * 9.0 / 16.0).round();
        let height = thumb + 6.0 + NAME + 6.0 + SPEC;
        let pitch = height + 14.0;
        let selected = self.nav.main().selected.as_ref();
        let at = selected.and_then(|s| cards.iter().position(|c| &c.path == s));
        // a card newly selected is brought into view; otherwise the wheel moves freely
        let mut scroll = self.grid_scroll;
        if let Some(i) = at.filter(|_| self.revealed.as_ref() != selected) {
            scroll = reveal(scroll, 14.0 + (i / per_row) as f32 * pitch, height, rect.height(), 14.0);
        }
        self.revealed = selected.cloned();
        let rows = cards.len().div_ceil(per_row);
        self.grid_scroll = scroll.clamp(0.0, (rows as f32 * pitch + 14.0 - rect.height()).max(0.0));
        self.per_row = per_row;

        // the cards on screen ask for their posters, and the one the pointer rests on for all of its previews first
        let now = ui.input(|i| i.time);
        let (mut wants, mut hovered) = (Vec::new(), false);
        for (i, card) in cards.iter().enumerate() {
            let top = rect.top() + 14.0 + (i / per_row) as f32 * pitch - self.grid_scroll;
            if top + height < rect.top() || top > rect.bottom() {
                continue;
            }
            let area = Rect::from_min_size(Pos2::new(rect.left() + 14.0 + (i % per_row) as f32 * (width + 12.0), top), Vec2::new(width, height));
            let response = ui.interact(area.intersect(rect), ui.scope_id().with(("brw-card", i)), Sense::click());
            let pictures = self.cards.get(&card.path).filter(|c| c.stamp == stamp(card));
            // the pointer's place across the picture scrubs it
            let scrub = response.hover_pos().map(|p| ((p.x - area.left()) / width).clamp(0.0, 1.0));
            draw_card(&painter, area, thumb, card, at == Some(i), pictures, scrub);
            if response.clicked() {
                clicks.push((Click::Card(card.path.clone()), Some(Click::Open(card.path.clone()))));
            }
            if !nav::makes_pictures(self.nav.volume, card) {
                continue;
            }
            let want = |all| Want { path: card.path.clone(), size: stamp(card).0, modified: stamp(card).1, all };
            if scrub.is_some() && !pictures.is_some_and(|c| c.all) {
                hovered = true;
                // a pointer only crossing the card reads nothing more than its poster
                let wait = rest(&mut self.hover, &card.path, now);
                if wait > 0.0 {
                    ui.ctx().request_repaint_after(Duration::from_secs_f64(wait));
                } else {
                    wants.insert(0, want(true));
                    continue;
                }
            }
            if !pictures.is_some_and(|c| c.poster) {
                wants.push(want(false));
            }
        }
        if !hovered {
            self.hover = None;
        }
        // the pictures of a folder no longer shown are let go
        self.cards.retain(|path, _| cards.iter().any(|c| &c.path == path));
        if !wants.is_empty() {
            ui.ctx().request_repaint_after(POLL);
        }
        self.want(wants);
    }

    /// the folder shown could not be listed: why, and the places to go to instead.
    fn cannot_read(&self, ui: &egui::Ui, rect: Rect, why: &str, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        let (left, width) = (rect.left() + 14.0, rect.width() - 28.0);
        let mut y = rect.top() + 14.0;
        text(&painter, left, y + NAME / 2.0, "this folder cannot be read", font(MONO_MEDIUM, 11.5), FG, width);
        y += NAME + 6.0;
        let galley = painter.layout(why.to_string(), font(MONO, 11.5), DIM, width);
        let height = galley.size().y;
        painter.galley(Pos2::new(left, y), galley, DIM);
        y += height + 14.0;
        let mut x = left;
        for (i, (place, _)) in self.places.iter().filter(|(p, _)| p.path.parent().is_some()).enumerate() {
            let label = painter.layout_no_wrap(place.name.clone(), font(MONO_MEDIUM, 11.0), FG);
            let size = Vec2::new(label.size().x + 18.0, 25.0);
            if x > left && x + size.x > left + width {
                (x, y) = (left, y + size.y + 6.0);
            }
            if button(ui, &painter, Rect::from_min_size(Pos2::new(x, y), size), ("brw-go", i), label, false) {
                clicks.push((Click::Place(place.path.clone()), None));
            }
            x += size.x + 6.0;
        }
    }

    /// the selected card's metadata: a place for its picture, then its facts, and at the foot what can be done with it.
    fn metadata(&mut self, ui: &egui::Ui, rect: Rect, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, PANEL);
        rule(&painter, rect.left(), rect);
        let Some(card) = self.nav.card() else { return };
        let (left, width) = (rect.left() + 1.0 + 14.0, rect.width() - 1.0 - 28.0);
        let readable = nav::readable(card);
        let reads = readable && nav::reads_metadata(self.nav.volume, card, self.asked.contains(&card.path));
        // at the foot: open, and on a network volume, until asked, read metadata
        let foot = rect.bottom() - 12.0 - 25.0;
        let mut x = left;
        let ask = (readable && !reads).then(|| ("read metadata", Click::Read(card.path.clone()), false));
        for (i, (label, click, strong)) in std::iter::once(("open", Click::Open(card.path.clone()), true)).chain(ask).enumerate() {
            let galley = painter.layout_no_wrap(label.to_string(), font(MONO_MEDIUM, 11.0), if strong { ON_STRONG } else { FG });
            let button_rect = Rect::from_min_size(Pos2::new(x, foot), Vec2::new(galley.size().x + 18.0, 25.0));
            if button(ui, &painter, button_rect, ("brw-act", i), galley, strong) {
                clicks.push((click, None));
            }
            x = button_rect.right() + 6.0;
        }

        let painter = painter.with_clip_rect(Rect::from_min_max(rect.min, Pos2::new(rect.right(), foot - 10.0)));
        self.meta_scroll = self.meta_scroll.clamp(0.0, (self.meta_height - (foot - 10.0 - rect.top())).max(0.0));
        let top = rect.top() + 12.0 - self.meta_scroll;
        let picture = Rect::from_min_size(Pos2::new(left, top), Vec2::new(width, (width * 9.0 / 16.0).round()));
        match self.cards.get(&card.path).filter(|c| c.stamp == stamp(card)).and_then(|c| c.shown(None)) {
            Some((_, poster)) => draw_picture(&painter, picture, poster),
            None => {
                painter.rect_filled(picture, 0.0, PLAIN);
            }
        }
        painter.rect_stroke(picture, 0.0, (1.0, LINE), StrokeKind::Outside);
        let listed;
        let groups: &[Vec<Row>] = match self.read.get(&card.path).filter(|_| reads) {
            Some(Ok(meta)) => &meta.groups,
            read => {
                let rest = match read {
                    _ if !readable => fact("everything else", "not read for stills and sequences yet.".to_string(), Tone::Assumed),
                    _ if !reads && card.away => fact("everything else", "not read: the file is not on this disk, and reading it downloads it.".to_string(), Tone::Assumed),
                    _ if !reads => fact("everything else", "not read. opening the file reads it.".to_string(), Tone::Assumed),
                    // not amber: a file locked or out of reach is no fault of the file's
                    Some(Err(why)) => {
                        let why = why.strip_prefix(&format!("{}: ", card.path.to_string_lossy())).unwrap_or(why);
                        fact("everything else", format!("could not be read: {}", why.to_lowercase()), Tone::Assumed)
                    }
                    _ => fact("everything else", "reading…".to_string(), Tone::Assumed),
                };
                listed = vec![listed_facts(card), vec![rest]];
                &listed
            }
        };
        let bottom = rows(&painter, left, picture.bottom() + 10.0, width, groups);
        self.meta_height = bottom - top + 12.0;
    }
}

/// how long the selection has still to rest on `file` before it is read: `REST` from when it got there.
fn rest(resting: &mut Option<(PathBuf, f64)>, file: &Path, now: f64) -> f64 {
    match resting {
        Some((on, since)) if on.as_path() == file => (REST - (now - *since)).max(0.0),
        _ => {
            *resting = Some((file.to_path_buf(), now));
            REST
        }
    }
}

fn list(folder: &Path) -> Listing {
    browse::list(folder).map_err(|e| e.to_string())
}

fn read_meta(file: &Path) -> Result<Meta, String> {
    Meta::read_known(&file.to_string_lossy())
}

/// a scroll that keeps the span from `top`, `height` high, inside a view `view` high with `pad` to spare, moving as
/// little as it can.
fn reveal(scroll: f32, top: f32, height: f32, view: f32, pad: f32) -> f32 {
    scroll.max(top + height + pad - view).min(top - pad)
}

/// one line of text, its left at `left` and centered on `middle`, cut at the end to `width`. returns its width.
fn text(painter: &Painter, left: f32, middle: f32, words: &str, font_id: FontId, color: Color32, width: f32) -> f32 {
    let mut job = LayoutJob::simple_singleline(words.to_string(), font_id, color);
    job.wrap = TextWrapping::truncate_at_width(width.max(0.0));
    let galley = painter.layout_job(job);
    let width = galley.size().x;
    painter.galley(Pos2::new(left, top_of(painter, &galley, middle)), galley, color);
    width
}

/// where the top of a line already laid out goes for it to look centered on `middle`: by its figures (`text_top`),
/// not by its box
fn top_of(painter: &Painter, galley: &Galley, middle: f32) -> f32 {
    galley.job.sections.first().map_or(middle - galley.size().y / 2.0, |s| text_top(painter, s.format.font_id.clone(), middle))
}

/// small text with its letters spaced out, as the design's labels have it
fn spaced(painter: &Painter, words: &str, font_id: FontId, spacing: f32, color: Color32) -> Arc<Galley> {
    let mut job = LayoutJob::default();
    job.append(words, 0.0, TextFormat { font_id, color, extra_letter_spacing: spacing, ..Default::default() });
    painter.layout_job(job)
}

/// a rule one unit wide down `rect` at `x`
fn rule(painter: &Painter, x: f32, rect: Rect) {
    painter.rect_filled(Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(x + 1.0, rect.bottom())), 0.0, LINE);
}

/// an outlined button with its label 9 in from the left (8 for a small one), filled when `strong`. true when clicked.
fn button(ui: &egui::Ui, painter: &Painter, rect: Rect, name: impl std::hash::Hash + std::fmt::Debug, label: Arc<Galley>, strong: bool) -> bool {
    let response = ui.interact(rect, ui.scope_id().with(name), Sense::click());
    if strong {
        painter.rect_filled(rect, 0.0, STRONG);
    } else if response.hovered() {
        painter.rect_filled(rect, 0.0, HOVER);
    }
    painter.rect_stroke(rect, 0.0, (1.0, if strong { STRONG } else { LINE }), StrokeKind::Inside);
    let pad = if rect.height() < 27.0 { 8.0 } else { 9.0 };
    painter.galley(Pos2::new(rect.left() + 1.0 + pad, top_of(painter, &label, rect.center().y)), label, FG);
    response.clicked()
}

/// a picture at its own shape, as large as fits in `rect`, on black
fn draw_picture(painter: &Painter, rect: Rect, picture: &egui::TextureHandle) {
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    let size = picture.size_vec2();
    let fitted = Rect::from_center_size(rect.center(), size * (rect.width() / size.x).min(rect.height() / size.y));
    painter.image(picture.id(), fitted, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
}

/// a card: its picture, or until there is one the file's type in the middle of a plain one (the design's plain
/// card); a sequence's marks at the top right; then the name and what the listing says of the file. while the
/// pointer scrubs it (`scrub`, 0 to 1 across), a line marks the place and the frame shown is named.
fn draw_card(painter: &Painter, area: Rect, thumb: f32, entry: &Entry, on: bool, pictures: Option<&Card>, scrub: Option<f32>) {
    let picture = Rect::from_min_size(area.min, Vec2::new(area.width(), thumb));
    match pictures.and_then(|c| c.shown(scrub)) {
        Some((frame, shown)) => {
            draw_picture(painter, picture, shown);
            if let Some(at) = scrub {
                let x = picture.left() + at * (picture.width() - 1.0);
                painter.rect_filled(Rect::from_min_max(Pos2::new(x, picture.top()), Pos2::new(x + 1.0, picture.bottom())), 0.0, Color32::WHITE);
                let galley = painter.layout_no_wrap(format!("{frame:06}"), font(MONO_MEDIUM, 10.0), FG);
                let label = Rect::from_min_size(Pos2::new(picture.left() + 4.0, picture.bottom() - 4.0 - 16.0), Vec2::new(galley.size().x + 8.0, 16.0));
                painter.rect_filled(label, 0.0, BADGE_BACK);
                painter.galley(Pos2::new(label.left() + 4.0, top_of(painter, &galley, label.center().y)), galley, FG);
            }
        }
        None => {
            painter.rect_filled(picture, 0.0, PLAIN);
            let kind = Path::new(&entry.name).extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
            let galley = spaced(painter, &kind, font(MONO_MEDIUM, 11.0), 1.54, PLAIN_TEXT);
            painter.galley(Pos2::new(picture.center().x - galley.size().x / 2.0, top_of(painter, &galley, picture.center().y)), galley, PLAIN_TEXT);
        }
    }
    painter.rect_stroke(picture, 0.0, if on { (2.0, FG) } else { (1.0, LINE) }, StrokeKind::Outside);
    if let Kind::Sequence { missing, .. } = &entry.kind {
        // from the right: the missing frames, a fault in amber, then the sequence's own mark
        let mut right = picture.right() - 4.0;
        let marks = [(!missing.is_empty()).then(|| (format!("{} missing", missing.len()), AMBER)), Some(("seq".to_string(), BADGE))];
        for (words, color) in marks.into_iter().flatten() {
            let galley = spaced(painter, &words, font(MONO_MEDIUM, 9.0), 0.54, color);
            let mark = Rect::from_min_size(Pos2::new(right - galley.size().x - 8.0, picture.top() + 4.0), Vec2::new(galley.size().x + 8.0, 15.0));
            painter.rect_filled(mark, 0.0, BADGE_BACK);
            painter.galley(Pos2::new(mark.center().x - galley.size().x / 2.0, top_of(painter, &galley, mark.center().y)), galley, color);
            right = mark.left() - 3.0;
        }
    }
    let name = cut_middle(painter, &entry.name, font(MONO_MEDIUM, 11.5), FG, area.width());
    painter.galley(Pos2::new(area.left(), top_of(painter, &name, picture.bottom() + 6.0 + NAME / 2.0)), name, FG);
    let (spec, fault) = spec(entry);
    text(painter, area.left(), picture.bottom() + 6.0 + NAME + 6.0 + SPEC / 2.0, &spec, font(MONO, 10.5), if fault { AMBER } else { DIM }, area.width());
}

/// what a card says under its name, from the listing: for a sequence how many of its frames are there, then the
/// size and when it was last written. true when frames are missing, which is a fault in the file.
fn spec(entry: &Entry) -> (String, bool) {
    let mut parts = Vec::new();
    let mut fault = false;
    if let Kind::Sequence { first, last, missing, .. } = &entry.kind {
        let total = last - first + 1;
        fault = !missing.is_empty();
        parts.push(if fault { format!("{} of {total}", total - missing.len() as u64) } else { format!("{total} frames") });
    }
    parts.extend(entry.size.map(short_size));
    parts.extend(entry.modified.map(when));
    (parts.join(" · "), fault)
}

/// what a folder holds, as the path line says it: "4 folders · 3 videos · 1 sequence"
fn holds(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "empty".to_string();
    }
    let count = |kind: fn(&Kind) -> bool| entries.iter().filter(|e| kind(&e.kind)).count();
    let counts = [(count(|k| *k == Kind::Folder), "folder"), (count(|k| *k == Kind::Clip), "video"), (count(|k| matches!(k, Kind::Sequence { .. })), "sequence")];
    counts.iter().filter(|(n, _)| *n > 0).map(|(n, what)| format!("{n} {what}{}", if *n == 1 { "" } else { "s" })).collect::<Vec<_>>().join(" · ")
}

fn fact(label: &'static str, value: String, tone: Tone) -> Row {
    Row { label, value, tone }
}

/// what the folder listing says of a file: its name, a sequence's frames, its size and when it was last written
fn listed_facts(entry: &Entry) -> Vec<Row> {
    let mut rows = vec![fact("file", entry.name.clone(), Tone::Plain)];
    if let Kind::Sequence { first, last, missing, .. } = &entry.kind {
        let total = last - first + 1;
        rows.push(match missing.len() as u64 {
            0 => fact("frames", total.to_string(), Tone::Plain),
            n => fact("frames", format!("{} of {total} · {n} missing", total - n), Tone::Fault),
        });
    }
    rows.extend(entry.size.map(|s| fact("size", short_size(s), Tone::Plain)));
    rows.extend(entry.modified.map(|t| fact("modified", when(t), Tone::Plain)));
    rows
}

/// the metadata column's facts, each label beside its value, with a rule between groups. a value's first part is
/// bright and what follows its first " · " dim; a fault is amber, an assumption dim. returns where they end.
fn rows(painter: &Painter, left: f32, mut y: f32, width: f32, groups: &[Vec<Row>]) -> f32 {
    let format = |color| TextFormat { font_id: font(MONO, 11.5), color, line_height: Some(META_LINE), ..Default::default() };
    for (g, group) in groups.iter().enumerate() {
        if g > 0 {
            painter.rect_filled(Rect::from_min_size(Pos2::new(left, y + 2.0), Vec2::new(width, 1.0)), 0.0, LINE);
            y += 2.0 + 1.0 + 2.0 + 5.0;
        }
        for row in group {
            let mut label = LayoutJob::default();
            label.wrap.max_width = 86.0;
            label.append(row.label, 0.0, format(DIM));
            let mut value = LayoutJob::default();
            value.wrap.max_width = width - 96.0;
            match row.tone {
                Tone::Fault => value.append(&format!("! {}", row.value), 0.0, format(AMBER)),
                Tone::Assumed => value.append(&row.value, 0.0, format(DIM)),
                Tone::Plain => match row.value.split_once(" · ") {
                    Some((first, rest)) => {
                        value.append(first, 0.0, format(FG));
                        value.append(&format!(" · {rest}"), 0.0, format(DIM));
                    }
                    None => value.append(&row.value, 0.0, format(FG)),
                },
            }
            let (label, value) = (painter.layout_job(label), painter.layout_job(value));
            let height = label.size().y.max(value.size().y);
            painter.galley(Pos2::new(left, y), label, DIM);
            painter.galley(Pos2::new(left + 96.0, y), value, FG);
            y += height + 5.0;
        }
    }
    y - 5.0
}

/// a size to three figures in decimal units, as a card gives it: "4.82 GB"
fn short_size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let (mut v, mut unit) = (bytes as f64, 0);
    while v >= 1000.0 && unit < 4 {
        v /= 1000.0;
        unit += 1;
    }
    let places = if v < 10.0 { 2 } else if v < 100.0 { 1 } else { 0 };
    format!("{v:.places$} {}", ["", "KB", "MB", "GB", "TB"][unit])
}

/// a time from the listing to the minute, in utc: the time zone is not read (as in media/meta.rs, whose own date
/// formatter is private to it).
fn when(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // days to a civil date (howard hinnant's algorithm), with years starting in march so leap days fall last
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!("{year}-{month:02}-{day:02} {:02}:{:02} utc", rest / 3600, rest / 60 % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_read_once_the_selection_rests_on_it() {
        let mut resting = None;
        let (a, b) = (Path::new("a.mov"), Path::new("b.mov"));
        assert_eq!(rest(&mut resting, a, 10.0), REST);
        // passing on to the next card starts over, so an arrow held down reads none of the cards it passes
        assert_eq!(rest(&mut resting, b, 10.1), REST);
        assert!(rest(&mut resting, b, 10.1 + REST / 2.0) > 0.0);
        assert_eq!(rest(&mut resting, b, 11.0), 0.0);
    }

    #[test]
    fn a_card_says_what_the_listing_says() {
        let at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(when(at), "2023-11-14 22:13 utc");
        assert_eq!(when(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29 00:00 utc");
        assert_eq!((short_size(4_820_000_000), short_size(86_000_000), short_size(512)), ("4.82 GB".into(), "86.0 MB".into(), "512 bytes".into()));
        let clip = Entry { name: "a.mov".into(), path: "a.mov".into(), kind: Kind::Clip, size: Some(4_820_000_000), modified: Some(at), away: false };
        assert_eq!(spec(&clip), ("4.82 GB · 2023-11-14 22:13 utc".to_string(), false));
        let seq = Entry { name: "s.[0001-0240].exr".into(), path: "s.0001.exr".into(), kind: Kind::Sequence { first: 1, last: 240, missing: vec![7, 9], pad: 4 }, size: None, modified: None, away: false };
        assert_eq!(spec(&seq), ("238 of 240".to_string(), true));
        assert_eq!(listed_facts(&seq)[1].value, "238 of 240 · 2 missing");
        assert_eq!(holds(&[clip, seq]), "1 video · 1 sequence");
        assert_eq!(holds(&[]), "empty");
    }

    #[test]
    fn the_columns_give_way_to_the_main_area_and_the_metadata_column() {
        let area = |w: f32| Rect::from_min_size(Pos2::ZERO, Vec2::new(w, 668.0));
        // fig. 28's width with a card selected: two columns, the main area, the metadata column
        let layout = Layout::new(area(1240.0), 5, true);
        assert_eq!((layout.first, layout.columns.len(), layout.meta.is_some()), (3, 2, true));
        assert_eq!((layout.main.left(), layout.main.width()), (SIDE + 2.0 * COLUMN, 1240.0 - SIDE - 2.0 * COLUMN - META));
        // with nothing selected there is no metadata column, and more columns fit
        assert_eq!(Layout::new(area(1240.0), 5, false).columns.len(), 4);
        // narrow: one column, the deepest, and no metadata column
        let layout = Layout::new(area(700.0), 5, true);
        assert_eq!((layout.first, layout.columns.len(), layout.meta.is_none()), (4, 1, true));
        // never more columns than the path has
        assert_eq!(Layout::new(area(2400.0), 1, false).columns.len(), 1);
    }
}
