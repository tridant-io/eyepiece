//! the file browser: what an empty tab shows, and what O lays over a clip (the design's figs. 27 to 29). at the left
//! the places and volumes, then the path as finder-style columns, the folder shown in the main area with its videos
//! and sequences as cards, and at the right the selected card's metadata. folders are read from their listing alone,
//! off the drawing thread; on a network volume nothing more is read until asked (adr 0004).

mod jobs;
mod nav;

use super::icons::{self, Icon};
use super::{cut_middle, font, icon_button, text_top, widgets, Lit, AMBER, DIM, FG, HOVER, ICON_BUTTON, LINE, MONO, MONO_MEDIUM, SANS};
use crate::browse::{self, Entry, Group, Kind, Place, View, Volume, Widths};
use crate::event::{Key, Mods};
use crate::folder::{self, Clips};
use crate::media::meta::{Meta, Row, Tone};
use crate::preview::make::{self, Pictures, Want};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align2, Color32, FontId, Galley, Painter, Pos2, Rect, Sense, StrokeKind, Vec2};
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
/// the row the keys act on, rgba(255,255,255,.2); a row selected on the way to it, .08, as a selected place is
const ROW_ON: Color32 = Color32::from_rgba_premultiplied(51, 51, 51, 51);
const PLACE_ON: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
/// the column or grid the keys act in, with nothing selected in it: outlined, as the design's focused player is,
/// rgba(255,255,255,.45)
const RING: Color32 = Color32::from_rgba_premultiplied(115, 115, 115, 115);
/// a card selected on the way to the one the keys act on
const CARD_PATH: Color32 = Color32::from_rgb(0x6d, 0x6d, 0x6d);
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
/// how many recent folders the sidebar keeps
const RECENT: usize = 5;
/// the right-click menu: one column of rows as the player's own right-click menu has them, wide enough for the
/// longest words beside the longest key
const MENU_WIDTH: f32 = 300.0;
const MENU_ROW: f32 = 24.0;
/// what is shown of a folder, as the view's list and the right-click menu name them
const VIEWS: [(View, &str); 3] = [(View::Both, "list + gallery"), (View::Gallery, "gallery"), (View::List, "list")];

/// what was chosen in the browser
pub enum Chosen {
    /// open this file, in the tab the browser is in. a sequence is opened by its first file
    Open(PathBuf),
    /// open this file in a new tab (true) or in the tab the browser is in, whatever the settings say
    OpenIn(PathBuf, bool),
    /// show this file in the system's own file browser
    Reveal(PathBuf),
    /// play this folder
    PlayFolder(PathBuf),
    /// play this folder with its subfolders: the clips their walk found, or why it found none
    PlayTree(PathBuf, Result<Clips, String>),
    /// play these files of one folder, one after another
    PlayFiles(Vec<PathBuf>),
    /// back to the clip under the browser
    Back,
}

pub struct Browser {
    /// where it starts, the folders it last opened files from, and the maker of its cards' pictures: held until it
    /// is first shown, so a launch with a file reads no folder
    start: Option<PathBuf>,
    recent: Vec<PathBuf>,
    pictures: Option<Pictures>,
    view: View,
    widths: Widths,
    state: Option<State>,
}

impl Browser {
    /// starts in `folder`; with none, in the first of the system's places (downloads). without `pictures` the
    /// cards stay plain.
    pub fn new(folder: Option<PathBuf>, pictures: Option<Pictures>) -> Browser {
        Browser { start: folder, recent: Vec::new(), pictures, view: View::Both, widths: Widths::default(), state: None }
    }

    /// the folders it last opened files from, the newest first: kept between runs.
    pub fn recent(&self) -> &[PathBuf] {
        self.state.as_ref().map_or(&self.recent, |state| &state.recent)
    }

    pub fn set_recent(&mut self, recent: Vec<PathBuf>) {
        match &mut self.state {
            Some(state) => state.recent = recent,
            None => self.recent = recent,
        }
    }

    /// the browser is not on screen: no pictures are wanted, its menu goes, and a walk on its way is dropped.
    pub fn away(&mut self) {
        if let Some(state) = &mut self.state {
            state.want(Vec::new());
            state.menu = None;
            state.walks.keep_only(|_| false);
        }
    }

    /// a folder is being walked for its subfolders' clips, to play them.
    pub fn walking(&self) -> bool {
        self.state.as_ref().is_some_and(|state| state.walks.busy())
    }

    /// the folder the main area shows: kept between runs.
    pub fn folder(&self) -> &Path {
        match &self.state {
            Some(state) => &state.nav.main().folder,
            None => self.start.as_deref().unwrap_or(Path::new("")),
        }
    }

    fn state(&mut self) -> &mut State {
        let (start, recent, pictures, view, widths) = (&mut self.start, &mut self.recent, &mut self.pictures, self.view, self.widths);
        self.state.get_or_insert_with(|| {
            let mut state = State::new(start.take(), std::mem::take(recent), pictures.take());
            (state.nav.view, state.widths) = (view, widths);
            state
        })
    }

    pub fn key(&mut self, key: Key, mods: Mods) {
        let state = self.state();
        state.take_answers();
        // a key puts the right-click menu away, and esc does nothing more
        if state.menu.take().is_some() && key == Key::Escape {
            return;
        }
        // control + shift + enter plays the folder shown with its subfolders
        if key == Key::Enter && mods.shift && mods.ctrl {
            let shown = state.nav.main().folder.clone();
            return state.click(Click::PlayTree(shown));
        }
        // enter opens the videos picked together; any other key lets them go
        if key == Key::Enter && !mods.shift && state.picked.len() > 1 {
            return state.click(Click::OpenPicked);
        }
        state.picked.clear();
        if let Some(chosen) = state.nav.key(key, mods, state.per_row) {
            state.choose(chosen);
        }
    }

    /// shift, and control (the mac's command), as they were held with the last pointer press.
    pub fn set_held(&mut self, shift: bool, more: bool) {
        if let Some(state) = &mut self.state {
            state.held = (shift, more);
        }
    }

    /// the folder shown is listed and holds no video or still: a screenshot may be written there, never beside media.
    pub fn clear_of_media(&self) -> bool {
        self.state.as_ref().is_some_and(|state| matches!(state.nav.listings.get(&state.nav.main().folder), Some(Ok(_))) && state.nav.cards().next().is_none())
    }

    /// how wide its parts are: kept between runs.
    pub fn widths(&self) -> Widths {
        self.state.as_ref().map_or(self.widths, |state| state.widths)
    }

    pub fn set_widths(&mut self, widths: Widths) {
        self.widths = widths.kept();
        if let Some(state) = &mut self.state {
            state.widths = widths.kept();
        }
    }

    /// the pointer is on an edge that resizes a part, or is dragging one: the window shows the pointer for that.
    pub fn over_edge(&self) -> bool {
        self.state.as_ref().is_some_and(|state| state.over_edge)
    }

    /// what is shown of a folder: kept between runs.
    pub fn view(&self) -> View {
        self.state.as_ref().map_or(self.view, |state| state.nav.view)
    }

    pub fn set_view(&mut self, view: View) {
        self.view = view;
        if let Some(state) = &mut self.state {
            state.nav.view = view;
        }
    }

    /// back (-1) or forward (1) along the folders the browser has shown.
    pub fn retrace(&mut self, by: isize) {
        self.state().click(Click::Retrace(by));
    }

    /// reads the folders on screen again.
    pub fn refresh(&mut self) {
        self.state().click(Click::Refresh);
    }

