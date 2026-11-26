//! where the browser is: the folders from a place down to the one the main area shows, what is selected in each,
//! and how the keys move about them. plain logic: the module above lists the folders off the drawing thread and
//! draws this.
//!
//! as in the design's fig. 28, a folder selected in the last column is shown in the main area before it is entered;
//! entering it (right, or a double click) moves the keys into it. the keys act in one place at a time, as in finder's
//! column view: the last column entered, or the grid of cards.

use super::Chosen;
use crate::browse::{self, Entry, Kind, View, Volume};
use crate::event::{Key, Mods};
use crate::preview::make;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// typed letters add up to one name until the typing pauses this long
const PAUSE: Duration = Duration::from_secs(1);

/// a folder's entries, or why it could not be listed
pub type Listing = Result<Vec<Entry>, String>;

/// one folder on the path
pub struct Column {
    pub folder: PathBuf,
    /// the entry selected in it, by path: it holds while the listing is on its way, and is found again in a new one
    pub selected: Option<PathBuf>,
    /// the keys came into it before its listing did: what they land on is chosen when it comes
    arriving: bool,
    /// how far its list is scrolled, and whether its selection is still to be brought into view
    pub scroll: f32,
    pub reveal: bool,
}

impl Column {
    fn new(folder: PathBuf, selected: Option<PathBuf>) -> Column {
        Column { folder, selected, arriving: false, scroll: 0.0, reveal: true }
    }
}

/// where the arrows act: the last column entered, or the grid of cards
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Focus {
    Columns,
    Grid,
}

/// a clip or a sequence: what gets a card
pub fn is_card(entry: &Entry) -> bool {
    matches!(entry.kind, Kind::Clip | Kind::Sequence { .. })
}

/// a file whose metadata can be read: a clip that is not a still (stills and sequences have no reader yet)
pub fn readable(entry: &Entry) -> bool {
    entry.kind == Kind::Clip && !browse::is_still(&entry.name)
}

/// a file's metadata is read at once when it is on a local volume and its data is on the disk; on a network
/// volume, or for a file that would have to be downloaded first, only when asked for (adr 0004)
pub fn reads_metadata(volume: Volume, entry: &Entry, asked: bool) -> bool {
    (volume != Volume::Network && !entry.away) || asked
}

/// a card gets a poster and hover previews when its file's data is on the disk, and on a network volume only once the
/// previews of its folder were asked for (`asked`, adr 0004). making them reads the file, so no file that would have
/// to be downloaded first gets them, asked or not. asking for a file's metadata does not ask for its pictures.
pub fn makes_pictures(volume: Volume, entry: &Entry, asked: bool) -> bool {
    (volume != Volume::Network || asked) && !entry.away && is_card(entry)
}

/// how many files making a card's pictures reads at most: a clip's are all in its one file, a sequence's 24 previews
/// are a file each
pub fn files_read(entry: &Entry) -> usize {
    match &entry.kind {
        Kind::Sequence { first, last, .. } => make::frames((last - first + 1) as usize).len(),
        _ => 1,
    }
}

/// where the columns start for `folder`: the deepest of `places` that holds it, else its drive or share
pub fn root_of(folder: &Path, places: &[PathBuf]) -> PathBuf {
    let place = places.iter().filter(|p| folder.starts_with(p)).max_by_key(|p| p.components().count());
    place.cloned().unwrap_or_else(|| folder.ancestors().last().unwrap_or(folder).to_path_buf())
}

/// the columns from `root` down to `folder`, each with the next one selected and the last with nothing
pub fn columns(root: &Path, folder: &Path) -> Vec<Column> {
    let mut at = root.to_path_buf();
    let mut out = Vec::new();
    for part in folder.strip_prefix(root).unwrap_or(Path::new("")).components() {
        let next = at.join(part);
        out.push(Column::new(std::mem::replace(&mut at, next.clone()), Some(next)));
    }
    out.push(Column::new(at, None));
    out
}

pub struct Nav {
    /// from the place the columns start at down to the folder the main area shows. never empty
    pub path: Vec<Column>,
    /// the last folder on the path is only shown, not entered: it has no column
    pub previewing: bool,
    /// the folders picked together in the path's last column, in its order, when there are more than one: the main
    /// area shows the videos of them all, and none of them has a column of its own (the owner's ask, 2026-11-26)
    pub together: Vec<PathBuf>,
    pub focus: Focus,
    /// what the path is on
    pub volume: Volume,
    /// what is shown of the folder the main area is on: its column, its cards, or both
    pub view: View,
    pub listings: HashMap<PathBuf, Listing>,
    /// what has been typed to select by name, and when the last letter came
    typed: String,
    typed_at: Option<Instant>,
    /// the folders the main area has shown, oldest first, and which of them it shows now: back and forward go along
    /// them. `stepped`: the one it is on came by a step up or down a column, and the next such step takes its place
    visited: Vec<PathBuf>,
    at: usize,
    stepped: bool,
}

impl Nav {
    /// at `folder`, its columns starting at the one of `places` that holds it.
    pub fn new(folder: &Path, places: &[PathBuf]) -> Nav {
        let root = root_of(folder, places);
        Nav {
            path: columns(&root, folder),
            previewing: false,
            together: Vec::new(),
            focus: Focus::Columns,
            volume: browse::volume_of(&root),
            view: View::Both,
            listings: HashMap::new(),
            typed: String::new(),
            typed_at: None,
            visited: vec![folder.to_path_buf()],
            at: 0,
            stepped: false,
        }
    }

    /// starts again at a folder of the sidebar, with nothing selected in it: its columns start at the one of `places`
    /// that holds it, which is the folder itself for a place.
    pub fn go(&mut self, folder: &Path, places: &[PathBuf]) {
        let root = root_of(folder, places);
        (self.path, self.previewing, self.focus) = (columns(&root, folder), false, Focus::Columns);
        self.together.clear();
        self.volume = browse::volume_of(&root);
    }

    /// notes the folder the main area shows now, when it is another than before: the folders that lay forward of the
    /// old one go. a run of steps up and down a column (`stepping`) leaves the folder it ends on, not every one it
    /// passed, and a run that ends where it began leaves none. anything else done in a folder stepped to keeps it.
    pub fn visit(&mut self, stepping: bool) {
        let folder = self.main().folder.clone();
        if self.visited[self.at] == folder {
            self.stepped &= stepping;
            return;
        }
        if !(stepping && self.stepped) {
            self.at += 1;
        }
        self.visited.truncate(self.at);
        self.stepped = stepping;
        if self.at > 0 && self.visited[self.at - 1] == folder {
            (self.at, self.stepped) = (self.at - 1, false);
        } else {
            self.visited.push(folder);
        }
    }

