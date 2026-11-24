//! where the browser is: the folders from a place down to the one the main area shows, what is selected in each,
//! and how the keys move about them. plain logic: the module above lists the folders off the drawing thread and
//! draws this.
//!
//! as in the design's fig. 28, a folder selected in the last column is shown in the main area before it is entered,
//! with no column of its own; entering it (right, or a double click) gives it its column.

use super::Chosen;
use crate::browse::{self, Entry, Kind, Volume};
use crate::event::{Key, Mods};
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
    /// its first card is selected when its listing comes in, if nothing is by then
    first_card: bool,
    /// how far its list is scrolled, and whether its selection is still to be brought into view
    pub scroll: f32,
    pub reveal: bool,
}

impl Column {
    fn new(folder: PathBuf, selected: Option<PathBuf>, first_card: bool) -> Column {
        Column { folder, selected, first_card, scroll: 0.0, reveal: true }
    }
}

/// where the arrows act: the last column, or the grid of cards
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

/// a card gets a poster and hover previews only when its file is on a local volume and its data is on the disk.
/// making them reads the file, so nothing on a network volume gets them, and no file that would have to be
/// downloaded first. asking for a file's metadata does not change that: the design asks for previews separately
/// (adr 0004), and that asking is not built.
pub fn makes_pictures(volume: Volume, entry: &Entry) -> bool {
    volume != Volume::Network && !entry.away && is_card(entry)
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
        out.push(Column::new(std::mem::replace(&mut at, next.clone()), Some(next), false));
    }
    out.push(Column::new(at, None, false));
    out
}

pub struct Nav {
    /// from the place the columns start at down to the folder the main area shows. never empty
    pub path: Vec<Column>,
    /// the last folder on the path is only shown, not entered: it has no column
    pub previewing: bool,
    pub focus: Focus,
    /// what the path is on
    pub volume: Volume,
    pub listings: HashMap<PathBuf, Listing>,
    /// what has been typed to select by name, and when the last letter came
    typed: String,
    typed_at: Option<Instant>,
}

impl Nav {
    /// at `folder`, its columns starting at the one of `places` that holds it.
    pub fn new(folder: &Path, places: &[PathBuf]) -> Nav {
        let root = root_of(folder, places);
        Nav {
            path: columns(&root, folder),
            previewing: false,
            focus: Focus::Columns,
            volume: browse::volume_of(&root),
            listings: HashMap::new(),
            typed: String::new(),
            typed_at: None,
        }
    }

    /// starts again at a place of the sidebar, with nothing selected.
    pub fn go(&mut self, place: &Path) {
        (self.path, self.previewing, self.focus) = (columns(place, place), false, Focus::Columns);
        self.volume = browse::volume_of(place);
    }