    /// makes the address a field to type into (control + L, as in the system's file browser and a web browser).
    pub fn edit_address(&mut self) {
        self.state().click(Click::Address);
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

    /// draws the browser over `area`. returns what was chosen, by key or by pointer. `new_tab`: the settings open a
    /// file from here in a tab of its own, so the right-click menu offers this tab as the other place.
    pub fn draw(&mut self, ui: &mut egui::Ui, area: Rect, new_tab: bool) -> Option<Chosen> {
        let state = self.state();
        state.new_tab = new_tab;
        state.draw(ui, area)
    }
}

struct State {
    nav: Nav,
    /// the sidebar: the places, the folders pinned in explorer and the volumes, each with the kind of volume it is on;
    /// their paths, where the columns start; and the recent folders, which are no place to start at
    places: Vec<(Place, Volume)>,
    roots: Vec<PathBuf>,
    recent: Vec<PathBuf>,
    /// the folders pinned in explorer, read off the drawing thread
    pins: Jobs<Vec<Place>>,
    lists: Jobs<Listing>,
    metas: Jobs<Result<Meta, String>>,
    /// listings read before the browser was last away: shown while they are read again
    stale: HashSet<PathBuf>,
    /// the metadata read: the selected card's, and that of the files asked about
    read: HashMap<PathBuf, Result<Meta, String>>,
    /// the files whose metadata was asked for, or that were opened from here: on a network volume, read
    asked: HashSet<PathBuf>,
    /// the folders whose previews were asked for: on a network volume, their cards get pictures
    previews: HashSet<PathBuf>,
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
    /// the address while it is a field being typed into, and that it has just become one
    address: Option<String>,
    address_fresh: bool,
    address_at: f64,
    /// the list of views is open under its button
    view_menu: bool,
    /// the videos picked together with shift or control, when there are more than the one selected; and the one a
    /// run picked with shift starts from
    picked: Vec<PathBuf>,
    anchor: Option<PathBuf>,
    /// shift and control, as held with the last pointer press
    held: (bool, bool),
    /// how wide the parts are, and whether the pointer is on an edge that resizes one
    widths: Widths,
    over_edge: bool,
    /// the pass it was last drawn in
    drawn: u64,
    /// a folder being walked for its subfolders' clips, to play them: one at a time. a network folder's walk is kept
    /// while the player's ask about it stands: asked for again then, it plays what was counted, with no second walk
    walks: Jobs<Result<Clips, String>>,
    walked: Option<(PathBuf, Clips, Instant)>,
    /// the right-click menu: what it is on and where it was asked for; and where it was last drawn
    menu: Option<(On, Pos2)>,
    menu_rect: Rect,
    /// a path the menu copied, for the clipboard
    copied: Option<String>,
    /// the settings open a file from here in a tab of its own
    new_tab: bool,
}

/// what the right-click menu is on
#[derive(Clone, Debug, PartialEq)]
enum On {
    /// a file: a video's card, or a file's row in a column
    File(PathBuf),
    /// a folder's row in a column
    Folder(PathBuf),
    /// the main area away from its cards: the folder it shows
    Shown(PathBuf),
}

/// a row of the right-click menu: its words, its key where it has one, what it does, and for a view whether it is the
/// one shown
type MenuRow = (&'static str, &'static str, Click, Option<bool>);

/// a press in the browser: what it asks for, done once everything is drawn, and what a double click on the same thing
/// asks for
type Press = (Click, Option<Click>);

#[derive(Clone)]
enum Click {
    Place(PathBuf),
    /// an entry of the drawn column at this place on the path; true for a folder
    Row(usize, PathBuf, bool),
    Enter,
    Card(PathBuf),
    Open(PathBuf),
    /// in a new tab (true) or in this one, whatever the settings say
    OpenIn(PathBuf, bool),
    Reveal(PathBuf),
    PlayFolder(PathBuf),
    /// this folder with its subfolders: they are walked first
    PlayTree(PathBuf),
    Read(PathBuf),
    /// this path to the clipboard
    Copy(PathBuf),
    /// the right-click menu opens on this, at this place
    Menu(On, Pos2),
    /// the previews of this folder
    Previews(PathBuf),
    /// the address becomes a field to type into; and what was typed there, entered
    Address,
    Go(PathBuf),
    /// back (-1) or forward (1) along the folders shown; and the folders on screen read again
    Retrace(isize),
    Refresh,
    /// the list of views opens or closes; and one of them is chosen
    ViewMenu,
    View(View),
    /// a video pressed with control held (one more picked, or one less) or with shift (true: a run of them), in a row
    /// or a card, which the keys then act in; and the videos picked, opened together
    Pick(PathBuf, bool, nav::Focus),
    OpenPicked,
}

/// where the parts go: the sidebar, the address across the top of the rest, and under it the columns that fit (the
/// deepest ones), the main area, the metadata column
struct Layout {
    side: Rect,
    address: Rect,
    /// the first column on the path that is drawn, and where each drawn one goes
    first: usize,
    columns: Vec<Rect>,
    main: Rect,
    meta: Option<Rect>,
}

impl Layout {
    /// `shown` columns on the path; `card`: one is selected, so the metadata column is wanted.
    fn new(area: Rect, shown: usize, card: bool, view: View, widths: Widths) -> Layout {
        let (side, wide) = (widths.side, widths.meta);
        // with the column alone showing, the cards keep no room
        let least = if view == View::List { 0.0 } else { MAIN_MIN };
        let deepest = widths.column(shown.saturating_sub(1));
        let meta = card && area.width() - side - deepest - wide >= least;
        let room = area.width() - side - if meta { wide } else { 0.0 } - least;
        // the deepest columns that fit, each at its own width, and always the deepest one
        let mut fit = 0;
        let mut taken = 0.0;
        while fit < shown && (fit == 0 || taken + widths.column(shown - fit - 1) <= room) {
            taken += widths.column(shown - fit - 1);
            fit += 1;
        }
        let first = shown - fit;
        let x = |offset: f32| area.left() + offset;
        let top = area.top() + PATH_LINE;
        let span = |from: f32, to: f32| Rect::from_min_max(Pos2::new(from, top), Pos2::new(to.max(from), area.bottom()));
        let mut left = side;
        let columns = (first..shown)
            .map(|depth| {
                left += widths.column(depth);
                span(x(left - widths.column(depth)), x(left))
            })
            .collect();
        Layout {
            side: Rect::from_min_max(area.min, Pos2::new(x(side), area.bottom())),
            address: Rect::from_min_max(Pos2::new(x(side), area.top()), Pos2::new(area.right(), top)),
            first,
            columns,
            main: span(x(left), if meta { area.right() - wide } else { area.right() }),
            meta: meta.then(|| span(area.right() - wide, area.right())),
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
    fn new(start: Option<PathBuf>, recent: Vec<PathBuf>, pictures: Option<Pictures>) -> State {
        let places = browse::places();
        let folder = start.or_else(|| places.first().map(|p| p.path.clone())).unwrap_or_default();
        let roots: Vec<PathBuf> = places.iter().map(|p| p.path.clone()).collect();
        let mut pins = Jobs::new();
        pins.ask(PathBuf::new(), |_| browse::pinned());
        State {
            nav: Nav::new(&folder, &roots),
            places: places
                .into_iter()
                .map(|place| {
                    let volume = browse::volume_of(&place.path);
                    (place, volume)
                })
                .collect(),
            roots,
            recent,
            pins,
            lists: Jobs::new(),
            metas: Jobs::new(),
            stale: HashSet::new(),
            read: HashMap::new(),
            asked: HashSet::new(),
            previews: HashSet::new(),
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
            address: None,
            address_fresh: false,
            address_at: 0.0,
            view_menu: false,
            picked: Vec::new(),
            anchor: None,
            held: (false, false),
            widths: Widths::default(),
            over_edge: false,
            drawn: 0,
            walks: Jobs::new(),
            walked: None,
            menu: None,
            menu_rect: Rect::NOTHING,
            copied: None,
            new_tab: false,
        }
    }

    /// what the host is asked for. a file opened from here counts as asked about, and the folder a file is opened or
    /// played from becomes the newest recent one.
    fn choose(&mut self, chosen: Chosen) {
        match &chosen {
            Chosen::Open(file) | Chosen::OpenIn(file, _) => {
                self.asked.insert(file.clone());
                if let Some(folder) = file.parent() {
                    self.remember(folder.to_path_buf());
                }
            }
            Chosen::PlayFolder(folder) | Chosen::PlayTree(folder, _) => self.remember(folder.clone()),
            Chosen::PlayFiles(files) => {
                self.asked.extend(files.iter().cloned());
                if let Some(folder) = files.first().and_then(|file| file.parent()) {
                    self.remember(folder.to_path_buf());
                }
            }
            Chosen::Reveal(_) | Chosen::Back => {}
        }
        // a walk on its way is dropped for anything else asked for: it would play after the user had moved on
        if !matches!(chosen, Chosen::PlayTree(..) | Chosen::Reveal(_)) {
            self.walks.keep_only(|_| false);
        }
        self.chosen = Some(chosen);
    }

    /// a folder files were opened from, first among the recent ones. a place of the sidebar is there already
    fn remember(&mut self, folder: PathBuf) {
        if self.roots.contains(&folder) {
            return;
        }
        self.recent.retain(|f| *f != folder);
        self.recent.insert(0, folder);
        self.recent.truncate(RECENT);
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

    /// the folders pinned in explorer join the sidebar after the places, but for the places themselves, which explorer
    /// pins on its own, and the drives.
    fn pin(&mut self, pinned: Vec<Place>) {
        let pinned: Vec<Place> = pinned.into_iter().filter(|p| !self.roots.contains(&p.path)).collect();
        self.roots.extend(pinned.iter().map(|p| p.path.clone()));
        self.places.extend(pinned.into_iter().map(|place| {
            let volume = browse::volume_of(&place.path);
            (place, volume)
        }));
        self.places.sort_by_key(|(place, _)| place.group);
    }

    fn take_answers(&mut self) {
        for (_, pinned) in self.pins.answers() {
            self.pin(pinned);
        }
        for (folder, listing) in self.lists.answers() {
            self.stale.remove(&folder);
            self.nav.listings.insert(folder, listing);
        }
        self.read.extend(self.metas.answers());
        for (folder, clips) in self.walks.answers() {
            if let Some(found) = clips.as_ref().ok().filter(|_| browse::volume_of(&folder) == Volume::Network) {
                self.walked = Some((folder.clone(), found.clone(), Instant::now()));
            }
            self.choose(Chosen::PlayTree(folder, clips));
        }
        self.nav.settle();
    }

    fn draw(&mut self, ui: &mut egui::Ui, area: Rect) -> Option<Chosen> {
        // away since the last pass (a clip was up): what is on screen may have changed, so it is listed again
        let pass = ui.ctx().cumulative_pass_nr();
        if pass > self.drawn + 1 {
            self.reread();
        }
        self.drawn = pass;
        self.take_answers();
        ui.painter().with_clip_rect(area).rect_filled(area, 0.0, BACK);
        // a press here is not a press on a picture under the browser
        ui.interact(area, ui.scope_id().with("browser"), Sense::click_and_drag());

        // videos picked together belong to the folder shown: gone from it, they are let go
        if self.picked.first().is_some_and(|picked| picked.parent() != Some(self.nav.main().folder.as_path())) {
            self.picked.clear();
        }
        let card = self.nav.card().is_some();
        let mut layout = Layout::new(area, self.nav.drawn(), card, self.nav.view, self.widths);
        // the column the keys act in is drawn, before the one it shows a folder in
        if let Some(at) = self.nav.current_column().filter(|at| *at < layout.first) {
            layout = Layout::new(area, at + 1, card, self.nav.view, self.widths);
        }
        self.scroll(ui, &layout);
        let mut clicks = Vec::new();
        self.side(ui, layout.side, &mut clicks);
        self.address_line(ui, layout.address, &mut clicks);
        for (i, rect) in layout.columns.iter().enumerate() {
            self.column(ui, layout.first + i, *rect, &mut clicks);
        }
        if self.nav.view == View::List {
            self.list_pictures(ui.ctx());
        } else {
            self.main_area(ui, layout.main, &mut clicks);
        }
        if let Some(rect) = layout.meta {
            self.metadata(ui, rect, &mut clicks);
        }
        // after the parts, so an edge takes the pointer before the rows and cards under it
        self.edges(ui, &layout);
        self.menu(ui, area.shrink(8.0), &mut clicks);
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
        // as text copied from a field goes: the window puts it on the system's clipboard
        if let Some(path) = self.copied.take() {
            ui.ctx().copy_text(path);
        }
        if let Some(wait) = self.ask(layout.first, ui.input(|i| i.time)) {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(wait));
        }
        if self.lists.busy() || self.metas.busy() || self.pins.busy() || self.walks.busy() {
            ui.ctx().request_repaint_after(POLL);
        }
        self.chosen.take()
    }

    fn click(&mut self, click: Click) {
        // a plain press on anything that selects or goes somewhere lets go of the videos picked together
        if matches!(click, Click::Place(_) | Click::Row(..) | Click::Enter | Click::Card(_) | Click::Go(_) | Click::Retrace(_)) {
            self.picked.clear();
        }
        match click {
            Click::Place(place) => self.nav.go(&place, &self.roots),
            Click::Row(at, entry, folder) => {
                self.nav.select(at, entry, folder);
                self.nav.focus = nav::Focus::Columns;
            }
            Click::Enter => self.nav.enter(),
            Click::Card(card) => {
                self.anchor = Some(card.clone());
                self.nav.choose_card(card);
            }
            Click::Pick(card, run, focus) => {
                self.pick(card, run);
                self.nav.focus = focus;
            }
            // in the order the folder lists them, whatever order they were picked in
            Click::OpenPicked => {
                let files: Vec<PathBuf> = self.nav.cards().map(|e| e.path.clone()).filter(|path| self.picked.contains(path)).collect();
                self.choose(Chosen::PlayFiles(files));
            }
            Click::Open(file) => self.choose(Chosen::Open(file)),
            Click::OpenIn(file, new_tab) => self.choose(Chosen::OpenIn(file, new_tab)),
            Click::Reveal(file) => self.choose(Chosen::Reveal(file)),
            Click::PlayFolder(to) => self.choose(Chosen::PlayFolder(to)),
            // off the drawing thread, as a folder is listed: played when its answer comes in
            Click::PlayTree(to) => match self.walked.take().filter(|(of, _, at)| *of == to && at.elapsed() < folder::ASKED_FOR) {
                Some((_, clips, _)) => self.choose(Chosen::PlayTree(to, Ok(clips))),
                None => {
                    self.walks.keep_only(|_| false);
                    self.walks.ask(to, folder::walk);
                }
            },
            Click::Read(file) => {
                self.asked.insert(file);
            }
            Click::Copy(path) => self.copied = Some(path.to_string_lossy().into_owned()),
            Click::Menu(on, at) => (self.menu, self.view_menu) = (Some((on, at)), false),
            Click::Previews(folder) => {
                self.previews.insert(folder);
            }
            Click::Address => {
                self.address = Some(self.nav.main().folder.to_string_lossy().into_owned());
                self.address_fresh = true;
            }
            // from the folder shown, so a folder's name alone goes into it. a file named by its type opens: the user
            // asked for that file. anything else is taken for a folder, and listed
            Click::Go(to) => {
                let to = self.nav.main().folder.join(to);
                if to.file_name().and_then(|name| name.to_str()).is_some_and(browse::is_media) {
                    self.choose(Chosen::Open(to));
                } else {
                    self.nav.go(&to, &self.roots);
                }
            }
            Click::Retrace(by) => self.nav.retrace(by, &self.roots),
            Click::ViewMenu => self.view_menu = !self.view_menu,
            Click::View(view) => (self.nav.view, self.view_menu) = (view, false),
            // what was read of the files is read again as well: a render may have written over one
            Click::Refresh => {
                self.reread();
                self.read.clear();
            }
        }
        self.nav.visit(false);
    }

    /// a video pressed with control held is one more picked, or one less; with shift, all from the one last pressed
    /// to this one are, in the order the folder lists them. the one the metadata column shows is the last one picked.
    fn pick(&mut self, card: PathBuf, run: bool) {
        let cards: Vec<PathBuf> = self.nav.cards().map(|e| e.path.clone()).collect();
        let Some(to) = cards.iter().position(|c| *c == card) else { return };
        let selected = self.nav.main().selected.clone().filter(|s| cards.contains(s));
        if run {
            let from = self.anchor.clone().filter(|a| cards.contains(a)).or(selected).and_then(|a| cards.iter().position(|c| *c == a)).unwrap_or(0);
            self.anchor = Some(cards[from].clone());
            self.picked = cards[from.min(to)..=from.max(to)].to_vec();
        } else {
            if self.picked.is_empty() {
                self.picked.extend(selected);
            }
            match self.picked.iter().position(|p| *p == card) {
                Some(at) => drop(self.picked.remove(at)),
                None => self.picked.push(card.clone()),
            }
            self.anchor = Some(card.clone());
        }
        self.nav.choose_card(self.picked.last().cloned().filter(|_| !self.picked.contains(&card)).unwrap_or(card));
    }

    /// the edges the parts are resized by, each 7 wide over its rule: the sidebar's right, every folder column's right
    /// (each column its own), the metadata column's left. a double click puts an edge back where the design has it.
    fn edges(&mut self, ui: &egui::Ui, layout: &Layout) {
        let at = ui.ctx().pointer_latest_pos();
        let grab = |name: usize, x: f32, rect: Rect| {
            let edge = Rect::from_min_max(Pos2::new(x - 3.5, rect.top()), Pos2::new(x + 3.5, rect.bottom()));
            let response = ui.interact(edge, ui.scope_id().with(("brw-edge", name)), Sense::click_and_drag());
            let to = at.filter(|_| response.dragged()).map(|at| at.x);
            (response.hovered() || response.dragged(), to, response.double_clicked())
        };
        let start = layout.side.left();
        let mut over = false;
        let (on, to, back) = grab(0, layout.side.right(), layout.side);
        over |= on;
        if let Some(x) = to {
            self.widths.side = x - start;
        }
        if back {
            self.widths.side = Widths::default().side;
        }
        for (i, rect) in layout.columns.iter().enumerate() {
            let (on, to, back) = grab(1 + i, rect.right(), *rect);
            over |= on;
            if let Some(x) = to {
                self.widths.set_column(layout.first + i, x - rect.left());
            }
            if back {
                self.widths.set_column(layout.first + i, browse::COLUMN);
            }
        }
        if let Some(rect) = layout.meta {
            let (on, to, back) = grab(usize::MAX, rect.left(), rect);
            over |= on;
            if let Some(x) = to {
                self.widths.meta = rect.right() - x;
            }
            if back {
                self.widths.meta = Widths::default().meta;
            }
        }
        (self.widths, self.over_edge) = (self.widths.kept(), over);
    }

    /// what is listed is listed again: it stays on screen until the new listing comes.
    fn reread(&mut self) {
        self.stale = self.nav.listings.keys().cloned().collect();
    }

    /// the address (the owner's ask, 2026-11-24): a line across the top, over the columns and the main area, with
    /// every folder from the drive or share down to the one shown, or the field a path is typed into.
    fn address_line(&mut self, ui: &egui::Ui, rect: Rect, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 1.0), rect.max), 0.0, LINE);
        let middle = rect.top() + (PATH_LINE - 1.0) / 2.0;
        // at its left, as a file browser has them: back and forward along the folders shown, dim where there is
        // nowhere to go, and read again (the owner's ask, 2026-11-24). pressed twice fast, each acts twice
        let mut left = rect.left() + 6.0;
        for (name, icon, click) in [("brw-back", Icon::Back, Click::Retrace(-1)), ("brw-forward", Icon::Forward, Click::Retrace(1)), ("brw-refresh", Icon::Refresh, Click::Refresh)] {
            let button = Rect::from_min_size(Pos2::new(left, middle - ICON_BUTTON.y / 2.0), ICON_BUTTON);
            left = button.right() + 2.0;
            if matches!(click, Click::Retrace(by) if !self.nav.can_retrace(by)) {
                icons::draw(&painter, button, icon, ROW_DIM);
            } else if icon_button(ui, button, name, icon, Lit::No).clicked() {
                clicks.push((click.clone(), Some(click)));
            }
        }
        // at its right, where nothing comes and goes: what is shown of a folder, and open folder where the folder
        // holds more than one video. (as "play folder" it stood at the main area's right, and jumped away when the
        // metadata column came in: the owner saw it flash, 2026-11-24.)
        let names = VIEWS;
        let now = self.nav.view;
        let label = painter.layout_no_wrap(names.iter().find(|(view, _)| *view == now).map_or("", |(_, name)| name).to_string(), font(SANS, 12.0), FG);
        let size = Vec2::new(1.0 + 9.0 + label.size().x + 8.0 + 7.0 + 9.0 + 1.0, 27.5);
        let view = Rect::from_min_size(Pos2::new(rect.right() - 10.0 - size.x, middle - size.y / 2.0), size);
        let over_view = ui.rect_contains_pointer(view);
        if button(ui, &painter, view, "brw-view", label, false) {
            clicks.push((Click::ViewMenu, None));
        }
        let tip = Pos2::new(view.right() - 13.5, middle + 2.0);
        painter.line_segment([tip + Vec2::new(-3.5, -3.5), tip], (1.2, DIM));
        painter.line_segment([tip, tip + Vec2::new(3.5, -3.5)], (1.2, DIM));
        if self.view_menu {
            let size = Vec2::new(view.width().max(132.0), 4.0 + names.len() as f32 * 26.0 + 4.0);
            let mut inside = false;
            egui::Area::new(egui::Id::unique("brw-view-menu")).order(egui::Order::Foreground).fixed_pos(Pos2::new(view.right() - size.x, view.bottom() + 4.0)).show(ui.ctx(), |ui| {
                let (list, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter().rect_filled(list, 0.0, PANEL);
                ui.painter().rect_stroke(list, 0.0, (1.0, LINE), StrokeKind::Inside);
                for (i, (to, name)) in names.iter().enumerate() {
                    let row = Rect::from_min_size(Pos2::new(list.left() + 1.0, list.top() + 4.0 + i as f32 * 26.0), Vec2::new(list.width() - 2.0, 26.0));
                    let response = ui.interact(row, ui.scope_id().with(("brw-view-row", i)), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(row, 0.0, HOVER);
                    }
                    let on = *to == now;
                    text(ui.painter(), row.left() + 10.0, row.center().y, name, font(if on { "sans-semibold" } else { SANS }, 12.0), if on { FG } else { DIM }, row.width() - 20.0);
                    if response.clicked() {
                        clicks.push((Click::View(*to), None));
                    }
                }
                inside = ui.rect_contains_pointer(list);
            });
            // a press anywhere else puts the list away
            if ui.input(|i| i.pointer.any_pressed()) && !inside && !over_view {
                self.view_menu = false;
            }
        }
        let mut right = view.left() - 6.0;
        if self.nav.cards().nth(1).is_some() {
            let label = painter.layout_no_wrap("open folder".to_string(), font(SANS, 12.0), FG);
            let keys = painter.layout_no_wrap("shift + enter".to_string(), font(MONO_MEDIUM, 9.5), DIM);
            let size = Vec2::new(1.0 + 9.0 + label.size().x + 6.0 + keys.size().x + 8.0 + 9.0 + 1.0, 27.5);
            let play = Rect::from_min_size(Pos2::new(right - size.x, middle - size.y / 2.0), size);
            if button(ui, &painter, play, "brw-play", label, false) {
                clicks.push((Click::PlayFolder(self.nav.main().folder.clone()), None));
            }
            let cap = Rect::from_min_size(Pos2::new(play.right() - 10.0 - keys.size().x - 8.0, middle - 6.75), Vec2::new(keys.size().x + 8.0, 13.5));
            painter.rect_stroke(cap, 3.0, (1.0, LINE), StrokeKind::Inside);
            painter.galley(Pos2::new(cap.center().x - keys.size().x / 2.0, top_of(&painter, &keys, cap.center().y)), keys, DIM);
            right = play.left() - 6.0;
        }
        let (left, room) = (left + 6.0, right - 8.0 - left - 6.0);
        if self.address.is_some() {
            self.address_field(ui, Rect::from_min_size(Pos2::new(left - 8.0, middle - 13.75), Vec2::new(room + 16.0, 27.5)), clicks);
        } else {
            let folder = self.nav.main().folder.clone();
            self.address_crumbs(ui, &painter, &folder, left, middle, room, clicks);
        }
    }