    /// whether there is a folder `by` along those visited: -1 the one before the one shown, 1 the one after
    pub fn can_retrace(&self, by: isize) -> bool {
        self.at.checked_add_signed(by).is_some_and(|to| to < self.visited.len())
    }

    /// back (-1) or forward (1) along the folders visited: there, as `go` goes. nothing at either end.
    pub fn retrace(&mut self, by: isize, places: &[PathBuf]) {
        if self.can_retrace(by) {
            (self.at, self.stepped) = (self.at.wrapping_add_signed(by), false);
            let folder = self.visited[self.at].clone();
            self.go(&folder, places);
        }
    }

    /// the columns drawn: the whole path but a folder only shown
    pub fn shown(&self) -> &[Column] {
        &self.path[..self.path.len() - self.previewing as usize]
    }

    /// how many of the path's columns are drawn, from its end. with a folder's own column showing, every folder on
    /// the path has one, a folder only shown too (the owner's ask, 2026-11-24: its files are listed from the start,
    /// not once one of them is pressed). with the cards alone, the folders entered, less a last one that has no
    /// folder in it to list.
    pub fn drawn(&self) -> usize {
        if self.view != View::Gallery {
            return self.path.len();
        }
        let shown = self.shown().len();
        shown - (shown > 1 && self.bare()) as usize
    }

    /// with the cards alone showing, the last column entered has no folder in it to list
    fn bare(&self) -> bool {
        self.view == View::Gallery && matches!(self.listings.get(&self.path[self.active()].folder), Some(Ok(entries)) if !entries.iter().any(|e| e.kind == Kind::Folder))
    }

    /// the keys act in the grid: it was gone into, or with the cards alone showing it is all there is of the folder
    /// shown. with the column alone showing there is no grid.
    pub fn in_grid(&self) -> bool {
        self.view != View::List && (self.focus == Focus::Grid || self.bare())
    }

    /// the column the keys act in, unless they act in the grid: its selection is drawn as the current one, and the
    /// selections in the others as the way to it
    pub fn current_column(&self) -> Option<usize> {
        (!self.in_grid()).then(|| self.active())
    }