    /// the columns drawn: the whole path but a folder only shown
    pub fn shown(&self) -> &[Column] {
        &self.path[..self.path.len() - self.previewing as usize]
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

    /// the main area's cards
    pub fn cards(&self) -> impl Iterator<Item = &Entry> {
        self.entries(&self.main().folder).iter().filter(|e| is_card(e))
    }

    /// the selected card
    pub fn card(&self) -> Option<&Entry> {
        self.selected(self.main()).filter(|e| is_card(e))
    }

    /// selects an entry of drawn column `at`: the columns after it go, and a folder is shown in the main area until it
    /// is entered.
    pub fn select(&mut self, at: usize, entry: PathBuf, folder: bool) {
        // the folder already next on the path keeps what is selected in it
        let kept = folder && self.path.get(at + 1).is_some_and(|c| c.folder == entry);
        self.path.truncate(at + 1 + kept as usize);
        let column = &mut self.path[at];
        (column.selected, column.reveal) = (Some(entry.clone()), true);
        self.previewing = folder;
        if folder {
            if !kept {
                self.path.push(Column::new(entry, None, true));
            }
            self.focus = Focus::Columns;
        }
    }

    /// selects the entry at `index` of the active column.
    fn pick(&mut self, index: usize) {
        let at = self.active();
        let Some(entry) = self.entries(&self.path[at].folder).get(index) else { return };
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

    /// left: the active column goes, and the folder it listed is shown in the main area. from the first column, the
    /// folder above comes in on the left.
    fn back(&mut self) {
        if self.previewing {
            self.path.pop();
        }
        self.previewing = true;
        if self.path.len() == 1 && !self.reroot() {
            self.previewing = false;
        }
        self.focus = Focus::Columns;
    }

    /// backspace: the main area shows the folder above the one it shows.
    fn up(&mut self) {
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
        self.path.insert(0, Column::new(parent, Some(first), false));
        true
    }

    /// up or down the active column. with nothing selected, either selects the first entry.
    fn step(&mut self, by: isize) {
        let column = &self.path[self.active()];
        let entries = self.entries(&column.folder);
        let last = entries.len().saturating_sub(1) as isize;
        let at = column.selected.as_ref().and_then(|s| entries.iter().position(|e| &e.path == s));
        self.pick(at.map_or(0, |i| (i as isize + by).clamp(0, last) as usize));
    }

    /// the arrows, home and end in the grid. left from a card in its first column goes back to the columns.
    fn move_card(&mut self, key: Key, per_row: usize) {
        let cards: Vec<PathBuf> = self.cards().map(|e| e.path.clone()).collect();
        let Some(at) = cards.iter().position(|c| Some(c) == self.main().selected.as_ref()) else { return };
        let n = cards.len();
        let to = match key {
            Key::Left if at % per_row == 0 => {
                self.focus = Focus::Columns;
                return;
            }
            Key::Left => at - 1,
            Key::Right if at % per_row + 1 < per_row && at + 1 < n => at + 1,
            Key::Up if at >= per_row => at - per_row,
            Key::Down if at + per_row < n => at + per_row,
            // into the last row, which is shorter
            Key::Down if at / per_row < (n - 1) / per_row => n - 1,
            Key::Home => 0,
            Key::End => n - 1,
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
        match key {
            Key::Escape => return Some(Chosen::Back),
            Key::Enter if mods.shift => return Some(Chosen::PlayFolder(self.main().folder.clone())),
            Key::Enter => match self.card() {
                Some(card) => return Some(Chosen::Open(card.path.clone())),
                None => self.enter(),
            },
            Key::Backspace => self.up(),
            _ if self.focus == Focus::Grid => self.move_card(key, per_row.max(1)),
            Key::Up => self.step(-1),
            Key::Down => self.step(1),
            Key::Home => self.pick(0),
            Key::End => self.pick(self.entries(&self.path[self.active()].folder).len().saturating_sub(1)),
            Key::Left => self.back(),
            Key::Right => match self.selected(&self.path[self.active()]) {
                Some(e) if e.kind == Kind::Folder => self.enter(),
                Some(e) if is_card(e) => self.focus = Focus::Grid,
                _ => {}
            },
            _ => {}
        }
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
        let (at, grid) = (self.active(), self.focus == Focus::Grid);
        let found = self.entries(&self.path[at].folder).iter().find(|e| (!grid || is_card(e)) && e.name.to_lowercase().starts_with(&self.typed));
        if let Some((path, folder)) = found.map(|e| (e.path.clone(), e.kind == Kind::Folder)) {
            self.select(at, path, folder);
        }
    }

    /// a card clicked: its folder is entered if it was only shown, and the card selected.
    pub fn choose_card(&mut self, card: PathBuf) {
        if self.previewing {
            self.enter();
        }
        self.select(self.active(), card, false);
        self.focus = Focus::Grid;
    }

    /// once the main area's listing is in: a folder just shown or entered gets its first card selected.
    pub fn settle(&mut self) {
        let main = self.path.len() - 1;
        if !self.path[main].first_card || !self.listings.contains_key(&self.path[main].folder) {
            return;
        }
        self.path[main].first_card = false;
        if self.path[main].selected.is_none() {
            let first = self.cards().next().map(|e| e.path.clone());
            self.path[main].selected = first;
        }
    }

    /// the folders to list, most wanted first: the main area's, then the drawn columns from `first` on, deepest first.
    pub fn needed(&self, first: usize) -> Vec<PathBuf> {
        let mut out = vec![self.main().folder.clone()];
        for column in self.shown()[first.min(self.active())..].iter().rev() {
            if !out.contains(&column.folder) {
                out.push(column.folder.clone());
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
        Entry { name: name.to_string(), path: dir.join(name), kind, size: Some(10), modified: None, away: false }
    }

    /// a folder `r` on a drive that need not exist, with a folder `shots` holding two clips, then three files.
    fn tree() -> (Nav, PathBuf) {
        let r = PathBuf::from(r"C:\no\such\r");
        let shots = r.join("shots");
        let mut nav = Nav::new(&r, &[PathBuf::from(r"C:\no\such")]);
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

    fn opened(chosen: Option<Chosen>) -> Option<PathBuf> {
        match chosen {
            Some(Chosen::Open(file)) => Some(file),
            _ => None,
        }
    }

    #[test]
    fn the_columns_start_at_the_place_that_holds_the_folder() {
        let places = [PathBuf::from(r"C:\"), PathBuf::from(r"C:\Users\u\Downloads")];
        assert_eq!(root_of(Path::new(r"C:\Users\u\Downloads\a\b"), &places), Path::new(r"C:\Users\u\Downloads"));
        assert_eq!(root_of(Path::new(r"C:\Users\u\a"), &places), Path::new(r"C:\"));
        // a drive or share that is not a place starts at its own top
        assert_eq!(root_of(Path::new(r"E:\a\b"), &places), Path::new(r"E:\"));
        assert_eq!(root_of(Path::new(r"\\server\share\a"), &places), Path::new(r"\\server\share\"));

        let path = columns(Path::new(r"C:\d"), Path::new(r"C:\d\a\b"));
        let folders: Vec<&Path> = path.iter().map(|c| c.folder.as_path()).collect();
        assert_eq!(folders, [Path::new(r"C:\d"), Path::new(r"C:\d\a"), Path::new(r"C:\d\a\b")]);
        let selected: Vec<Option<&Path>> = path.iter().map(|c| c.selected.as_deref()).collect();
        assert_eq!(selected, [Some(Path::new(r"C:\d\a")), Some(Path::new(r"C:\d\a\b")), None]);
        assert_eq!(columns(Path::new(r"C:\d"), Path::new(r"C:\d")).len(), 1);
    }

    #[test]
    fn a_folder_selected_is_shown_before_it_is_entered() {
        let (mut nav, r) = tree();
        let shots = r.join("shots");
        assert_eq!(folders(&nav), [Path::new(r"C:\no\such"), r.as_path()]);
        assert!(nav.card().is_none());
        // down selects the first entry: a folder, shown in the main area with its first card selected, no column yet
        press(&mut nav, Key::Down);
        assert_eq!((nav.main().folder.as_path(), nav.previewing), (shots.as_path(), true));
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("c.mov"));
        assert_eq!(folders(&nav).len(), 2);
        // right enters it: a column of its own, the card still selected; down moves in it
        press(&mut nav, Key::Right);
        assert_eq!(folders(&nav), [Path::new(r"C:\no\such"), r.as_path(), shots.as_path()]);
        press(&mut nav, Key::Down);
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("d.mov"));
        // left: its column goes and it is shown again, as it was
        press(&mut nav, Key::Left);
        assert_eq!((folders(&nav).len(), nav.main().folder.as_path()), (2, shots.as_path()));
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("d.mov"));
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
        assert_eq!(folders(&nav), [Path::new(r"C:\no\such")]);
        press(&mut nav, Key::Backspace);
        assert_eq!(folders(&nav), [Path::new(r"C:\no")]);
        assert_eq!(nav.main().selected.as_deref(), Some(Path::new(r"C:\no\such")));
        press(&mut nav, Key::Backspace);
        assert_eq!(folders(&nav), [Path::new(r"C:\")]);
        // at the top of the drive it goes no further
        press(&mut nav, Key::Backspace);
        press(&mut nav, Key::Left);
        assert_eq!(folders(&nav), [Path::new(r"C:\")]);
    }

    #[test]
    fn the_arrows_move_about_the_grid_by_rows() {
        let r = PathBuf::from(r"C:\no\such\r");
        let mut nav = Nav::new(&r, std::slice::from_ref(&r));
        let names = ["c1.mov", "c2.mov", "c3.mov", "c4.mov", "c5.mov"];
        nav.listings.insert(r.clone(), Ok(names.iter().map(|n| entry(&r, n, Kind::Clip)).collect()));
        let at = |nav: &Nav| nav.card().map(|c| c.name.clone()).unwrap_or_default();
        press(&mut nav, Key::Down);
        // right from a clip in the column goes into the grid; two cards a row
        press(&mut nav, Key::Right);
        assert_eq!(nav.focus, Focus::Grid);
        press(&mut nav, Key::Right);
        assert_eq!(at(&nav), "c2.mov");
        // the end of a row is as far as right goes
        press(&mut nav, Key::Right);
        assert_eq!(at(&nav), "c2.mov");
        press(&mut nav, Key::Down);
        assert_eq!(at(&nav), "c4.mov");
        // the last row is shorter: down goes to its last card
        press(&mut nav, Key::Down);
        assert_eq!(at(&nav), "c5.mov");
        press(&mut nav, Key::Up);
        assert_eq!(at(&nav), "c3.mov");
        press(&mut nav, Key::Home);
        assert_eq!(at(&nav), "c1.mov");
        press(&mut nav, Key::End);
        assert_eq!(at(&nav), "c5.mov");
        // left from the grid's first column goes back to the columns, where up moves in the column again
        press(&mut nav, Key::Left);
        assert_eq!(nav.focus, Focus::Columns);
        press(&mut nav, Key::Up);
        assert_eq!(at(&nav), "c4.mov");
    }

    #[test]
    fn typed_letters_select_by_name_and_start_over_after_a_pause() {
        let r = PathBuf::from(r"C:\no\such\r");
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
        // a folder only shown: enter opens its selected card, as the card is what is selected
        press(&mut nav, Key::Down);
        assert_eq!(opened(press(&mut nav, Key::Enter)), Some(r.join(r"shots\c.mov")));
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
        nav.choose_card(r.join(r"shots\d.mov"));
        assert_eq!((nav.previewing, nav.focus, nav.shown().len()), (false, Focus::Grid, 3));
        assert_eq!(nav.card().map(|c| c.name.as_str()), Some("d.mov"));
    }

    #[test]
    fn pictures_are_made_only_of_local_files_that_are_on_the_disk() {
        let dir = Path::new("shots");
        let here = entry(dir, "a.mov", Kind::Clip);
        let cloud = Entry { away: true, ..entry(dir, "b.mov", Kind::Clip) };
        let run = entry(dir, "s.[0001-0003].exr", Kind::Sequence { first: 1, last: 3, missing: vec![], pad: 4 });
        assert!(makes_pictures(Volume::Local, &here) && makes_pictures(Volume::Local, &run));
        assert!(!makes_pictures(Volume::Local, &cloud));
        assert!(!makes_pictures(Volume::Network, &here) && !makes_pictures(Volume::Network, &run));
        // nor of what gets no card
        assert!(!makes_pictures(Volume::Local, &entry(dir, "notes.txt", Kind::Other)) && !makes_pictures(Volume::Local, &entry(dir, "sub", Kind::Folder)));
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
        assert_eq!(nav.needed(0), [r.join("shots"), r.clone(), PathBuf::from(r"C:\no\such")]);
        // the columns scrolled off the left are not listed
        assert_eq!(nav.needed(1), [r.join("shots"), r]);
    }
}