    /// the address, while it is not being typed into: the folders from the drive or share down to the one shown,
    /// each a press away, the first ones left out behind an ellipsis when the line is too short. a press on the rest
    /// of the line, or on the folder shown, makes the line a field.
    fn address_crumbs(&self, ui: &egui::Ui, painter: &Painter, folder: &Path, left: f32, middle: f32, room: f32, clicks: &mut Vec<Press>) {
        let line = Rect::from_min_max(Pos2::new(left, middle - 13.75), Pos2::new(left + room, middle + 13.75));
        if ui.interact(line, ui.scope_id().with("brw-address"), Sense::click()).clicked() {
            clicks.push((Click::Address, None));
        }
        let crumbs = crumbs(folder);
        let last = crumbs.len().saturating_sub(1);
        let laid: Vec<Arc<Galley>> = crumbs.iter().enumerate().map(|(i, (name, _))| painter.layout_no_wrap(name.clone(), font(if i == last { MONO_MEDIUM } else { MONO }, 11.5), FG)).collect();
        let between = painter.layout_no_wrap(" / ".to_string(), font(MONO, 11.5), ROW_DIM);
        let more = painter.layout_no_wrap("…".to_string(), font(MONO, 11.5), ROW_DIM);
        let wide = |first: usize| laid[first..].iter().map(|g| g.size().x).sum::<f32>() + between.size().x * (laid.len() - first - 1) as f32 + if first > 0 { more.size().x + between.size().x } else { 0.0 };
        let first = (0..=last).find(|&first| wide(first) <= room).unwrap_or(last);
        let mut x = left;
        if first > 0 {
            painter.galley(Pos2::new(x, top_of(painter, &more, middle)), more.clone(), ROW_DIM);
            x += more.size().x;
            painter.galley(Pos2::new(x, top_of(painter, &between, middle)), between.clone(), ROW_DIM);
            x += between.size().x;
        }
        for (i, galley) in laid.into_iter().enumerate().skip(first) {
            let at = Rect::from_min_size(Pos2::new(x, line.top()), Vec2::new(galley.size().x, line.height())).intersect(line);
            let response = ui.interact(at, ui.scope_id().with(("brw-crumb", i)), Sense::click());
            if response.clicked() {
                clicks.push((if i == last { Click::Address } else { Click::Place(crumbs[i].1.clone()) }, None));
            }
            let color = if i == last || response.hovered() { FG } else { DIM };
            x += galley.size().x;
            painter.galley_with_override_text_color(Pos2::new(at.left(), top_of(painter, &galley, middle)), galley, color);
            if i < last {
                painter.galley(Pos2::new(x, top_of(painter, &between, middle)), between.clone(), ROW_DIM);
                x += between.size().x;
            }
        }
    }