    /// what a column lists of a folder: everything in it, or with the cards alone showing its files, its folders.
    pub fn listed<'a>(&'a self, folder: &Path) -> impl Iterator<Item = &'a Entry> {
        let all = self.view != View::Gallery;
        self.entries(folder).iter().filter(move |e| all || e.kind == Kind::Folder)
    }

    /// the folder the main area shows
    pub fn main(&self) -> &Column {
        self.path.last().expect("the path is never empty")
    }

    /// the column the keys move in: the last one drawn
    fn active(&self) -> usize {
        self.shown().len() - 1
    }

    /// a folder's entries, once it is listed
    pub fn entries(&self, folder: &Path) -> &[Entry] {
        match self.listings.get(folder) {
            Some(Ok(entries)) => entries,
            _ => &[],
        }
    }

    /// the entry selected in a column, once its listing is in
    pub fn selected(&self, column: &Column) -> Option<&Entry> {
        let path = column.selected.as_ref()?;
        self.entries(&column.folder).iter().find(|e| &e.path == path)
    }

    /// the folders whose videos the main area shows: the one shown, or the ones picked together. with the column
    /// alone showing there is no main area: the folders picked are lit in their column and nothing more.
    pub fn shown_in_main(&self) -> &[PathBuf] {
        if self.together.is_empty() || self.view == View::List { std::slice::from_ref(&self.main().folder) } else { &self.together }
    }

    /// the main area's cards: of folders picked together, one folder's after another's
    pub fn cards(&self) -> impl Iterator<Item = &Entry> {
        self.shown_in_main().iter().flat_map(|folder| self.entries(folder)).filter(|e| is_card(e))
    }

    /// the selected card. a folder only shown has none: what was selected in it is not where the keys are
    pub fn card(&self) -> Option<&Entry> {
        let selected = self.main().selected.as_ref().filter(|_| !self.previewing)?;
        self.cards().find(|e| &e.path == selected)
    }

    /// folders of drawn column `at` are picked together: the path ends at their column, with none of them selected in
    /// it, so none has a column of its own.
    pub fn pick_together(&mut self, at: usize, folders: Vec<PathBuf>) {
        self.path.truncate(at + 1);
        (self.path[at].selected, self.path[at].arriving) = (None, false);
        (self.previewing, self.focus, self.together) = (false, Focus::Columns, folders);
    }

    /// selects an entry of drawn column `at`: the columns after it go, and a folder is shown in the main area until it
    /// is entered.
    pub fn select(&mut self, at: usize, entry: PathBuf, folder: bool) {
        // anything selected but a card of the folders picked together lets them go
        if folder || !self.cards().any(|card| card.path == entry) {
            self.together.clear();
        }
        // the folder already next on the path keeps what is selected in it
        let kept = folder && self.path.get(at + 1).is_some_and(|c| c.folder == entry);
        self.path.truncate(at + 1 + kept as usize);
        let column = &mut self.path[at];
        (column.selected, column.reveal, column.arriving) = (Some(entry.clone()), true, false);
        self.previewing = folder;
        if folder {
            if !kept {
                self.path.push(Column::new(entry, None));
            }
            self.focus = Focus::Columns;
        }
    }

    /// selects the entry at `index` of the active column.
    fn pick(&mut self, index: usize) {
        let at = self.active();
        let Some(entry) = self.listed(&self.path[at].folder).nth(index) else { return };
        let (path, folder) = (entry.path.clone(), entry.kind == Kind::Folder);
        self.select(at, path, folder);
    }

    /// the selected folder of the active column
    fn selected_folder(&self, column: &Column) -> Option<PathBuf> {
        self.selected(column).filter(|e| e.kind == Kind::Folder).map(|e| e.path.clone())
    }

    /// into the folder that is only shown, or that is selected in the active column.
    pub fn enter(&mut self) {
        if !self.previewing {
            let Some(folder) = self.selected_folder(&self.path[self.active()]) else { return };
            self.select(self.active(), folder, true);
        }
        self.previewing = false;
        let main = self.path.len() - 1;
        self.path[main].reveal = true;
        // a folder selected in it before is shown in turn, as it was
        if let Some(folder) = self.selected_folder(&self.path[main]) {
            self.select(main, folder, true);
        }
    }

    /// right, or enter, on a folder: into it, onto what was selected in it before, else its first entry. a folder
    /// with nothing in it is not gone into.
    fn into_folder(&mut self) {
        let Some(folder) = self.selected_folder(&self.path[self.active()]) else { return };
        if self.listings.contains_key(&folder) && self.listed(&folder).next().is_none() && !self.entries(&folder).iter().any(is_card) {
            return;
        }
        self.enter();
        self.arrive();
    }

    /// the keys have come into the active column: onto what was selected in it before, else its first entry. with
    /// the cards alone showing, into its cards when it has any, which its column does not list. once its listing is in.
    fn arrive(&mut self) {
        let at = self.active();
        let listed = self.listings.contains_key(&self.path[at].folder);
        self.path[at].arriving = !listed;
        if !listed {
            return;
        }
        let column = &self.path[at];
        let card = self.selected(column).filter(|e| is_card(e)).or_else(|| self.entries(&column.folder).iter().find(|e| is_card(e)));
        if let Some(card) = card.map(|e| e.path.clone()).filter(|_| self.view == View::Gallery) {
            self.select(at, card, false);
            self.focus = Focus::Grid;
            return;
        }
        self.focus = Focus::Columns;
        let column = &self.path[at];
        if !self.listed(&column.folder).any(|e| Some(&e.path) == column.selected.as_ref()) {
            self.pick(0);
        }
    }

    /// left: the keys go to the column on the left, and the column they leave stays, as what the folder they came
    /// from holds. from the first column, the folder above comes in on the left; at the top of a drive or share
    /// there is none.
    fn back(&mut self) {
        if self.active() == 0 && !self.reroot() {
            return;
        }
        if self.previewing {
            self.path.pop();
        }
        self.previewing = true;
        self.focus = Focus::Columns;
        self.together.clear();
    }

    /// left from the grid: to the column that lists its cards, on the card, or with the cards alone showing and that
    /// column not drawn, to the column on its left.
    fn leave_grid(&mut self) {
        if self.bare() {
            return self.back();
        }
        self.focus = Focus::Columns;
    }

    /// right in a column: into a folder; from a video, or from nothing in the folder shown, into the grid on that video
    /// or the first. with the column alone showing there is no grid to go into.
    fn right(&mut self) {
        let at = self.active();
        match self.selected(&self.path[at]) {
            Some(e) if e.kind == Kind::Folder => self.into_folder(),
            _ if self.view == View::List => {}
            Some(e) if is_card(e) => self.focus = Focus::Grid,
            Some(_) => {}
            None => {
                let first = self.cards().next().map(|e| e.path.clone());
                if let Some(first) = first {
                    self.select(at, first, false);
                    self.focus = Focus::Grid;
                }
            }
        }
    }

    /// backspace: the main area shows the folder above the one it shows.
    fn up(&mut self) {
        self.together.clear();
        if self.previewing {
            return self.back();
        }
        if self.path.len() > 1 || self.reroot() {
            self.path.pop();
            let main = self.path.len() - 1;
            self.path[main].reveal = true;
        }
        self.focus = Focus::Columns;
    }

    /// the folder above the first column becomes the first, with that one selected. false at the top of a drive or
    /// share.
    fn reroot(&mut self) -> bool {
        let first = self.path[0].folder.clone();
        let Some(parent) = first.parent().map(Path::to_path_buf) else { return false };
        self.path.insert(0, Column::new(parent, Some(first)));
        true
    }

    /// up or down the active column, stopping at its ends. with nothing selected, down selects the first entry and up
    /// the last.
    fn step(&mut self, by: isize) {
        let column = &self.path[self.active()];
        let entries: Vec<&Entry> = self.listed(&column.folder).collect();
        let last = entries.len().saturating_sub(1) as isize;
        let at = column.selected.as_ref().and_then(|s| entries.iter().position(|e| &e.path == s));
        self.pick(at.map_or(if by > 0 { 0 } else { last as usize }, |i| (i as isize + by).clamp(0, last) as usize));
    }

    /// the arrows, home and end in the grid: left and right go card by card in reading order, over the ends of the
    /// rows, and up and down a row. left from the first card leaves the grid. with no card selected, right and down
    /// select the first, up the last, and left leaves.
    fn move_card(&mut self, key: Key, per_row: usize) {
        let cards: Vec<PathBuf> = self.cards().map(|e| e.path.clone()).collect();
        let at = cards.iter().position(|c| Some(c) == self.main().selected.as_ref());
        let n = cards.len();
        let to = match (key, at) {
            (Key::Left, Some(0) | None) => return self.leave_grid(),
            _ if n == 0 => return,
            (Key::Home, _) | (Key::Right | Key::Down, None) => 0,
            (Key::End, _) | (Key::Up, None) => n - 1,
            (Key::Left, Some(i)) => i - 1,
            (Key::Right, Some(i)) => (i + 1).min(n - 1),
            (Key::Up, Some(i)) if i >= per_row => i - per_row,
            (Key::Down, Some(i)) if i + per_row < n => i + per_row,
            // into the last row, which is shorter
            (Key::Down, Some(i)) if i / per_row < (n - 1) / per_row => n - 1,
            _ => return,
        };
        self.select(self.active(), cards[to].clone(), false);
    }

    /// a key. returns what it asks of the host, if anything. `per_row` is how many cards the grid had across when it
    /// was last drawn.
    pub fn key(&mut self, key: Key, mods: Mods, per_row: usize) -> Option<Chosen> {
        let acts = matches!(key, Key::Escape | Key::Enter | Key::Backspace | Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End);
        if acts {
            // a key that moves ends what was being typed (letters come as characters)
            self.typed.clear();
        }
        // the grid was gone into, and the view has none now
        if !self.in_grid() {
            self.focus = Focus::Columns;
        }
        match key {
            Key::Escape => return Some(Chosen::Back),
            Key::Enter if mods.shift => return Some(Chosen::PlayFolder(self.main().folder.clone())),
            Key::Enter => match self.card() {
                Some(card) => return Some(Chosen::Open(card.path.clone())),
                None => self.into_folder(),
            },
            Key::Backspace => self.up(),
            _ if self.in_grid() => self.move_card(key, per_row.max(1)),
            Key::Up => self.step(-1),
            Key::Down => self.step(1),
            Key::Home => self.pick(0),
            Key::End => self.pick(self.listed(&self.path[self.active()].folder).count().saturating_sub(1)),
            Key::Left => self.back(),
            Key::Right => self.right(),
            _ => {}
        }
        self.visit(matches!(key, Key::Up | Key::Down | Key::Home | Key::End));
        None
    }

    /// a typed character: what has been typed so far selects the first entry whose name starts with it, in the active
    /// column (in the grid, the first card). after a pause the typing starts over.
    pub fn char(&mut self, c: char, now: Instant) {
        if self.typed_at.is_some_and(|at| now.duration_since(at) > PAUSE) {
            self.typed.clear();
        }
        self.typed_at = Some(now);
        self.typed.extend(c.to_lowercase());
        let (at, grid) = (self.active(), self.in_grid());
        let folders = self.view == View::Gallery;
        let named = |e: &&Entry| e.name.to_lowercase().starts_with(&self.typed);
        let found = if grid { self.cards().find(named) } else { self.entries(&self.path[at].folder).iter().filter(|e| !folders || e.kind == Kind::Folder).find(named) };
        if let Some((path, folder)) = found.map(|e| (e.path.clone(), e.kind == Kind::Folder)) {
            self.select(at, path, folder);
        }
        self.visit(true);
    }

    /// a card clicked: its folder is entered if it was only shown, and the card selected.
    pub fn choose_card(&mut self, card: PathBuf) {
        if self.previewing {
            self.enter();
        }
        self.select(self.active(), card, false);
        self.focus = Focus::Grid;
    }

    /// once the listing of a folder the keys came into is in: what they land on in it.
    pub fn settle(&mut self) {
        let at = self.active();
        if self.path[at].arriving && self.listings.contains_key(&self.path[at].folder) {
            self.arrive();
        }
    }

    /// the folders to list, most wanted first: the main area's, the ones picked together in it, then the drawn
    /// columns from `first` on, deepest first.
    pub fn needed(&self, first: usize) -> Vec<PathBuf> {
        let mut out = vec![self.main().folder.clone()];
        for folder in self.shown_in_main().iter().chain(self.shown()[first.min(self.active())..].iter().rev().map(|column| &column.folder)) {
            if !out.contains(folder) {
                out.push(folder.clone());
            }
        }
        out
    }

    /// the file whose metadata is to be read now: the selected card, when it can be read and may be (adr 0004).
    pub fn metadata(&self, asked: &HashSet<PathBuf>) -> Option<PathBuf> {
        let card = self.card().filter(|e| readable(e))?;
        reads_metadata(self.volume, card, asked.contains(&card.path)).then(|| card.path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(dir: &Path, name: &str, kind: Kind) -> Entry {
        Entry { name: name.to_string(), path: dir.join(name), kind, size: Some(10), modified: None, created: None, added: None, away: false, link: false }
    }

    /// a folder that need not exist, named from the top of a local drive: `C:\no\such` on windows, `/no/such` on
    /// macos. "" is the top itself.
    fn at(folder: &str) -> PathBuf {
        folder.split('/').filter(|part| !part.is_empty()).fold(PathBuf::from(if cfg!(windows) { r"C:\" } else { "/" }), |path, part| path.join(part))
    }

    /// a folder `r` on a drive that need not exist, with a folder `shots` holding two clips, then three files.
    fn tree() -> (Nav, PathBuf) {
        let r = at("no/such/r");
        let shots = r.join("shots");
        let mut nav = Nav::new(&r, &[at("no/such")]);
        nav.listings.insert(r.clone(), Ok(vec![entry(&r, "shots", Kind::Folder), entry(&r, "a.mov", Kind::Clip), entry(&r, "b.mov", Kind::Clip), entry(&r, "notes.txt", Kind::Other)]));
        nav.listings.insert(shots.clone(), Ok(vec![entry(&shots, "c.mov", Kind::Clip), entry(&shots, "d.mov", Kind::Clip)]));
        (nav, r)
    }

    fn press(nav: &mut Nav, key: Key) -> Option<Chosen> {
        let chosen = nav.key(key, Mods::default(), 2);
        nav.settle();
        chosen
    }

    fn folders(nav: &Nav) -> Vec<&Path> {
        nav.shown().iter().map(|c| c.folder.as_path()).collect()
    }

    fn name(path: &Path) -> String {
        path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())
    }

    /// where the keys act, the grid or a column named by its folder, and what is selected there: "r: a.mov"
    fn here(nav: &Nav) -> String {
        let column = &nav.path[nav.active()];
        let place = if nav.in_grid() { "grid".to_string() } else { name(&column.folder) };
        format!("{place}: {}", column.selected.as_deref().map_or(String::new(), name))
    }

    /// a place `r` with a folder holding a folder and three clips, a folder with nothing in it, a folder of two clips,
    /// then two clips and a file that is no video; shown as `view` has it
    fn wide(view: View) -> Nav {
        let r = at("no/such/r");
        let mut nav = Nav::new(&r, std::slice::from_ref(&r));
        nav.view = view;
        let clips = |dir: &Path, names: &[&str]| names.iter().map(|n| entry(dir, n, Kind::Clip)).collect::<Vec<_>>();
        let (mixed, shots) = (r.join("mixed"), r.join("shots"));
        let folders = ["mixed", "none", "shots"].map(|n| entry(&r, n, Kind::Folder));
        let files = clips(&r, &["a.mov", "b.mov"]).into_iter().chain([entry(&r, "notes.txt", Kind::Other)]);
        nav.listings.insert(r.clone(), Ok(folders.into_iter().chain(files).collect()));
        nav.listings.insert(mixed.clone(), Ok([entry(&mixed, "deep", Kind::Folder)].into_iter().chain(clips(&mixed, &["m1.mov", "m2.mov", "m3.mov"])).collect()));
        nav.listings.insert(mixed.join("deep"), Ok(clips(&mixed.join("deep"), &["z.mov"])));
        nav.listings.insert(r.join("none"), Ok(Vec::new()));
        nav.listings.insert(shots.clone(), Ok(clips(&shots, &["c.mov", "d.mov"])));
        nav
    }

    /// presses the keys, then says where the keys act and what is selected there
    fn after(nav: &mut Nav, keys: &[Key]) -> String {
        for key in keys {
            press(nav, *key);
        }
        here(nav)
    }

    const VIEWS: [View; 3] = [View::Both, View::List, View::Gallery];

    fn opened(chosen: Option<Chosen>) -> Option<PathBuf> {
        match chosen {
            Some(Chosen::Open(file)) => Some(file),
            _ => None,
        }
    }

    #[test]
    fn the_columns_start_at_the_place_that_holds_the_folder() {
        let places = [at(""), at("Users/u/Downloads")];
        assert_eq!(root_of(&at("Users/u/Downloads/a/b"), &places), at("Users/u/Downloads"));
        assert_eq!(root_of(&at("Users/u/a"), &places), at(""));
        // a folder no place holds starts at the top of what it is on: its drive or share
        assert_eq!(root_of(&at("a/b"), &places[1..]), at(""));
        #[cfg(windows)]
        {
            assert_eq!(root_of(Path::new(r"E:\a\b"), &places), Path::new(r"E:\"));
            assert_eq!(root_of(Path::new(r"\\server\share\a"), &places), Path::new(r"\\server\share\"));
        }

        let path = columns(&at("d"), &at("d/a/b"));
        let folders: Vec<&Path> = path.iter().map(|c| c.folder.as_path()).collect();
        assert_eq!(folders, [at("d"), at("d/a"), at("d/a/b")]);
        let selected: Vec<Option<PathBuf>> = path.iter().map(|c| c.selected.clone()).collect();
        assert_eq!(selected, [Some(at("d/a")), Some(at("d/a/b")), None]);
        assert_eq!(columns(&at("d"), &at("d")).len(), 1);
    }

    #[test]
    fn a_folder_selected_is_shown_before_it_is_entered() {
        let (mut nav, r) = tree();
        let shots = r.join("shots");
        assert_eq!(folders(&nav), [at("no/such"), r.clone()]);
        assert!(nav.card().is_none());
        // down selects the first entry: a folder, shown in the main area with nothing selected in it, not entered
        press(&mut nav, Key::Down);
        assert_eq!((nav.main().folder.as_path(), nav.previewing), (shots.as_path(), true));
        assert!(nav.card().is_none());
        assert_eq!(folders(&nav).len(), 2);
        // right enters it, onto its first card; down moves in it
        press(&mut nav, Key::Right);
        assert_eq!(folders(&nav), [at("no/such"), r.clone(), shots.clone()]);
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("c.mov"));
        press(&mut nav, Key::Down);
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("d.mov"));
        // left: it is shown again, with what was selected in it, but the keys are not there: no card is selected
        press(&mut nav, Key::Left);
        assert_eq!((folders(&nav).len(), nav.main().folder.as_path()), (2, shots.as_path()));
        assert_eq!((nav.main().selected.clone(), nav.card().is_none()), (Some(shots.join("d.mov")), true));
        // down in the folder above leaves it for a clip there
        press(&mut nav, Key::Down);
        assert_eq!((nav.main().folder.as_path(), nav.previewing), (r.as_path(), false));
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("a.mov"));
        // end and home go to the ends of the column; a file that is no video is no card
        press(&mut nav, Key::End);
        assert!(nav.card().is_none());
        press(&mut nav, Key::Home);
        assert!(nav.previewing);
    }

    #[test]
    fn backspace_goes_up_a_folder_and_left_from_the_first_column_brings_in_the_one_above() {
        let (mut nav, r) = tree();
        let shots = r.join("shots");
        press(&mut nav, Key::Down);
        press(&mut nav, Key::Right);
        press(&mut nav, Key::Backspace);
        // up from the folder entered: the main area shows the folder above, with it still selected there
        assert_eq!((nav.main().folder.as_path(), nav.previewing), (r.as_path(), false));
        assert_eq!(nav.main().selected.as_deref(), Some(shots.as_path()));
        assert!(nav.card().is_none());
        press(&mut nav, Key::Down);
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("a.mov"));
        // backspace again: the place the columns start at; and again, the folder above it comes in
        press(&mut nav, Key::Backspace);
        assert_eq!(folders(&nav), [at("no/such")]);
        press(&mut nav, Key::Backspace);
        assert_eq!(folders(&nav), [at("no")]);
        assert_eq!(nav.main().selected, Some(at("no/such")));
        press(&mut nav, Key::Backspace);
        assert_eq!(folders(&nav), [at("")]);
        // at the top of the drive it goes no further
        press(&mut nav, Key::Backspace);
        press(&mut nav, Key::Left);
        assert_eq!(folders(&nav), [at("")]);
    }

    /// in the grid, left and right go card by card in reading order, over the ends of the rows, and up and down a row;
    /// each stops at the ends. left from the very first card goes back to the column, on that card.
    #[test]
    fn the_grid_goes_card_by_card_in_reading_order_and_a_row_at_a_time() {
        let r = at("no/such/r");
        let mut nav = Nav::new(&r, std::slice::from_ref(&r));
        let names = ["c1.mov", "c2.mov", "c3.mov", "c4.mov", "c5.mov"];
        nav.listings.insert(r.clone(), Ok(names.iter().map(|n| entry(&r, n, Kind::Clip)).collect()));
        press(&mut nav, Key::Down);
        // right from a clip in the column goes into the grid on it; two cards a row
        press(&mut nav, Key::Right);
        assert_eq!(here(&nav), "grid: c1.mov");
        let walk = [(Key::Right, "c2.mov"), (Key::Right, "c3.mov"), (Key::Left, "c2.mov"), (Key::Down, "c4.mov"), (Key::Down, "c5.mov"), (Key::Down, "c5.mov"), (Key::Right, "c5.mov"), (Key::Up, "c3.mov"), (Key::Up, "c1.mov"), (Key::Up, "c1.mov"), (Key::End, "c5.mov"), (Key::Left, "c4.mov"), (Key::Left, "c3.mov"), (Key::Home, "c1.mov")];
        for (key, card) in walk {
            press(&mut nav, key);
            assert_eq!(here(&nav), format!("grid: {card}"), "after {key:?}");
        }
        // the column, on the card left from; up and down move in the column again
        press(&mut nav, Key::Left);
        assert_eq!(here(&nav), "r: c1.mov");
        press(&mut nav, Key::Down);
        assert_eq!(here(&nav), "r: c2.mov");
    }

    #[test]
    fn typed_letters_select_by_name_and_start_over_after_a_pause() {
        let r = at("no/such/r");
        let mut nav = Nav::new(&r, std::slice::from_ref(&r));
        nav.listings.insert(r.clone(), Ok(["a.mov", "w.mov", "wall_v011.mov", "wall_v012.mov"].iter().map(|n| entry(&r, n, Kind::Clip)).collect()));
        let at = |nav: &Nav| nav.card().map(|c| c.name.clone()).unwrap_or_default();
        let t0 = Instant::now();
        nav.char('W', t0);
        assert_eq!(at(&nav), "w.mov");
        nav.char('a', t0 + Duration::from_millis(80));
        assert_eq!(at(&nav), "wall_v011.mov");
        // nothing starts with this: the selection stays
        nav.char('q', t0 + Duration::from_millis(160));
        assert_eq!(at(&nav), "wall_v011.mov");
        // after a pause the typing starts over
        nav.char('a', t0 + Duration::from_millis(1500));
        assert_eq!(at(&nav), "a.mov");
        // and a key that moves starts it over too: this "a" is not "aa"
        press(&mut nav, Key::Down);
        assert_eq!(at(&nav), "w.mov");
        nav.char('a', t0 + Duration::from_millis(1600));
        assert_eq!(at(&nav), "a.mov");
    }

    #[test]
    fn enter_opens_the_selected_card_and_shift_enter_plays_the_folder() {
        let (mut nav, r) = tree();
        // nothing selected: enter does nothing
        assert!(press(&mut nav, Key::Enter).is_none());
        assert_eq!(here(&nav), "r: ");
        // a folder: enter goes into it as right does, and then opens the card it lands on
        press(&mut nav, Key::Down);
        assert!(press(&mut nav, Key::Enter).is_none());
        assert_eq!(here(&nav), "shots: c.mov");
        assert_eq!(opened(press(&mut nav, Key::Enter)), Some(r.join("shots").join("c.mov")));
        // back out, the card is still selected in the folder shown, but enter goes into the folder, not to the card
        press(&mut nav, Key::Left);
        assert!(press(&mut nav, Key::Enter).is_none());
        assert_eq!(here(&nav), "shots: c.mov");
        press(&mut nav, Key::Left);
        press(&mut nav, Key::Down);
        assert_eq!(opened(press(&mut nav, Key::Enter)), Some(r.join("a.mov")));
        let shift = Mods { shift: true, ..Mods::default() };
        assert!(matches!(nav.key(Key::Enter, shift, 2), Some(Chosen::PlayFolder(f)) if f == r));
        assert!(matches!(nav.key(Key::Escape, Mods::default(), 2), Some(Chosen::Back)));
    }

    #[test]
    fn a_card_clicked_in_a_folder_only_shown_enters_it() {
        let (mut nav, r) = tree();
        press(&mut nav, Key::Down);
        nav.choose_card(r.join("shots").join("d.mov"));
        assert_eq!((nav.previewing, nav.focus, nav.shown().len()), (false, Focus::Grid, 3));
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("d.mov"));
    }

    #[test]
    fn pictures_are_made_of_files_on_the_disk_and_on_a_share_only_when_asked() {
        let dir = Path::new("shots");
        let here = entry(dir, "a.mov", Kind::Clip);
        let cloud = Entry { away: true, ..entry(dir, "b.mov", Kind::Clip) };
        let run = entry(dir, "s.[0001-0003].exr", Kind::Sequence { first: 1, last: 3, missing: vec![], pad: 4 });
        assert!(makes_pictures(Volume::Local, &here, false) && makes_pictures(Volume::Local, &run, false));
        assert!(!makes_pictures(Volume::Local, &cloud, false));
        assert!(!makes_pictures(Volume::Network, &here, false) && !makes_pictures(Volume::Network, &run, false));
        // asked for: a share's files get them, and still no file that would have to be downloaded
        assert!(makes_pictures(Volume::Network, &here, true) && makes_pictures(Volume::Network, &run, true));
        assert!(!makes_pictures(Volume::Network, &cloud, true) && !makes_pictures(Volume::Local, &cloud, true));
        // nor of what gets no card
        assert!(!makes_pictures(Volume::Local, &entry(dir, "notes.txt", Kind::Other), true) && !makes_pictures(Volume::Local, &entry(dir, "sub", Kind::Folder), true));
        // what making them reads: one file for a clip, a file a preview for a sequence
        let long = entry(dir, "l.[0001-0240].exr", Kind::Sequence { first: 1, last: 240, missing: vec![], pad: 4 });
        assert_eq!((files_read(&here), files_read(&run), files_read(&long)), (1, 3, 24));
    }

    #[cfg(windows)]
    #[test]
    fn a_folder_of_the_sidebar_starts_its_columns_at_the_place_that_holds_it() {
        let (mut nav, r) = tree();
        let places = [PathBuf::from(r"C:\no\such"), PathBuf::from(r"\\server\share\pinned")];
        // a recent folder under a place: the columns from the place down to it, nothing selected in it
        nav.go(&r.join("shots"), &places);
        assert_eq!(folders(&nav), [Path::new(r"C:\no\such"), r.as_path(), r.join("shots").as_path()]);
        assert_eq!((nav.main().folder.clone(), nav.previewing, nav.main().selected.clone()), (r.join("shots"), false, None));
        // a place is the first column itself
        nav.go(&places[1], &places);
        assert_eq!((folders(&nav), nav.volume), (vec![places[1].as_path()], Volume::Network));
    }

    #[test]
    fn a_network_folder_reads_no_metadata_until_asked() {
        let share = PathBuf::from(r"\\server\share\shots");
        let mut nav = Nav::new(&share, &[]);
        assert_eq!(nav.volume, Volume::Network);
        let seq = Kind::Sequence { first: 1, last: 3, missing: vec![2], pad: 4 };
        nav.listings.insert(share.clone(), Ok(vec![entry(&share, "a.mov", Kind::Clip), entry(&share, "s.png", Kind::Clip), entry(&share, "q.[0001-0003].exr", seq)]));
        press(&mut nav, Key::Down);
        let mut asked = HashSet::new();
        assert_eq!(nav.metadata(&asked), None);
        asked.insert(share.join("a.mov"));
        assert_eq!(nav.metadata(&asked), Some(share.join("a.mov")));

        // nor is a file whose data is not on the disk, wherever it is: reading it would download it
        let (mut nav, r) = tree();
        let folder = r.join("shots");
        nav.listings.insert(folder.clone(), Ok(vec![Entry { away: true, ..entry(&folder, "cloud.mov", Kind::Clip) }]));
        press(&mut nav, Key::Down);
        press(&mut nav, Key::Right);
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("cloud.mov"));
        let mut asked = HashSet::new();
        assert_eq!(nav.metadata(&asked), None);
        asked.insert(folder.join("cloud.mov"));
        assert_eq!(nav.metadata(&asked), Some(folder.join("cloud.mov")));

        // a local folder reads the selected clip's at once; a still or a sequence has no reader yet
        let (mut nav, r) = tree();
        assert_eq!(nav.volume, Volume::Local);
        press(&mut nav, Key::Down);
        press(&mut nav, Key::Down);
        assert_eq!(nav.metadata(&HashSet::new()), Some(r.join("a.mov")));
        assert!(!readable(&entry(&r, "s.png", Kind::Clip)));
        assert!(!readable(&entry(&r, "q.[0001-0003].exr", Kind::Sequence { first: 1, last: 3, missing: vec![], pad: 4 })));
    }

    #[test]
    fn the_folder_shown_is_listed_first() {
        let (mut nav, r) = tree();
        press(&mut nav, Key::Down);
        assert_eq!(nav.needed(0), [r.join("shots"), r.clone(), at("no/such")]);
        // the columns scrolled off the left are not listed
        assert_eq!(nav.needed(1), [r.join("shots"), r]);
    }

    /// back and forward go along the folders the main area has shown. a run of steps up and down a column leaves
    /// the folder it ends on, not every one passed, and a run that ends where it began leaves none; going somewhere
    /// new from the middle drops the folders that lay forward.
    #[test]
    fn the_folders_shown_are_gone_back_and_forward_along() {
        let (mut nav, r) = tree();
        let (places, shots) = ([at("no/such")], r.join("shots"));
        assert!(!nav.can_retrace(-1) && !nav.can_retrace(1));
        // down the column: shots is shown, then a file selected puts r back in the main area
        press(&mut nav, Key::Down);
        assert!(nav.main().folder == shots && nav.can_retrace(-1));
        press(&mut nav, Key::Down);
        assert!(nav.main().folder == r && !nav.can_retrace(-1));
        // shown again and entered, then up from it: r, shots, r
        press(&mut nav, Key::Up);
        press(&mut nav, Key::Right);
        press(&mut nav, Key::Backspace);
        assert_eq!(nav.main().folder, r);
        nav.retrace(-1, &places);
        assert_eq!(nav.main().folder, shots);
        nav.retrace(-1, &places);
        assert!(nav.main().folder == r && !nav.can_retrace(-1) && nav.can_retrace(1));
        nav.retrace(-1, &places);
        assert_eq!(nav.main().folder, r, "there is nowhere further back");
        nav.retrace(1, &places);
        assert_eq!(nav.main().folder, shots);
        // somewhere new from the middle: what lay forward is gone
        nav.go(&at("no/such/other"), &places);
        nav.visit(false);
        assert!(!nav.can_retrace(1));
        nav.retrace(-1, &places);
        assert_eq!(nav.main().folder, shots);
    }

    /// what is shown of a folder: with its own column, a folder only shown has one too; with the cards alone the
    /// columns list folders only, the keys step over nothing else, and a column with no folder to list is not drawn;
    /// with the column alone there is no grid for the keys to go into.
    #[test]
    fn the_view_says_which_columns_are_drawn_and_what_they_list() {
        let (mut nav, r) = tree();
        let shots = r.join("shots");
        press(&mut nav, Key::Down);
        assert!(nav.main().folder == shots && nav.previewing);
        assert_eq!((nav.drawn(), nav.shown().len()), (3, 2), "the folder shown has its column from the start");
        nav.view = View::Gallery;
        assert_eq!(nav.listed(&r).map(|e| e.name.as_str()).collect::<Vec<_>>(), ["shots"]);
        assert_eq!(nav.drawn(), 2);
        // down again finds no other folder: the files are the cards' business
        press(&mut nav, Key::Down);
        assert_eq!(nav.main().folder, shots);
        // entered, shots has no folder to list, so no column of its own: right goes into its cards, on the first
        press(&mut nav, Key::Right);
        assert_eq!((nav.shown().len(), nav.drawn()), (3, 2));
        assert_eq!(here(&nav), "grid: c.mov");
        // with the column alone the keys are in the column, where right finds no grid to go into
        nav.view = View::List;
        assert_eq!((nav.drawn(), here(&nav)), (3, "shots: c.mov".to_string()));
        press(&mut nav, Key::Right);
        assert_eq!(here(&nav), "shots: c.mov", "there is no grid to go into");
        press(&mut nav, Key::Down);
        assert_eq!(here(&nav), "shots: d.mov");
    }

    /// up and down move in the column the keys are in and stop at its ends; with nothing selected, down takes the first
    /// entry and up the last. a folder moved onto is shown in the main area, and the keys stay where they are.
    #[test]
    fn up_and_down_move_in_the_column_and_stop_at_its_ends() {
        use Key::{Down, Left, Right, Up};
        for view in VIEWS {
            let last = if view == View::Gallery { "shots" } else { "notes.txt" };
            let mut nav = wide(view);
            assert_eq!(after(&mut nav, &[Down]), "r: mixed", "{view:?}");
            assert_eq!((name(&nav.main().folder), nav.previewing, nav.card().is_none()), ("mixed".to_string(), true, true));
            assert_eq!(after(&mut nav, &[Down; 7]), format!("r: {last}"), "{view:?}");
            assert_eq!(after(&mut nav, &[Up; 7]), "r: mixed", "{view:?}");
            assert_eq!(after(&mut wide(view), &[Up]), format!("r: {last}"), "{view:?}");
        }
        // in a column further in: down onto a video selects it in the folder shown, up onto a folder shows that
        for view in [View::Both, View::List] {
            let mut nav = wide(view);
            assert_eq!(after(&mut nav, &[Down, Right, Down]), "mixed: m1.mov", "{view:?}");
            assert_eq!((name(&nav.main().folder), nav.card().map(|c| c.name.as_str())), ("mixed".to_string(), Some("m1.mov")));
            assert_eq!(after(&mut nav, &[Up]), "mixed: deep", "{view:?}");
            assert_eq!((name(&nav.main().folder), nav.previewing), ("deep".to_string(), true));
        }
        // with the cards alone the column lists folders only
        let mut nav = wide(View::Gallery);
        assert_eq!(after(&mut nav, &[Down, Right, Left, Down]), "mixed: deep");
        assert_eq!(after(&mut nav, &[Down]), "mixed: deep");
    }

    /// right: on a folder, into its column, onto what was selected there before, else its first entry; on a video,
    /// into the grid on that card, and with nothing selected on the first. a folder with nothing in it and a file that
    /// is no video go nowhere.
    #[test]
    fn right_goes_into_a_folder_onto_what_was_selected_there_else_its_first() {
        use Key::{Down, End, Left, Right};
        for view in [View::Both, View::List] {
            let mut nav = wide(view);
            assert_eq!(after(&mut nav, &[Down, Right]), "mixed: deep", "{view:?}");
            assert_eq!((name(&nav.main().folder), nav.previewing), ("deep".to_string(), true), "its first entry, a folder, is shown");
            assert_eq!(after(&mut nav, &[Right]), "deep: z.mov", "{view:?}");
            assert_eq!(after(&mut wide(view), &[Down, Down, Right]), "r: none", "{view:?}");
            assert_eq!(after(&mut wide(view), &[End, Right]), "r: notes.txt", "{view:?}");
            let mut nav = wide(view);
            assert_eq!(after(&mut nav, &[Down, Down, Down, Right, Down, Left]), "r: shots", "{view:?}");
            assert_eq!(after(&mut nav, &[Right]), "shots: d.mov", "{view:?}");
        }
        assert_eq!(after(&mut wide(View::Both), &[Down, Right, Right, Right]), "grid: z.mov");
        assert_eq!(after(&mut wide(View::Both), &[Right]), "grid: a.mov");
        assert_eq!(after(&mut wide(View::Both), &[Down, Right, Right, Right, Right]), "grid: z.mov", "the last card is as far as it goes");
        // with the column alone there is no grid to go into
        assert_eq!(after(&mut wide(View::List), &[Down, Right, Right, Right]), "deep: z.mov");
        assert_eq!(after(&mut wide(View::List), &[Right]), "r: ");
        // with the cards alone the columns list folders: into a folder's cards when it has any, else into its folders
        assert_eq!(after(&mut wide(View::Gallery), &[Right]), "grid: a.mov");
        assert_eq!(after(&mut wide(View::Gallery), &[Down, Right]), "grid: m1.mov");
        assert_eq!(after(&mut wide(View::Gallery), &[Down, Down, Right]), "r: none");
        assert_eq!(after(&mut wide(View::Gallery), &[Down, Down, Down, Right]), "grid: c.mov");
        let mut nav = wide(View::Gallery);
        let mixed = at("no/such/r/mixed");
        nav.listings.insert(mixed.clone(), Ok(vec![entry(&mixed, "deep", Kind::Folder)]));
        assert_eq!(after(&mut nav, &[Down, Right]), "mixed: deep");
        // a folder not listed yet: the keys go into it, and land on its first entry when its listing comes
        for view in VIEWS {
            let mut nav = wide(view);
            let shots = at("no/such/r/shots");
            let listing = nav.listings.remove(&shots).unwrap();
            assert_eq!(after(&mut nav, &[Down, Down, Down, Right]), "shots: ", "{view:?}");
            nav.listings.insert(shots, listing);
            nav.settle();
            assert_eq!(here(&nav), if view == View::Gallery { "grid: c.mov" } else { "shots: c.mov" }, "{view:?}");
        }
    }

    /// left: the keys go to the column on the left, onto the folder they came from, which stays shown with what was
    /// selected in it; from the first column the folder above comes in. from the grid's first card, back to the
    /// column that lists the cards, on that card.
    #[test]
    fn left_goes_to_the_column_on_the_left_and_the_one_left_stays_shown() {
        use Key::{Down, Left, Right};
        for view in [View::Both, View::List] {
            let mut nav = wide(view);
            assert_eq!(after(&mut nav, &[Down, Right, Right]), "deep: z.mov", "{view:?}");
            assert_eq!(after(&mut nav, &[Left]), "mixed: deep", "{view:?}");
            assert_eq!((name(&nav.main().folder), nav.previewing, nav.drawn()), ("deep".to_string(), true, 3), "the column left stays");
            assert_eq!(nav.main().selected.as_deref().map(name), Some("z.mov".to_string()));
            assert_eq!(after(&mut nav, &[Left]), "r: mixed", "{view:?}");
            assert_eq!(after(&mut nav, &[Left]), "such: r", "{view:?}");
            assert_eq!(folders(&nav), [at("no/such")]);
        }
        assert_eq!(after(&mut wide(View::Both), &[Right, Left]), "r: a.mov");
        // with the cards alone: from the first card to the column of the folder's own folders, or where it has none,
        // to the column on the left
        assert_eq!(after(&mut wide(View::Gallery), &[Down, Right, Left]), "mixed: m1.mov");
        let mut nav = wide(View::Gallery);
        assert_eq!(after(&mut nav, &[Down, Right, Left, Down, Right]), "grid: z.mov");
        assert_eq!(after(&mut nav, &[Left]), "mixed: deep");
        assert_eq!(after(&mut nav, &[Left]), "r: mixed");
        assert_eq!(after(&mut nav, &[Right]), "grid: m1.mov", "back in, onto its cards: its folders are a left away");
        assert_eq!(after(&mut nav, &[Left, Down]), "mixed: deep");
    }

    #[test]
    fn home_and_end_go_to_the_ends_of_the_column_or_the_grid() {
        use Key::{Down, End, Home, Right};
        for view in VIEWS {
            let last = if view == View::Gallery { "shots" } else { "notes.txt" };
            let mut nav = wide(view);
            assert_eq!(after(&mut nav, &[End]), format!("r: {last}"), "{view:?}");
            assert_eq!(after(&mut nav, &[Home]), "r: mixed", "{view:?}");
        }
        let mut nav = wide(View::Both);
        assert_eq!(after(&mut nav, &[Down, Right, Down, Right, End]), "grid: m3.mov");
        assert_eq!(after(&mut nav, &[Home]), "grid: m1.mov");
        assert_eq!(after(&mut wide(View::Gallery), &[Right, End]), "grid: b.mov");
    }

    /// enter, backspace and typed names act where the keys are: in the grid a name selects a card, in a column an
    /// entry of it, and with the cards alone a folder; backspace from the grid goes up a folder as from a column.
    #[test]
    fn enter_backspace_and_names_act_where_the_keys_are() {
        use Key::{Backspace, Down, Right};
        let t0 = Instant::now();
        let mut nav = wide(View::Both);
        assert_eq!(after(&mut nav, &[Down, Right, Down, Right]), "grid: m1.mov");
        nav.char('m', t0);
        nav.char('3', t0 + Duration::from_millis(50));
        assert_eq!(here(&nav), "grid: m3.mov");
        assert_eq!(opened(press(&mut nav, Key::Enter)), Some(at("no/such/r/mixed/m3.mov")));
        assert_eq!(after(&mut nav, &[Backspace]), "r: mixed");
        assert_eq!((name(&nav.main().folder), nav.previewing), ("r".to_string(), false));
        nav.char('a', t0 + Duration::from_secs(5));
        assert_eq!(here(&nav), "r: a.mov");
        let mut nav = wide(View::Gallery);
        nav.char('a', t0);
        assert_eq!(here(&nav), "r: ", "with the cards alone the column lists no video");
        nav.char('s', t0 + Duration::from_secs(5));
        assert_eq!(here(&nav), "r: shots");
    }

    /// the keys act where the view draws them: switched to the column alone they are in the column; with the cards
    /// alone and a folder that holds no folder, in the grid from the start, where left goes to the column on the left.
    #[test]
    fn the_keys_act_where_the_view_draws_them() {
        use Key::{Down, Left, Right, Up};
        let mut nav = wide(View::Both);
        assert_eq!(after(&mut nav, &[Right, Right]), "grid: b.mov");
        nav.view = View::List;
        assert_eq!(here(&nav), "r: b.mov");
        assert_eq!(after(&mut nav, &[Up]), "r: a.mov");
        nav.view = View::Gallery;
        assert_eq!(here(&nav), "r: a.mov");
        assert_eq!(after(&mut nav, &[Right]), "grid: a.mov");
        let mut nav = wide(View::Gallery);
        nav.go(&at("no/such/r/shots"), &[at("no/such/r")]);
        assert_eq!(here(&nav), "grid: ");
        assert_eq!(after(&mut nav, &[Down]), "grid: c.mov");
        assert_eq!(after(&mut nav, &[Left]), "r: shots");
        assert_eq!(nav.current_column(), Some(0));
    }
}