    /// the address as a field: the path as the system writes it, all of it selected, to type over or paste into.
    /// enter goes there; escape, or a press elsewhere, leaves the browser where it was. outlined as markup's field
    /// is: the accent is for the menus' fields alone.
    fn address_field(&mut self, ui: &egui::Ui, rect: Rect, clicks: &mut Vec<Press>) {
        let fresh = std::mem::take(&mut self.address_fresh);
        let now = ui.input(|i| i.time);
        if fresh {
            self.address_at = now;
        }
        // a double click that made the line a field would leave the word under its second press selected: the
        // whole path stays selected
        let whole = fresh || now - self.address_at < 0.5 && ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
        let Some(text) = &mut self.address else { return };
        let id = egui::Id::unique("brw-address-edit");
        let mut entered = None;
        egui::Area::new(egui::Id::unique("brw-address-field")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ui.ctx(), |ui| {
            // an area keeps the width it first had (egui's 600 at most): the line's own, every pass
            ui.set_width(rect.width());
            // the line stands in the middle of the field's height, as much room over it as under: egui would give
            // the field its own least height and leave the line at the top
            ui.spacing_mut().interact_size.y = 0.0;
            let line = ui.painter().layout_no_wrap("0".to_string(), font(MONO, 11.5), FG).size().y;
            let over = ((rect.height() - 2.0 - line) / 2.0).floor();
            let margin = egui::Margin { left: 7, right: 7, top: over as i8, bottom: (rect.height() - 2.0 - line - over).ceil() as i8 };
            egui::Frame::new().fill(BACK).stroke((1.0, FG)).inner_margin(margin).show(ui, |ui| {
                let edit = ui.add(egui::TextEdit::singleline(text).id(id).frame(egui::Frame::NONE).margin(egui::Margin::ZERO).font(font(MONO, 11.5)).text_color(FG).desired_width(rect.width() - 16.0));
                if fresh {
                    edit.request_focus();
                }
                if whole {
                    let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()))));
                    state.store(ui.ctx(), id);
                }
                if edit.lost_focus() {
                    entered = Some(ui.input(|i| i.key_pressed(egui::Key::Enter)));
                }
            });
        });
        if let Some(go) = entered {
            let text = self.address.take().unwrap_or_default();
            if let Some(to) = typed(&text).filter(|_| go) {
                clicks.push((Click::Go(to), None));
            }
        }
    }

    /// with the column alone showing no card is drawn, and the selected file's poster is still wanted, for the
    /// metadata column: asked for as a card's is, and under the same rule on a network volume.
    fn list_pictures(&mut self, ctx: &egui::Context) {
        self.take_pictures(ctx);
        let asked = self.previews.contains(&self.nav.main().folder);
        let card = self.nav.card().filter(|card| nav::makes_pictures(self.nav.volume, card, asked));
        let wants: Vec<Want> = card.filter(|card| !self.cards.get(&card.path).is_some_and(|c| c.stamp == stamp(card) && c.poster)).map(|card| Want { path: card.path.clone(), size: stamp(card).0, modified: stamp(card).1, all: false, asked }).into_iter().collect();
        let selected = card.map(|card| card.path.clone());
        self.cards.retain(|path, _| Some(path) == selected.as_ref());
        if !wants.is_empty() {
            ctx.request_repaint_after(POLL);
        }
        self.want(wants);
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

    /// the sidebar, as fig. 27 has it: the places, the folders pinned in explorer, the volumes and the recent folders,
    /// each group under its heading, and anything on a network volume tagged.
    fn side(&mut self, ui: &egui::Ui, rect: Rect, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, PANEL);
        rule(&painter, rect.right() - 1.0, rect);
        self.side_scroll = self.side_scroll.clamp(0.0, (self.side_height - rect.height()).max(0.0));
        let width = rect.width() - 1.0;
        let mut y = rect.top() + 12.0 - self.side_scroll;
        let mut first = true;
        // a row's name has the row but for 14 each side, and a network one's tag with 8 before it
        let tag = spaced(&painter, "net", font(MONO_MEDIUM, 9.0), 0.9, TAG);
        let room = |volume: Volume| width - 28.0 - if volume == Volume::Network { tag.size().x + 8.0 + 8.0 } else { 0.0 };
        let fits = |name: &str, volume| painter.layout_no_wrap(name.to_string(), font(SANS, 12.5), DIM).size().x <= room(volume);
        let recent: Vec<(Place, Volume)> = self.recent.iter().map(|f| (browse::volume_of(f), f)).map(|(volume, f)| (Place { name: recent_name(f, |name| fits(name, volume)), path: f.clone(), group: Group::Recent }, volume)).collect();
        for (group, heading) in [(Group::Known, "PLACES"), (Group::Pinned, "PINNED IN EXPLORER"), (Group::Volumes, "VOLUMES"), (Group::Recent, "RECENT")] {
            let places: Vec<&(Place, Volume)> = self.places.iter().chain(&recent).filter(|(p, _)| p.group == group).collect();
            if places.is_empty() {
                continue;
            }
            if !std::mem::take(&mut first) {
                y += 1.0 + 12.0;
            }
            let heading = spaced(&painter, heading, font(MONO_MEDIUM, 9.5), 1.52, GRAY);
            painter.galley(Pos2::new(rect.left() + 14.0, top_of(&painter, &heading, y + 4.75)), heading, GRAY);
            y += 9.5 + 5.0;
            for (i, (place, volume)) in places.into_iter().enumerate() {
                let row = Rect::from_min_size(Pos2::new(rect.left(), y + 1.0), Vec2::new(width, ROW));
                y = row.bottom();
                let on = place.path == self.nav.path[0].folder;
                if on {
                    painter.rect_filled(row, 0.0, PLACE_ON);
                }
                if *volume == Volume::Network {
                    let size = Vec2::new(tag.size().x + 8.0, 15.0);
                    let mark = Rect::from_min_size(Pos2::new(row.right() - 14.0 - size.x, row.center().y - size.y / 2.0), size);
                    painter.rect_stroke(mark, 0.0, (1.0, TAG_EDGE), StrokeKind::Inside);
                    painter.galley(Pos2::new(mark.center().x - tag.size().x / 2.0, top_of(&painter, &tag, mark.center().y)), tag.clone(), TAG);
                }
                text(&painter, row.left() + 14.0, row.center().y, &place.name, font(SANS, 12.5), if on { FG } else { DIM }, room(*volume));
                if ui.interact(row.intersect(rect), ui.scope_id().with(("brw-place", group as usize, i)), Sense::click()).clicked() {
                    clicks.push((Click::Place(place.path.clone()), None));
                }
            }
        }
        self.side_height = y + self.side_scroll - rect.top() + 12.0;
    }

    /// one folder on the path as a column: its entries, the selected one lit, brightly where the keys act and dimly
    /// on the way there. where the keys act and nothing is selected, the column is outlined.
    fn column(&mut self, ui: &egui::Ui, at: usize, rect: Rect, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        rule(&painter, rect.right() - 1.0, rect);
        let inner = Rect::from_min_max(rect.min, Pos2::new(rect.right() - 1.0, rect.bottom()));
        let current = self.nav.current_column() == Some(at);
        let column = &self.nav.path[at];
        if current && !self.nav.listed(&column.folder).any(|e| Some(&e.path) == column.selected.as_ref()) {
            painter.rect_stroke(inner, 0.0, (1.0, RING), StrokeKind::Inside);
        }
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
        // with the cards alone showing a folder's files, the columns list folders only
        let all = self.nav.view != View::Gallery;
        let entries: Vec<&Entry> = entries.iter().filter(|e| all || e.kind == Kind::Folder).collect();
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
            let on = selected == Some(i) || self.picked.contains(&entry.path);
            if on {
                painter.rect_filled(row, 0.0, if current { ROW_ON } else { PLACE_ON });
            }
            let folder = entry.kind == Kind::Folder;
            let color = match entry.kind {
                Kind::Other => ROW_DIM,
                _ if on && current => Color32::WHITE,
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
            let response = ui.interact(row.intersect(inner), ui.scope_id().with(("brw-row", at, i)), Sense::click());
            if response.secondary_clicked() {
                let on = if folder { On::Folder(entry.path.clone()) } else { On::File(entry.path.clone()) };
                right_click(ui, &self.picked, Some(Click::Row(at, entry.path.clone(), folder)), on, clicks);
            }
            if response.clicked() {
                // with shift or control held, a video in the folder shown is picked with the others
                let (shift, more) = self.held;
                if (shift || more) && is_card(entry) && at + 1 == self.nav.path.len() {
                    clicks.push((Click::Pick(entry.path.clone(), shift, nav::Focus::Columns), None));
                    continue;
                }
                let twice = if folder { Some(Click::Enter) } else { is_card(entry).then(|| Click::Open(entry.path.clone())) };
                clicks.push((Click::Row(at, entry.path.clone(), folder), twice));
            }
        }
    }

    /// the main area: the path line, the note on a network folder, and the grid of cards.
    fn main_area(&mut self, ui: &egui::Ui, rect: Rect, clicks: &mut Vec<Press>) {
        let painter = ui.painter().with_clip_rect(rect);
        let folder = self.nav.main().folder.clone();
        // under the cards and buttons drawn after it: a right click away from them is on the folder shown
        if ui.interact(rect, ui.scope_id().with("brw-main"), Sense::click()).secondary_clicked() {
            right_click(ui, &self.picked, None, On::Shown(folder.clone()), clicks);
        }
        // the path line: what the folder holds. its name is the address's last word, right over this line, and the
        // button that opens it stands at the address's right
        let line = Rect::from_min_size(rect.min, Vec2::new(rect.width(), PATH_LINE));
        painter.rect_filled(Rect::from_min_max(Pos2::new(line.left(), line.bottom() - 1.0), line.max), 0.0, LINE);
        let middle = line.top() + (PATH_LINE - 1.0) / 2.0;
        let holds = match self.nav.listings.get(&folder) {
            None => "reading…".to_string(),
            Some(Err(_)) => String::new(),
            Some(Ok(entries)) => holds(entries),
        };
        text(&painter, line.left() + 14.0, middle, &holds, font(MONO, 11.5), DIM, line.width() - 28.0);

        // on a network volume, a note that nothing is opened without asking, and at its right the button that asks
        // for the folder's previews, saying how many files they read (fig. 29)
        let mut top = line.bottom();
        if self.nav.volume == Volume::Network {
            let asked = self.previews.contains(&folder);
            let reads: usize = self.nav.entries(&folder).iter().filter(|e| nav::makes_pictures(Volume::Network, e, true)).map(nav::files_read).sum();
            let ask = (!asked && reads > 0).then(|| painter.layout_no_wrap(previews_label(reads), font(SANS, 12.0), FG));
            let (head, rest) = network_note(asked, self.asked.iter().any(|f| f.parent() == Some(folder.as_path())));
            let button_size = ask.as_ref().map(|label| Vec2::new(label.size().x + 20.0, 11.5 * 1.5 + 14.0));
            let mut job = LayoutJob::default();
            job.wrap.max_width = rect.width() - 28.0 - 3.0 - 24.0 - button_size.map_or(0.0, |s| s.x + 14.0);
            let format = |family: &str, color| TextFormat { font_id: font(family, 12.0), color, line_height: Some(12.0 * 1.5), ..Default::default() };
            job.append(head, 0.0, format("sans-semibold", FG));
            job.append(&rest, 0.0, format(SANS, DIM));
            let galley = painter.layout_job(job);
            // the text and the button are centered on each other
            let inside = galley.size().y.max(button_size.map_or(0.0, |s| s.y));
            let note = Rect::from_min_size(Pos2::new(rect.left() + 14.0, top + 14.0), Vec2::new(rect.width() - 28.0, inside + 22.0));
            let middle = note.top() + 11.0 + inside / 2.0;
            painter.rect_stroke(note, 0.0, (1.0, LINE), StrokeKind::Inside);
            // fig. 29 draws this edge in amber. amber is kept for a fault in a file, and this is none
            painter.rect_filled(Rect::from_min_size(note.min, Vec2::new(2.0, note.height())), 0.0, GRAY);
            painter.galley(Pos2::new(note.left() + 2.0 + 12.0, middle - galley.size().y / 2.0), galley, DIM);
            if let (Some(label), Some(size)) = (ask, button_size) {
                let at = Rect::from_min_size(Pos2::new(note.right() - 1.0 - 12.0 - size.x, middle - size.y / 2.0), size);
                if button(ui, &painter, at, "brw-previews", label, false) {
                    clicks.push((Click::Previews(folder.clone()), None));
                }
                // outlined in white, as fig. 29 has it
                painter.rect_stroke(at, 0.0, (1.0, FG), StrokeKind::Inside);
            }
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
        let say = |words: &str| text(&painter, rect.left() + 14.0, rect.top() + 14.0 + NAME / 2.0, words, font(SANS, 12.5), DIM, rect.width() - 28.0);
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
        // the selected card is outlined in white where the keys act, in gray on the way there; where the keys act
        // and no card is selected, the grid is outlined
        let current = self.nav.in_grid();
        if current && at.is_none() {
            painter.rect_stroke(rect, 0.0, (1.0, RING), StrokeKind::Inside);
        }
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
        let asked = self.previews.contains(folder);
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
            let outline = match (at == Some(i) || self.picked.contains(&card.path), current) {
                (false, _) => (1.0, LINE),
                (true, true) => (2.0, FG),
                (true, false) => (2.0, CARD_PATH),
            };
            draw_card(&painter, area, thumb, card, outline, pictures, scrub);
            if response.secondary_clicked() {
                right_click(ui, &self.picked, Some(Click::Card(card.path.clone())), On::File(card.path.clone()), clicks);
            }
            if response.clicked() {
                // with shift or control held the card is picked with the others; a plain press selects it alone
                let (shift, more) = self.held;
                clicks.push(if shift || more { (Click::Pick(card.path.clone(), shift, nav::Focus::Grid), None) } else { (Click::Card(card.path.clone()), Some(Click::Open(card.path.clone()))) });
            }
            if !nav::makes_pictures(self.nav.volume, card, asked) {
                continue;
            }
            let want = |all| Want { path: card.path.clone(), size: stamp(card).0, modified: stamp(card).1, all, asked };
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
        let galley = painter.layout(why.to_string(), font(SANS, 12.5), DIM, width);
        let height = galley.size().y;
        painter.galley(Pos2::new(left, y), galley, DIM);
        y += height + 14.0;
        let mut x = left;
        for (i, (place, _)) in self.places.iter().filter(|(p, _)| p.group == Group::Known).enumerate() {
            let label = painter.layout_no_wrap(place.name.clone(), font(SANS, 12.0), FG);
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
        // at the foot: open, and then on a network volume read metadata until it is read
        // (fig. 29), or for a local file that is on the disk reveal (fig. 28). explorer would read a network folder
        // to show it, so a file there is not revealed
        let foot = rect.bottom() - 12.0 - 25.0;
        let mut x = left;
        let third = match (readable && !reads, self.nav.volume != Volume::Network && !card.away) {
            (true, _) => Some(("read metadata", Click::Read(card.path.clone()))),
            (false, true) => Some(("reveal", Click::Reveal(card.path.clone()))),
            (false, false) => None,
        };
        // with several videos picked the one action opens them all, one after another
        let many = self.picked.len() > 1;
        let first = if many { ("open selected", Click::OpenPicked) } else { ("open", Click::Open(card.path.clone())) };
        let acts = [first].into_iter().chain(third.filter(|_| !many));
        for (i, (label, click)) in acts.enumerate() {
            let strong = i == 0;
            let galley = painter.layout_no_wrap(label.to_string(), font(SANS, 12.0), if strong { ON_STRONG } else { FG });
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

    /// what the right-click menu offers for what it is on: what the browser does with that thing, and only what
    /// applies to it here (reveal is not offered on a network volume, where explorer would read the folder to show it).
    fn menu_rows(&self, on: &On) -> Vec<MenuRow> {
        let local = self.nav.volume != Volume::Network;
        let reveal = if cfg!(target_os = "macos") { "reveal in finder" } else { "reveal in explorer" };
        let mut rows: Vec<MenuRow> = Vec::new();
        match on {
            On::File(file) => {
                let Some(entry) = file.parent().and_then(|folder| self.nav.entries(folder).iter().find(|e| &e.path == file)) else { return rows };
                // one of several picked: enter plays them all, and open is this one alone
                let many = self.picked.len() > 1 && self.picked.contains(file);
                if is_card(entry) {
                    rows.push(("open", if many { "" } else { "enter" }, Click::Open(file.clone()), None));
                    rows.push((if self.new_tab { "open in this tab" } else { "open in a new tab" }, "", Click::OpenIn(file.clone(), !self.new_tab), None));
                }
                if many {
                    rows.push(("play the picked videos", "enter", Click::OpenPicked, None));
                }
                if local && !entry.away {
                    rows.push((reveal, "", Click::Reveal(file.clone()), None));
                }
                rows.push(("copy path", "", Click::Copy(file.clone()), None));
                if nav::readable(entry) && !nav::reads_metadata(self.nav.volume, entry, self.asked.contains(file)) {
                    rows.push(("read metadata", "", Click::Read(file.clone()), None));
                }
            }
            On::Folder(folder) | On::Shown(folder) => {
                let shown = matches!(on, On::Shown(_));
                if !shown {
                    rows.push(("open", "right", Click::Enter, None));
                }
                // the folder shown is listed: with no video of its own it has nothing to play but its subfolders'
                if !shown || self.nav.cards().next().is_some() {
                    rows.push(("play folder", "shift enter", Click::PlayFolder(folder.clone()), None));
                }
                rows.push(("play with subfolders", "ctrl shift enter", Click::PlayTree(folder.clone()), None));
                if shown {
                    rows.push(("read again", "f5", Click::Refresh, None));
                }
                if local {
                    rows.push((reveal, "", Click::Reveal(folder.clone()), None));
                }
                rows.push(("copy path", "", Click::Copy(folder.clone()), None));
                if !local && !self.previews.contains(folder) {
                    rows.push(("make previews", "", Click::Previews(folder.clone()), None));
                }
                if shown {
                    rows.extend(VIEWS.map(|(view, words)| (words, "", Click::View(view), Some(view == self.nav.view))));
                }
            }
        }
        rows
    }

    /// the right-click menu (the owner's ask, 2026-11-25), drawn as the player's own: at the pointer where it was
    /// asked for and kept inside `room` by its size when it was last drawn, a row each with its key at the right where
    /// it has one, and the views after a rule. a row chosen does what it says; that, a press outside it and esc put it
    /// away.
    fn menu(&mut self, ui: &egui::Ui, room: Rect, clicks: &mut Vec<Press>) {
        let Some((on, corner)) = self.menu.clone() else { return };
        if ui.input(|i| i.pointer.any_pressed()) && !ui.ctx().pointer_latest_pos().is_some_and(|p| self.menu_rect.contains(p)) {
            self.menu = None;
            return;
        }
        // what it was on is gone from its folder, read again
        let rows = self.menu_rows(&on);
        if rows.is_empty() {
            self.menu = None;
            return;
        }
        let last = self.menu_rect;
        let at = Pos2::new(corner.x.min(room.right() - last.width()).max(room.left()), corner.y.min(room.bottom() - last.height()).max(room.top()));
        let mut chosen = None;
        let drawn = widgets::menu(ui, "brw-menu", at, Align2::LEFT_TOP, MENU_WIDTH, 1.0, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut ruled = false;
            for (i, (words, key, click, view)) in rows.into_iter().enumerate() {
                if view.is_some() && !std::mem::replace(&mut ruled, true) {
                    let rule = widgets::row(ui, 9.0);
                    ui.painter().rect_filled(Rect::from_min_size(Pos2::new(rule.left(), rule.center().y), Vec2::new(rule.width(), 1.0)), 0.0, LINE);
                }
                let row = widgets::row(ui, MENU_ROW);
                let response = ui.interact(row, ui.scope_id().with(("brw-menu-row", i)), Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(row, 3.0, HOVER);
                }
                let family = if view == Some(true) { "sans-semibold" } else { SANS };
                let galley = ui.painter().layout_no_wrap(words.to_string(), font(family, 12.0), FG);
                ui.painter().galley(Pos2::new(row.left() + 6.0, text_top(ui.painter(), font(family, 12.0), row.center().y)), galley, FG);
                if !key.is_empty() {
                    widgets::key_cap(ui, Pos2::new(row.right() - 6.0, row.center().y), key, false, DIM);
                }
                if response.clicked() {
                    chosen = Some(click);
                }
            }
        });
        // a menu of another size than the last is kept inside the room from the next pass
        if drawn.size() != last.size() {
            ui.ctx().request_repaint();
        }
        self.menu_rect = drawn;
        if let Some(click) = chosen {
            self.menu = None;
            clicks.push((click, None));
        }
    }
}

/// a right click on something the menu can be on: a row or a card is selected first, as a press selects it, unless it
/// is one of several videos picked together, which stay picked; then the menu opens at the pointer.
fn right_click(ui: &egui::Ui, picked: &[PathBuf], select: Option<Click>, on: On, clicks: &mut Vec<Press>) {
    let kept = matches!(&on, On::File(file) if picked.len() > 1 && picked.contains(file));
    clicks.extend(select.filter(|_| !kept).map(|click| (click, None)));
    if let Some(at) = ui.ctx().pointer_latest_pos() {
        clicks.push((Click::Menu(on, at), None));
    }
}

/// how long the selection has still to rest on `file` before it is read: `REST` from when it got there.
/// the folders from the top of a drive or share down to `folder`, each with the name the address shows for it.
fn crumbs(folder: &Path) -> Vec<(String, PathBuf)> {
    let mut down: Vec<&Path> = folder.ancestors().filter(|above| !above.as_os_str().is_empty()).collect();
    down.reverse();
    down.into_iter().map(|at| (browse::name_of(at), at.to_path_buf())).collect()
}

/// what was typed into the address: a path, as typed or as the system's file browser copies one, in quotes. none
/// when nothing was.
fn typed(text: &str) -> Option<PathBuf> {
    let path = text.trim().trim_matches('"').trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

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
/// card), with `outline` round it; a sequence's marks at the top right; then the name and what the listing says of
/// the file. while the pointer scrubs it (`scrub`, 0 to 1 across), a line marks the place and the frame shown is named.
fn draw_card(painter: &Painter, area: Rect, thumb: f32, entry: &Entry, outline: (f32, Color32), pictures: Option<&Card>, scrub: Option<f32>) {
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
    painter.rect_stroke(picture, 0.0, outline, StrokeKind::Outside);
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

/// what the note on a network folder says, in bold and then plain: whether its previews were asked for, and whether
/// a file of it was opened or read
fn network_note(previews: bool, opened: bool) -> (&'static str, String) {
    let (head, rest) = match (previews, opened) {
        (true, _) => ("network folder: previews asked for.", "posters are read from the files here, and a card's previews when the pointer rests on it"),
        (false, true) => ("network folder: look-only.", "only the files you asked for have been opened"),
        (false, false) => ("network folder: look-only.", "nothing here has been opened"),
    };
    (head, format!(" {rest}. names, sizes and dates come from the folder listing."))
}

/// the button that asks for a network folder's previews, with how many files they read
fn previews_label(reads: usize) -> String {
    format!("make previews · reads {reads} file{}", if reads == 1 { "" } else { "s" })
}

/// a recent folder as the sidebar names it: the folder above it, then its own name ("delivery_2026-11-20 / wall").
/// where that does not `fit`, the folder above is cut short first, so the folder's own name stays.
fn recent_name(folder: &Path, fits: impl Fn(&str) -> bool) -> String {
    let own = browse::name_of(folder);
    let Some(up) = folder.parent().map(browse::name_of) else { return own };
    let up: Vec<char> = up.chars().collect();
    let cut = |keep: usize| if keep == up.len() { format!("{} / {own}", String::from_iter(&up)) } else { format!("{}… / {own}", String::from_iter(&up[..keep])) };
    (0..=up.len()).rev().map(cut).find(|name| fits(name)).unwrap_or_else(|| cut(0))
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
    // names and values mono; what the file does not say, and a fault, are sentences, in archivo
    let format = |color| TextFormat { font_id: font(MONO, 11.5), color, line_height: Some(META_LINE), ..Default::default() };
    let words = |color| TextFormat { font_id: font(SANS, 12.0), color, line_height: Some(META_LINE), ..Default::default() };
    for (g, group) in groups.iter().enumerate() {
        if g > 0 {
            painter.rect_filled(Rect::from_min_size(Pos2::new(left, y + 2.0), Vec2::new(width, 1.0)), 0.0, LINE);
            y += 2.0 + 1.0 + 2.0 + 5.0;
        }
        // not the file's path: the address says where it is, and the row over this one its name
        for row in group.iter().filter(|row| row.label != "path") {
            let mut label = LayoutJob::default();
            label.wrap.max_width = 86.0;
            label.append(row.label, 0.0, format(DIM));
            let mut value = LayoutJob::default();
            value.wrap.max_width = width - 96.0;
            match row.tone {
                Tone::Fault => value.append(&format!("! {}", row.value), 0.0, words(AMBER)),
                Tone::Assumed => value.append(&row.value, 0.0, words(DIM)),
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

    /// the address names every folder from the top of the drive or share down to the one shown, and each goes to
    /// its own folder. what is typed into it is a path as typed, or as the system's file browser copies one, in
    /// quotes; nothing typed is nowhere to go.
    #[test]
    fn the_address_is_every_folder_from_the_top_down() {
        let top = if cfg!(windows) { r"C:\" } else { "/" };
        let folder = Path::new(top).join("delivery").join("wall");
        let named: Vec<(String, PathBuf)> = crumbs(&folder);
        assert_eq!(named.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), [if cfg!(windows) { "C:" } else { "/" }, "delivery", "wall"]);
        assert_eq!(named.iter().map(|(_, to)| to.clone()).collect::<Vec<_>>(), [PathBuf::from(top), Path::new(top).join("delivery"), folder.clone()]);
        assert_eq!(typed("  \"D:\\renders out\\wall\"  "), Some(PathBuf::from(r"D:\renders out\wall")));
        assert_eq!(typed("shots"), Some(PathBuf::from("shots")));
        assert_eq!((typed(""), typed("  \"\" ")), (None, None));
        #[cfg(windows)]
        assert_eq!(crumbs(Path::new(r"\\nas\renders\wall")).iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), [r"\\nas\renders", "wall"]);
    }

    #[test]
    fn a_card_says_what_the_listing_says() {
        let at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(when(at), "2023-11-14 22:13 utc");
        assert_eq!(when(UNIX_EPOCH + Duration::from_secs(951_782_400)), "2000-02-29 00:00 utc");
        assert_eq!((short_size(4_820_000_000), short_size(86_000_000), short_size(512)), ("4.82 GB".into(), "86.0 MB".into(), "512 bytes".into()));
        let clip = Entry { name: "a.mov".into(), path: "a.mov".into(), kind: Kind::Clip, size: Some(4_820_000_000), modified: Some(at), away: false, link: false };
        assert_eq!(spec(&clip), ("4.82 GB · 2023-11-14 22:13 utc".to_string(), false));
        let seq = Entry { name: "s.[0001-0240].exr".into(), path: "s.0001.exr".into(), kind: Kind::Sequence { first: 1, last: 240, missing: vec![7, 9], pad: 4 }, size: None, modified: None, away: false, link: false };
        assert_eq!(spec(&seq), ("238 of 240".to_string(), true));
        assert_eq!(listed_facts(&seq)[1].value, "238 of 240 · 2 missing");
        assert_eq!(holds(&[clip, seq]), "1 video · 1 sequence");
        assert_eq!(holds(&[]), "empty");
    }

    #[test]
    fn the_columns_give_way_to_the_main_area_and_the_metadata_column() {
        let area = |w: f32| Rect::from_min_size(Pos2::ZERO, Vec2::new(w, 668.0));
        // fig. 28's width with a card selected: two columns, the main area, the metadata column
        let layout = Layout::new(area(1240.0), 5, true, View::Both, Widths::default());
        assert_eq!((layout.first, layout.columns.len(), layout.meta.is_some()), (3, 2, true));
        assert_eq!((layout.main.left(), layout.main.width()), { let w = Widths::default(); (w.side + 2.0 * browse::COLUMN, 1240.0 - w.side - 2.0 * browse::COLUMN - w.meta) });
        // with nothing selected there is no metadata column, and more columns fit
        assert_eq!(Layout::new(area(1240.0), 5, false, View::Both, Widths::default()).columns.len(), 4);
        // with the column alone showing, the cards keep no room: the columns have it
        assert_eq!(Layout::new(area(1240.0), 9, false, View::List, Widths::default()).columns.len(), 6);
        // wider columns, fewer of them; a wider sidebar moves all after it
        let wide = Layout::new(area(1240.0), 5, false, View::Both, Widths { side: 200.0, columns: [240.0; browse::COLUMNS], meta: 316.0 });
        assert_eq!((wide.columns.len(), wide.columns[0].left(), wide.main.left()), (2, 200.0, 680.0));
        // each column at its own width: the fourth of five 300 wide takes one column's room, so three fit, not four
        let mut widths = Widths::default();
        widths.set_column(3, 300.0);
        let own = Layout::new(area(1240.0), 5, false, View::Both, widths);
        assert_eq!((own.first, own.columns.iter().map(|c| c.width()).collect::<Vec<_>>()), (2, vec![browse::COLUMN, 300.0, browse::COLUMN]));
        // narrow: one column, the deepest, and no metadata column
        let layout = Layout::new(area(700.0), 5, true, View::Both, Widths::default());
        assert_eq!((layout.first, layout.columns.len(), layout.meta.is_none()), (4, 1, true));
        // never more columns than the path has
        assert_eq!(Layout::new(area(2400.0), 1, false, View::Both, Widths::default()).columns.len(), 1);
    }

    /// a sign the fonts do not have is drawn as an empty box
    #[test]
    fn the_sidebar_the_note_and_the_actions_are_all_in_their_fonts() {
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let has = |family: &str, words: &str| ui.ctx().fonts_mut(|f| f.has_glyphs(&font(family, 11.5), words));
            assert!(["PLACES", "PINNED IN EXPLORER", "VOLUMES", "RECENT", "net"].iter().all(|w| has(MONO_MEDIUM, w)));
            assert!(["open", "reveal", "read metadata", "open selected", "open folder", "list + gallery"].iter().all(|w| has(SANS, w)));
            assert!(has(SANS, &recent_name(Path::new(r"D:\delivery_2026-11-20\wall"), |n| n.len() < 12)) && has(SANS, "system (C:)"));
            for (previews, opened) in [(false, false), (false, true), (true, false)] {
                let (head, rest) = network_note(previews, opened);
                assert!(has("sans-semibold", head) && has(SANS, &rest), "{head}{rest}");
            }
            assert!(has(SANS, &previews_label(300)) && has(SANS, &previews_label(1)));
        });
        out.textures_delta.clear();
    }

    #[cfg(windows)]
    #[test]
    fn pinned_folders_join_after_the_places_and_recent_ones_come_newest_first() {
        let mut state = State::new(Some(PathBuf::from(r"C:\no\such")), Vec::new(), None);
        // fig. 27's sidebar, in place of this pc's own
        let place = |name: &str, path: &str, group| (Place { name: name.into(), path: path.into(), group }, browse::volume_of(Path::new(path)));
        state.places = vec![place("downloads", r"C:\u\Downloads", Group::Known), place("desktop", r"C:\u\Desktop", Group::Known), place("system (C:)", r"C:\", Group::Volumes)];
        state.roots = state.places.iter().map(|(p, _)| p.path.clone()).collect();
        // explorer pins downloads itself, which is a place already
        let pin = |path: &str| Place { name: browse::name_of(Path::new(path)), path: path.into(), group: Group::Pinned };
        state.pin(vec![pin(r"C:\u\Downloads"), pin(r"C:\show_content"), pin(r"\\nas\projects\renders_out")]);
        let names: Vec<&str> = state.places.iter().map(|(p, _)| p.name.as_str()).collect();
        assert_eq!(names, ["downloads", "desktop", "show_content", "renders_out", "system (C:)"]);
        assert_eq!((state.places[2].1, state.places[3].1), (Volume::Local, Volume::Network));
        assert!(state.roots.contains(&PathBuf::from(r"C:\show_content")));

        // the folder a file is opened or played from comes first; a place is none, and one already there moves up
        let wall = PathBuf::from(r"C:\delivery_2026-11-20\wall");
        for file in [wall.join("a.mov"), PathBuf::from(r"C:\u\Downloads\b.mov"), PathBuf::from(r"C:\show_content\totems\c.mov")] {
            state.choose(Chosen::Open(file));
        }
        state.choose(Chosen::Open(wall.join("d.mov")));
        assert_eq!(state.recent, [wall.clone(), PathBuf::from(r"C:\show_content\totems")]);
        // named by the folder above and its own; where that does not fit, the folder above is cut short first
        assert_eq!(recent_name(&state.recent[0], |_| true), "delivery_2026-11-20 / wall");
        let fits = |most: usize| move |name: &str| name.chars().count() <= most;
        assert_eq!((recent_name(&wall, fits(16)), recent_name(&wall, fits(4)), recent_name(Path::new(r"C:\x"), fits(9))), ("delivery… / wall".to_string(), "… / wall".to_string(), "C: / x".to_string()));
        for n in 0..RECENT {
            state.choose(Chosen::PlayFolder(PathBuf::from(format!(r"C:\played\{n}"))));
        }
        assert_eq!((state.recent.len(), state.recent[0].clone()), (RECENT, PathBuf::from(format!(r"C:\played\{}", RECENT - 1))));
    }

    /// a folder tree under the temp folder that removes what it made one item at a time, the files first, never
    /// recursively
    #[cfg(windows)]
    struct Scratch(Vec<(PathBuf, bool)>);

    #[cfg(windows)]
    impl Scratch {
        fn folder(&mut self, path: PathBuf) -> PathBuf {
            std::fs::create_dir(&path).unwrap();
            self.0.push((path.clone(), true));
            path
        }

        fn file(&mut self, path: PathBuf) {
            std::fs::write(&path, b"no clip: nothing may read it").unwrap();
            self.0.push((path, false));
        }
    }

    #[cfg(windows)]
    impl Drop for Scratch {
        fn drop(&mut self) {
            for (path, folder) in self.0.iter().rev() {
                let removed = if *folder { std::fs::remove_dir(path) } else { std::fs::remove_file(path) };
                if let Err(why) = removed {
                    eprintln!("left behind {}: {why}", path.display());
                }
            }
        }
    }

    /// the same folder through this pc's own share, which stands for a network volume
    #[cfg(windows)]
    fn shared(local: &Path) -> PathBuf {
        let local = local.to_string_lossy();
        PathBuf::from(format!(r"\\localhost\{}${}", &local[..1], &local[2..]))
    }

    /// one pass of the browser over fig. 28's window, at `time`, with the pointer at `at` or out of the window
    #[cfg(windows)]
    fn pass(ctx: &egui::Context, state: &mut State, time: f64, at: Option<Pos2>) {
        let mut raw = egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1240.0, 700.0))), ..Default::default() };
        raw.events.push(at.map_or(egui::Event::PointerGone, egui::Event::PointerMoved));
        let mut out = ctx.run_ui(raw, |ui| {
            state.draw(ui, ui.max_rect());
        });
        out.textures_delta.clear();
    }

    /// draws until the folder is listed
    #[cfg(windows)]
    fn listed(ctx: &egui::Context, state: &mut State, folder: &Path, time: &mut f64) {
        for _ in 0..200 {
            if state.nav.listings.contains_key(folder) {
                return;
            }
            pass(ctx, state, *time, None);
            *time += 0.05;
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("the folder was not listed");
    }

    /// what is typed into the address is gone to: a folder's name alone goes into that folder, a path to a video is
    /// asked to be opened and the browser stays where it is, and a folder that is not there says so in the main
    /// area, where the places are.
    #[cfg(windows)]
    #[test]
    fn what_is_typed_into_the_address_is_gone_to_and_a_folder_not_there_says_so() {
        let mut made = Scratch(Vec::new());
        let top = made.folder(std::env::temp_dir().join(format!("eyepiece-address-{}", std::process::id())));
        let shots = made.folder(top.join("shots"));
        let mut state = State::new(Some(top.clone()), Vec::new(), None);
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut time = 0.0;
        listed(&ctx, &mut state, &top, &mut time);
        state.click(Click::Go(PathBuf::from("shots")));
        assert_eq!(state.nav.main().folder, shots);
        state.click(Click::Go(top.join("wall_v012.mov")));
        assert!(matches!(&state.chosen, Some(Chosen::Open(file)) if *file == top.join("wall_v012.mov")));
        assert_eq!(state.nav.main().folder, shots);
        let gone = top.join("nothere");
        state.click(Click::Go(gone.clone()));
        listed(&ctx, &mut state, &gone, &mut time);
        let raw = egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1240.0, 700.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            state.draw(ui, ui.max_rect());
        });
        out.textures_delta.clear();
        let said = |words: &str| out.shapes.iter().any(|drawn| matches!(&drawn.shape, egui::Shape::Text(text) if text.galley.text() == words));
        assert!(said("this folder cannot be read") && said("nothere"), "the main area says so, under an address that ends in what was typed");
        // and as a field the address holds that path, to put right
        state.click(Click::Address);
        assert_eq!(state.address, Some(gone.to_string_lossy().into_owned()));
    }

    /// an edge dragged resizes its part, each folder column alone, so the edge stays under the pointer; a double click
    /// on an edge puts it back. the widths keep within what keeps every part usable.
    #[test]
    fn the_parts_are_resized_by_their_edges() {
        let folder = std::env::temp_dir().join("eyepiece-no-such-edges");
        let mut state = State::new(Some(folder.clone()), Vec::new(), None);
        state.nav.listings.insert(folder.clone(), Ok(Vec::new()));
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut time = 0.0;
        let mut pass_with = |state: &mut State, events: Vec<egui::Event>| {
            let mut raw = egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1240.0, 700.0))), ..Default::default() };
            raw.events = events;
            let mut out = ctx.run_ui(raw, |ui| {
                state.draw(ui, ui.max_rect());
            });
            out.textures_delta.clear();
            time += 0.05;
        };
        let press = |x: f32, down: bool| egui::Event::PointerButton { pos: Pos2::new(x, 300.0), button: egui::PointerButton::Primary, pressed: down, modifiers: egui::Modifiers::NONE };
        let drag = |state: &mut State, pass_with: &mut dyn FnMut(&mut State, Vec<egui::Event>), from: f32, to: f32| {
            pass_with(state, vec![egui::Event::PointerMoved(Pos2::new(from, 300.0))]);
            pass_with(state, vec![press(from, true)]);
            for step in 1..=4 {
                pass_with(state, vec![egui::Event::PointerMoved(Pos2::new(from + (to - from) * step as f32 / 4.0, 300.0))]);
            }
            pass_with(state, vec![press(to, false)]);
        };
        drag(&mut state, &mut pass_with, 150.0, 210.0);
        assert_eq!(state.widths.side, 210.0);
        // the second column's right edge, at 210 + 2 x 168, dragged 40 right: that column alone is 40 wider
        let columns = state.nav.drawn();
        if columns >= 2 {
            drag(&mut state, &mut pass_with, 210.0 + 2.0 * 168.0, 210.0 + 2.0 * 168.0 + 40.0);
            let changed: Vec<f32> = state.widths.columns.iter().copied().filter(|&w| w != browse::COLUMN).collect();
            assert_eq!(changed, vec![208.0], "the second column alone is wider; the first keeps its width");
        }
        drag(&mut state, &mut pass_with, 210.0, 20.0);
        assert_eq!(state.widths.side, 110.0, "no narrower than a row's name needs");
        for _ in 0..2 {
            pass_with(&mut state, vec![egui::Event::PointerMoved(Pos2::new(110.0, 300.0)), press(110.0, true), press(110.0, false)]);
        }
        assert_eq!(state.widths.side, 150.0, "a double click puts it back");
    }

    /// what is selected where the keys act is drawn brightly, and what is selected on the way there dimly, in the
    /// columns and in the grid; where the keys act and nothing is selected, the column is outlined. a press on a row
    /// gives the keys to its column, and a press on a card to the grid.
    #[test]
    fn the_current_item_and_the_way_to_it_are_drawn_apart() {
        let place = std::env::temp_dir().join("eyepiece-no-such-marks");
        let (r, shots) = (place.join("r"), place.join("r").join("shots"));
        let mut state = State::new(Some(r.clone()), Vec::new(), None);
        state.nav = Nav::new(&r, std::slice::from_ref(&place));
        let entry = |dir: &Path, name: &str, kind| Entry { name: name.into(), path: dir.join(name), kind, size: Some(10), modified: None, away: false };
        let listings = || [(place.clone(), vec![entry(&place, "r", Kind::Folder)]), (r.clone(), vec![entry(&r, "shots", Kind::Folder), entry(&r, "a.mov", Kind::Clip)]), (shots.clone(), vec![entry(&shots, "c.mov", Kind::Clip), entry(&shots, "d.mov", Kind::Clip)])];
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        // a pass over fig. 28's window, the folders listed as above: the fill of the row each name is on, the
        // outlines of the selected cards, and whether a column or the grid is outlined
        let seen = |state: &mut State| {
            for (folder, entries) in listings() {
                state.nav.listings.insert(folder, Ok(entries));
            }
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1240.0, 700.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                state.draw(ui, ui.max_rect());
            });
            out.textures_delta.clear();
            let rects: Vec<&egui::epaint::RectShape> = out.shapes.iter().filter_map(|s| if let egui::Shape::Rect(r) = &s.shape { Some(r) } else { None }).collect();
            let fill = |name: &str| {
                let texts = out.shapes.iter().filter_map(|s| if let egui::Shape::Text(t) = &s.shape { Some(t) } else { None });
                texts.filter(|t| t.galley.text() == name).find_map(|t| rects.iter().find(|r| r.rect.height() == ROW && r.rect.contains(t.pos + Vec2::new(1.0, t.galley.size().y / 2.0))).map(|r| r.fill))
            };
            let fills = ["r", "shots", "a.mov", "c.mov", "d.mov"].map(fill);
            let cards: Vec<Color32> = rects.iter().filter(|r| r.stroke.width == 2.0).map(|r| r.stroke.color).collect();
            (fills, cards, rects.iter().any(|r| r.stroke.color == RING))
        };
        let press = |state: &mut State, key| {
            state.nav.key(key, Mods::default(), 2);
            state.nav.settle();
        };
        let (on, path) = (Some(ROW_ON), Some(PLACE_ON));
        // at the start the keys are in the folder's column, where nothing is selected
        assert_eq!(seen(&mut state), ([path, None, None, None, None], vec![], true));
        press(&mut state, Key::Down);
        assert_eq!(seen(&mut state), ([path, on, None, None, None], vec![], false));
        press(&mut state, Key::Right);
        assert_eq!(seen(&mut state), ([None, path, None, on, None], vec![CARD_PATH], false), "the place's column has given way");
        press(&mut state, Key::Right);
        assert_eq!(seen(&mut state), ([None, path, None, path, None], vec![FG], false));
        state.click(Click::Row(2, shots.join("d.mov"), false));
        assert_eq!(seen(&mut state), ([None, path, None, None, on], vec![CARD_PATH], false));
        state.click(Click::Card(shots.join("c.mov")));
        assert_eq!(seen(&mut state), ([None, path, None, path, None], vec![FG], false));
        state.click(Click::Pick(shots.join("d.mov"), false, nav::Focus::Columns));
        assert_eq!(seen(&mut state), ([None, path, None, on, on], vec![CARD_PATH, CARD_PATH], false));
    }

    /// shift picks a run of videos from the one last pressed, control one more or one less, and the videos picked
    /// open together in the order the folder lists them. a plain press lets them go.
    #[test]
    fn shift_and_control_presses_pick_several_videos_to_open_together() {
        let folder = std::env::temp_dir().join("eyepiece-no-such-wall");
        let mut state = State::new(Some(folder.clone()), Vec::new(), None);
        let clip = |name: &str| Entry { name: name.into(), path: folder.join(name), kind: Kind::Clip, size: Some(10), modified: None, away: false, link: false };
        state.nav.listings.insert(folder.clone(), Ok(["a.mov", "b.mov", "c.mov", "d.mov"].map(clip).into()));
        let at = |name: &str| folder.join(name);
        let grid = nav::Focus::Grid;
        state.click(Click::Card(at("b.mov")));
        state.click(Click::Pick(at("d.mov"), true, grid));
        assert_eq!(state.picked, [at("b.mov"), at("c.mov"), at("d.mov")]);
        assert_eq!(state.nav.card().map(|card| card.path.clone()), Some(at("d.mov")));
        state.click(Click::Pick(at("c.mov"), false, grid));
        assert_eq!(state.picked, [at("b.mov"), at("d.mov")]);
        state.click(Click::Pick(at("a.mov"), false, grid));
        state.click(Click::OpenPicked);
        assert!(matches!(&state.chosen, Some(Chosen::PlayFiles(files)) if *files == [at("a.mov"), at("b.mov"), at("d.mov")]));
        state.click(Click::Card(at("c.mov")));
        assert!(state.picked.is_empty());
    }

    fn listed_entry(folder: &Path, name: &str, kind: Kind) -> Entry {
        Entry { name: name.into(), path: folder.join(name), kind, size: Some(10), modified: None, away: false, link: false }
    }

    /// the right-click menu offers what the browser does with what it is on, and only what applies there: a video, a
    /// file that is none, a folder's row, the folder shown; several videos picked; the settings' other tab; a network
    /// volume, where nothing is revealed and metadata and previews are asked for. every row fits the menu in its fonts.
    #[test]
    fn the_right_click_menu_offers_what_applies_to_what_it_is_on() {
        let reveal = if cfg!(target_os = "macos") { "reveal in finder" } else { "reveal in explorer" };
        let rows = |state: &State, on: On| state.menu_rows(&on).into_iter().map(|(words, key, _, view)| (words, key, view)).collect::<Vec<_>>();
        let folder = std::env::temp_dir().join("eyepiece-no-such-menu");
        let at = |name: &str| folder.join(name);
        let mut state = State::new(Some(folder.clone()), Vec::new(), None);
        let entries = vec![listed_entry(&folder, "dome", Kind::Folder), listed_entry(&folder, "a.mov", Kind::Clip), listed_entry(&folder, "b.mov", Kind::Clip), listed_entry(&folder, "notes.txt", Kind::Other)];
        state.nav.listings.insert(folder.clone(), Ok(entries));
        assert_eq!(rows(&state, On::File(at("a.mov"))), [("open", "enter", None), ("open in a new tab", "", None), (reveal, "", None), ("copy path", "", None)]);
        state.new_tab = true;
        assert_eq!(rows(&state, On::File(at("a.mov")))[1].0, "open in this tab");
        state.picked = vec![at("a.mov"), at("b.mov")];
        assert_eq!(rows(&state, On::File(at("a.mov"))), [("open", "", None), ("open in this tab", "", None), ("play the picked videos", "enter", None), (reveal, "", None), ("copy path", "", None)]);
        state.picked.clear();
        assert_eq!(rows(&state, On::File(at("notes.txt"))), [(reveal, "", None), ("copy path", "", None)]);
        let dome = [("open", "right", None), ("play folder", "shift enter", None), ("play with subfolders", "ctrl shift enter", None), (reveal, "", None), ("copy path", "", None)];
        assert_eq!(rows(&state, On::Folder(at("dome"))), dome);
        assert!(matches!(&state.menu_rows(&On::Folder(at("dome")))[2].2, Click::PlayTree(to) if *to == at("dome")));
        let views = [("list + gallery", "", Some(true)), ("gallery", "", Some(false)), ("list", "", Some(false))];
        let shown = [("play folder", "shift enter", None), ("play with subfolders", "ctrl shift enter", None), ("read again", "f5", None), (reveal, "", None), ("copy path", "", None)];
        assert_eq!(rows(&state, On::Shown(folder.clone())), [&shown[..], &views[..]].concat());

        let share = PathBuf::from(r"\\server\share\shots");
        let mut state = State::new(Some(share.clone()), Vec::new(), None);
        assert_eq!(state.nav.volume, Volume::Network);
        state.nav.listings.insert(share.clone(), Ok(vec![listed_entry(&share, "sub", Kind::Folder), listed_entry(&share, "a.mov", Kind::Clip)]));
        assert_eq!(rows(&state, On::File(share.join("a.mov"))), [("open", "enter", None), ("open in a new tab", "", None), ("copy path", "", None), ("read metadata", "", None)]);
        state.asked.insert(share.join("a.mov"));
        assert_eq!(rows(&state, On::File(share.join("a.mov"))).len(), 3, "read, it is not offered again");
        assert_eq!(rows(&state, On::Folder(share.join("sub"))).last(), Some(&("make previews", "", None)));
        let shown = [("play folder", "shift enter", None), ("play with subfolders", "ctrl shift enter", None), ("read again", "f5", None), ("copy path", "", None), ("make previews", "", None)];
        assert_eq!(rows(&state, On::Shown(share.clone())), [&shown[..], &views[..]].concat());
        state.previews.insert(share.clone());
        assert!(!rows(&state, On::Shown(share.clone())).iter().any(|(words, ..)| *words == "make previews"));
        // the folder shown with no video of its own has only its subfolders' to play
        state.nav.listings.insert(share.clone(), Ok(vec![listed_entry(&share, "sub", Kind::Folder)]));
        assert_eq!(rows(&state, On::Shown(share.clone()))[0].0, "play with subfolders");

        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for (words, key, view) in [&dome[..], &shown[..], &views[..], &[("play the picked videos", "enter", None), ("open in a new tab", "", None), ("open in this tab", "", None), ("read metadata", "", None)]].concat() {
                let family = if view == Some(true) { "sans-semibold" } else { SANS };
                assert!(ui.ctx().fonts_mut(|f| f.has_glyphs(&font(family, 12.0), words) && f.has_glyphs(&font(MONO_MEDIUM, 10.0), key)), "{words} {key}");
                let wide = ui.painter().layout_no_wrap(words.to_string(), font(family, 12.0), FG).size().x;
                let cap = if key.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(key.to_string(), font(MONO_MEDIUM, 10.0), DIM).size().x + 10.0 };
                // the menu's border and padding are 26, the row's insets 6 each side, and 12 at least between
                assert!(6.0 + wide + 12.0 + cap + 6.0 <= MENU_WIDTH - 26.0, "{words} with {key} does not fit");
            }
        });
        out.textures_delta.clear();
    }

    /// driven as a person does: a right click on a row selects it and opens the menu at the pointer, and a row of the
    /// menu does what it says (copy path puts the path on the clipboard, as a field's copy does). a right click on the
    /// main area away from the cards is on the folder shown and selects nothing; esc puts the menu away and does no
    /// more, as does a press outside it. control + shift + enter walks the folder shown.
    #[test]
    fn a_right_click_selects_what_it_is_on_and_opens_the_menu_there() {
        let folder = std::env::temp_dir().join("eyepiece-no-such-right");
        let mut browser = Browser::new(Some(folder.clone()), None);
        browser.state().nav.listings.insert(folder.clone(), Ok(vec![listed_entry(&folder, "dome", Kind::Folder), listed_entry(&folder, "a.mov", Kind::Clip)]));
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(1240.0, 700.0));
        let mut time = 0.0;
        let mut pass = |browser: &mut Browser, events: Vec<egui::Event>| {
            let mut raw = egui::RawInput { time: Some(time), screen_rect: Some(area), ..Default::default() };
            raw.events = events;
            let mut out = ctx.run_ui(raw, |ui| {
                browser.draw(ui, ui.max_rect(), false);
            });
            out.textures_delta.clear();
            time += 0.05;
            out.platform_output.commands
        };
        // the pointer goes there, and the button goes down and comes up, a pass each; then a pass for what it opened
        let press = |browser: &mut Browser, pass: &mut dyn FnMut(&mut Browser, Vec<egui::Event>) -> Vec<egui::OutputCommand>, at: Pos2, button: egui::PointerButton| {
            let event = |pressed| egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE };
            pass(browser, vec![egui::Event::PointerMoved(at)]);
            pass(browser, vec![event(true)]);
            let said = pass(browser, vec![event(false)]);
            pass(browser, Vec::new());
            pass(browser, Vec::new());
            said
        };
        let layout = |browser: &mut Browser| {
            let state = browser.state();
            Layout::new(area, state.nav.drawn(), state.nav.card().is_some(), state.nav.view, state.widths)
        };
        // the folder shown is the last column: its rows from 8 under the path line
        let row = |layout: &Layout, i: usize| Pos2::new(layout.columns.last().unwrap().center().x, PATH_LINE + 8.0 + ROW * (i as f32 + 0.5));
        let (dome, clip) = (folder.join("dome"), folder.join("a.mov"));
        let at = row(&layout(&mut browser), 1);
        press(&mut browser, &mut pass, at, egui::PointerButton::Secondary);
        assert_eq!(browser.state().nav.card().map(|card| card.path.clone()), Some(clip.clone()), "the row is selected");
        assert_eq!(browser.state().menu, Some((On::File(clip.clone()), at)));
        // open, open in a new tab, reveal, copy path: the fourth row, 13 into the menu and 24 a row
        let said = press(&mut browser, &mut pass, at + Vec2::new(60.0, 13.0 + 24.0 * 3.5), egui::PointerButton::Primary);
        assert!(said.contains(&egui::OutputCommand::CopyText(clip.to_string_lossy().into_owned())), "copy path copied {said:?}");
        assert_eq!(browser.state().menu, None);

        let main = layout(&mut browser).main;
        let away = Pos2::new(main.center().x, main.bottom() - 30.0);
        press(&mut browser, &mut pass, away, egui::PointerButton::Secondary);
        assert_eq!(browser.state().menu, Some((On::Shown(folder.clone()), away)));
        assert_eq!(browser.state().nav.card().map(|card| card.path.clone()), Some(clip.clone()), "the selection stays");
        browser.key(Key::Escape, Mods::default());
        assert!(browser.state().menu.is_none() && browser.state().chosen.is_none(), "esc puts the menu away and goes nowhere");

        let at = row(&layout(&mut browser), 0);
        press(&mut browser, &mut pass, at, egui::PointerButton::Secondary);
        assert_eq!((browser.state().nav.main().folder.clone(), browser.state().menu.clone()), (dome.clone(), Some((On::Folder(dome.clone()), at))));
        // the main area shows the folder now, where the columns have moved to give it its own
        let main = layout(&mut browser).main;
        press(&mut browser, &mut pass, Pos2::new(main.center().x, main.bottom() - 30.0), egui::PointerButton::Primary);
        assert_eq!(browser.state().menu, None, "a press outside puts it away");
        browser.key(Key::Enter, Mods { shift: true, ctrl: true, alt: false });
        assert!(browser.state().walks.waiting(&dome), "control + shift + enter walks the folder shown");
    }

    /// the player asks before it plays a network folder. asked for again while that stands, the folder plays the walk
    /// that was counted, with no second walk, which on a slow share could outlast the ask and so ask for ever. later
    /// than that, it is walked again.
    #[cfg(windows)]
    #[test]
    fn a_network_folder_asked_for_again_plays_the_walk_it_was_asked_about() {
        let mut made = Scratch(Vec::new());
        let top = made.folder(std::env::temp_dir().join(format!("eyepiece-asked-{}", std::process::id())));
        made.file(top.join("a.mov"));
        let share = shared(&top);
        let mut state = State::new(Some(share.clone()), Vec::new(), None);
        let walked = |state: &mut State| {
            for _ in 0..200 {
                state.take_answers();
                if let Some(Chosen::PlayTree(_, Ok(clips))) = state.chosen.take() {
                    return clips;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("the walk did not come");
        };
        state.click(Click::PlayTree(share.clone()));
        let first = walked(&mut state);
        assert_eq!(first.items, [share.join("a.mov")]);
        state.click(Click::PlayTree(share.clone()));
        assert!(matches!(&state.chosen, Some(Chosen::PlayTree(_, Ok(again))) if *again == first) && !state.walks.busy(), "a second walk");
        state.walked = Some((share.clone(), first, Instant::now().checked_sub(folder::ASKED_FOR).unwrap()));
        state.chosen = None;
        state.click(Click::PlayTree(share.clone()));
        assert!(state.chosen.is_none() && state.walks.waiting(&share), "a walk kept too long played");
        walked(&mut state);
    }

    /// read again, a folder shows what was put in it since, and not before; what was read of its files goes, to be
    /// read again.
    #[cfg(windows)]
    #[test]
    fn a_folder_read_again_shows_what_is_new_in_it() {
        let mut made = Scratch(Vec::new());
        let top = made.folder(std::env::temp_dir().join(format!("eyepiece-again-{}", std::process::id())));
        let mut state = State::new(Some(top.clone()), Vec::new(), None);
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut time = 0.0;
        listed(&ctx, &mut state, &top, &mut time);
        let late = made.folder(top.join("late"));
        pass(&ctx, &mut state, time, None);
        assert!(state.nav.entries(&top).is_empty(), "nothing is read again unasked");
        state.read.insert(top.join("a.mov"), Err("read before".to_string()));
        state.click(Click::Refresh);
        assert!(state.read.is_empty());
        for _ in 0..200 {
            if !state.nav.entries(&top).is_empty() {
                break;
            }
            time += 0.05;
            pass(&ctx, &mut state, time, None);
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(state.nav.entries(&top).iter().map(|e| e.path.clone()).collect::<Vec<_>>(), [late]);
    }

    /// adr 0004 through the browser itself, on a folder of this pc's own share: with a card selected and the pointer
    /// resting all over the cards, no picture and no metadata is asked of a file. asked for, the folder's previews are
    /// wanted of its cards and of no other folder's.
    #[cfg(windows)]
    #[test]
    fn a_network_folder_asks_nothing_of_its_files_until_its_previews_are_asked_for() {
        let mut made = Scratch(Vec::new());
        // as deep as the columns that fit, so they list only what the test made
        let top = made.folder(std::env::temp_dir().join(format!("eyepiece-share-{}", std::process::id())));
        let a = made.folder(top.join("a"));
        let b = made.folder(a.join("b"));
        let shots = made.folder(b.join("shots"));
        let other = made.folder(b.join("other"));
        let deeper = made.folder(shots.join("deeper"));
        for file in [shots.join("a.mov"), shots.join("b.mov"), deeper.join("c.mov"), other.join("d.mov")] {
            made.file(file);
        }
        let (pictures, wants, _got) = Pictures::new();
        let mut state = State::new(Some(shared(&shots)), Vec::new(), Some(pictures));
        let ctx = egui::Context::default();
        ctx.set_fonts(super::super::fonts());
        let mut time = 0.0;
        listed(&ctx, &mut state, &shared(&shots), &mut time);
        assert_eq!(state.nav.volume, Volume::Network);
        state.nav.choose_card(shared(&shots).join("a.mov"));
        // the pointer rests at every height of the main area's first cards, a second at each
        for y in (40..700).step_by(10) {
            for _ in 0..2 {
                pass(&ctx, &mut state, time, Some(Pos2::new(560.0, y as f32)));
                time += 1.0;
            }
        }
        // read again, the share's folders are listed again and nothing more is asked of them
        state.click(Click::Refresh);
        for _ in 0..200 {
            pass(&ctx, &mut state, time, Some(Pos2::new(560.0, 200.0)));
            time += 1.0;
            if !state.stale.contains(&shared(&shots)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(!state.stale.contains(&shared(&shots)), "the folder shown was not listed again");
        assert_eq!(wants.try_iter().flatten().count(), 0, "a picture was wanted of a file on a share");
        assert!(!state.metas.busy() && state.read.is_empty(), "a file on a share was read for its metadata");

        state.click(Click::Previews(shared(&shots)));
        pass(&ctx, &mut state, time, None);
        let wanted = wants.try_iter().last().expect("pictures are wanted");
        let mut of: Vec<PathBuf> = wanted.iter().map(|w| w.path.clone()).collect();
        of.sort();
        assert_eq!(of, [shared(&shots).join("a.mov"), shared(&shots).join("b.mov")]);
        assert!(wanted.iter().all(|w| w.asked && !w.all));
        // another folder of the share, not asked for, gets none
        state.click(Click::Place(shared(&other)));
        listed(&ctx, &mut state, &shared(&other), &mut time);
        pass(&ctx, &mut state, time, None);
        assert_eq!(wants.try_iter().last().map(|w| w.len()), Some(0));
    }
}
