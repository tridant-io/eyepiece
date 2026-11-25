//! a played folder: the clips of one folder that a tab plays one after another, in name order (the design's
//! fig. 14), or of a folder and all its subfolders. the rules here are the prototype's (`step` and `item` in
//! docs/design/plan.html).

use crate::browse::{self, Entry, Kind, Volume};
use crate::play;
use crate::settings::Loop;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// a walk of a folder's subfolders gathers this many clips at most, and lists this many folders at most: at the top
/// of a drive it would otherwise list for minutes
pub const MOST_CLIPS: usize = 2000;
pub const MOST_FOLDERS: usize = 10_000;
/// how long a network folder that was asked about plays when it is asked for again
pub const ASKED_FOR: Duration = Duration::from_secs(8);

/// what a folder plays: its clips in order, how many were left out because their data is not on the disk, whether
/// they come from its subfolders too (`deep`), and whether that walk stopped at its bound
#[derive(Clone, Debug, PartialEq)]
pub struct Clips {
    pub items: Vec<PathBuf>,
    pub left_out: usize,
    pub deep: bool,
    pub cut: bool,
}

impl Clips {
    /// what is said as they start to play: where the walk stopped, and what was left out as not on this disk
    pub fn said(&self) -> Option<String> {
        let stopped = self.cut.then(|| if self.items.len() >= MOST_CLIPS { format!("stopped at {MOST_CLIPS} videos: the rest are left out") } else { format!("stopped after {MOST_FOLDERS} folders: the rest are left out") });
        let away = (self.left_out > 0).then(|| format!("{} file{} not on this disk left out", self.left_out, if self.left_out == 1 { " is" } else { "s are" }));
        let said: Vec<String> = stopped.into_iter().chain(away).collect();
        (!said.is_empty()).then(|| said.join(" · "))
    }
}

/// the clips of a folder alone, from its listing.
pub fn listed(folder: &Path) -> Result<Clips, String> {
    gather(folder, false, usize::MAX, 1)
}

/// the clips of a folder and all its subfolders: its own in name order, then each subfolder's the same way, in name
/// order, depth first. from folder listings alone (adr 0004), and never into a link, which can lead back up the tree.
/// a subfolder that cannot be listed is passed over.
pub fn walk(folder: &Path) -> Result<Clips, String> {
    gather(folder, true, MOST_CLIPS, MOST_FOLDERS)
}

/// `walk`, or the folder alone, stopping at `most` clips or `folders` folders listed.
fn gather(folder: &Path, deep: bool, most: usize, folders: usize) -> Result<Clips, String> {
    let mut clips = Clips { items: Vec::new(), left_out: 0, deep, cut: false };
    let mut next = vec![folder.to_path_buf()];
    let mut listed = 0;
    while let Some(at) = next.pop() {
        if clips.items.len() >= most || listed == folders {
            clips.cut = true;
            break;
        }
        listed += 1;
        let entries = match browse::list(&at) {
            Ok(entries) => entries,
            Err(why) if listed == 1 => return Err(why.to_string().to_lowercase()),
            Err(_) => continue,
        };
        let (items, away) = items(&entries);
        clips.items.extend(items);
        clips.left_out += away;
        if deep {
            // the last on the stack is walked next: the first subfolder by name
            next.extend(entries.iter().rev().filter(|e| e.kind == Kind::Folder && !e.link).map(|e| e.path.clone()));
        }
    }
    clips.cut |= clips.items.len() > most;
    clips.items.truncate(most);
    Ok(clips)
}

/// a clip's name in a played folder's list: its path from the folder played, a slash between folders
/// ("dome/01_dusk.mov"), so clips of one name in two folders are told apart.
pub fn name_in(folder: &Path, file: &Path) -> String {
    match file.strip_prefix(folder) {
        Ok(within) => within.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
        Err(_) => file.file_name().unwrap_or(file.as_os_str()).to_string_lossy().into_owned(),
    }
}

/// the files of a folder that play, in the order its listing has them: its videos and sequences, a sequence by its
/// first file, and not its lone stills. with them, how many were left out because their data is not on the disk:
/// playing those would download them.
pub fn items(entries: &[Entry]) -> (Vec<PathBuf>, usize) {
    // a lone still has one frame and never plays to its end: play would stop on it
    let cards = || entries.iter().filter(|e| matches!(e.kind, Kind::Sequence { .. }) || (e.kind == Kind::Clip && !browse::is_still(&e.name)));
    (cards().filter(|e| !e.away).map(|e| e.path.clone()).collect(), cards().filter(|e| e.away).count())
}

/// what is asked before a folder is played: on a network volume, once, that it means opening its clips, with how
/// many (the design: "play folder on a network path asks once, with the count"). None when there is nothing to ask:
/// any other volume, or a folder that has just been asked about (`asked`). `deep`: it was asked for with its
/// subfolders.
pub fn ask(volume: Volume, clips: usize, deep: bool, asked: bool) -> Option<String> {
    let opens = if clips == 1 { "its 1 clip, and nothing else".to_string() } else { format!("its {clips} clips, each when it plays") };
    let again = if deep { "play with subfolders" } else { "play folder" };
    (volume == Volume::Network && !asked).then(|| format!("network folder · playing it opens {opens}. {again} again to go on"))
}

/// what play does at the ends of one clip of `items`, for the tab's loop mode: with `all`, a clip of several stops
/// at its end, for the next to take over, unless play runs backward, which goes round within the clip.
pub fn clip_loop(mode: Loop, items: usize, reversing: bool) -> play::Loop {
    match mode {
        Loop::Off => play::Loop::Off,
        Loop::One => play::Loop::One,
        Loop::All if items < 2 || reversing => play::Loop::One,
        Loop::All => play::Loop::Off,
    }
}

/// the clip play goes on to when it runs off the end of clip `at` going forward, or None when it stays where it is.
/// with `all` the folder goes round; with `off` it plays through once and stops on its last clip's last frame.
pub fn after(mode: Loop, items: usize, at: usize) -> Option<usize> {
    let more = items > 1 && (mode == Loop::All || at + 1 < items);
    (mode != Loop::One && more).then(|| (at + 1) % items)
}

/// the clip a step of `by` (1 or -1) from clip `at` goes to, round the folder's ends; None with one clip or none.
pub fn step(items: usize, at: usize, by: i64) -> Option<usize> {
    (items > 1).then(|| (at as i64 + by).rem_euclid(items as i64) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::tests::Scratch;

    fn entry(name: &str, kind: Kind, away: bool) -> Entry {
        Entry { name: name.to_string(), path: Path::new("shots").join(name), kind, size: Some(1), modified: None, created: None, added: None, away, link: false }
    }

    /// a folder's own clips first, then each subfolder's, in name order, depth first; a file that is no clip and a
    /// lone still are not played. each is named by its path from the folder played.
    #[test]
    fn a_folder_with_its_subfolders_plays_its_own_clips_then_each_subfolders_depth_first() {
        let mut s = Scratch::new("walk");
        for folder in ["b_dir", "b_dir/inner", "a_dir", "empty"] {
            s.folder(folder);
        }
        s.files(&["z_own.mov", "notes.txt", "b_dir/10.mov", "b_dir/2.mov", "b_dir/inner/c.mov", "a_dir/d.mov", "a_dir/plate.png"]);
        let walked = walk(&s.dir).unwrap();
        let names: Vec<String> = walked.items.iter().map(|file| name_in(&s.dir, file)).collect();
        assert_eq!(names, ["z_own.mov", "a_dir/d.mov", "b_dir/2.mov", "b_dir/10.mov", "b_dir/inner/c.mov"]);
        assert!(walked.deep && !walked.cut && walked.said().is_none());
        // the folder alone: its own clip, and nothing of its subfolders
        assert_eq!(listed(&s.dir).unwrap(), Clips { items: vec![s.dir.join("z_own.mov")], left_out: 0, deep: false, cut: false });
        // a folder that cannot be listed says why
        assert!(walk(&s.dir.join("no such")).is_err());
        // picked files are named by their own names, from the folder they are in
        assert_eq!(name_in(&s.dir.join("b_dir"), &s.dir.join("b_dir").join("2.mov")), "2.mov");
    }

    /// a walk stops at its bound of clips or of folders listed, keeps the first in its order, and says so.
    #[test]
    fn a_walk_stops_at_its_bound_and_says_so() {
        let mut s = Scratch::new("bound");
        for folder in ["a", "b"] {
            s.folder(folder);
        }
        s.files(&["own.mov", "a/1.mov", "a/2.mov", "b/3.mov"]);
        let cut = gather(&s.dir, true, 2, MOST_FOLDERS).unwrap();
        assert_eq!((cut.items, cut.cut), (vec![s.dir.join("own.mov"), s.dir.join("a").join("1.mov")], true));
        // two folders listed of three: b is not reached
        let cut = gather(&s.dir, true, MOST_CLIPS, 2).unwrap();
        assert_eq!((cut.items.len(), cut.cut), (3, true));
        let whole = gather(&s.dir, true, 4, 3).unwrap();
        assert_eq!((whole.items.len(), whole.cut), (4, false));
        let said = |items: usize, left_out: usize| Clips { items: vec![PathBuf::new(); items], left_out, deep: true, cut: true }.said().unwrap();
        assert_eq!(said(MOST_CLIPS, 0), "stopped at 2000 videos: the rest are left out");
        assert_eq!(said(12, 3), "stopped after 10000 folders: the rest are left out · 3 files are not on this disk left out");
    }

    /// a link to a folder (a junction, a symlink) is not gone into: here one back to the top, which would loop.
    #[test]
    fn a_walk_does_not_go_into_a_link() {
        let mut s = Scratch::new("link");
        s.folder("real");
        s.files(&["top.mov", "real/in.mov"]);
        let link = s.dir.join("loop");
        #[cfg(windows)]
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(&s.dir).output().is_ok_and(|out| out.status.success());
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&s.dir, &link).is_ok();
        if !made {
            eprintln!("skipped: the system made no link");
            return;
        }
        // a junction goes as a folder does, which removes the link and not what it leads to; a symlink as a file
        s.made.push((link.clone(), cfg!(windows)));
        assert!(browse::list(&s.dir).unwrap().iter().any(|e| e.path == link && e.link), "the listing does not mark the link");
        assert!(!browse::list(&s.dir).unwrap().iter().any(|e| e.name == "real" && e.link));
        assert_eq!(walk(&s.dir).unwrap().items, [s.dir.join("top.mov"), s.dir.join("real").join("in.mov")]);
    }

    /// adr 0004: through this pc's own share, which stands for a network volume, the walk is the same, from listings.
    #[cfg(windows)]
    #[test]
    fn a_walk_over_a_share_finds_what_a_local_one_does() {
        let mut s = Scratch::new("share");
        s.folder("in");
        s.files(&["a.mov", "in/b.mov"]);
        let local = s.dir.to_string_lossy();
        let share = PathBuf::from(format!(r"\\localhost\{}${}", &local[..1], &local[2..]));
        assert_eq!(browse::volume_of(&share), Volume::Network);
        assert_eq!(walk(&share).unwrap().items, [share.join("a.mov"), share.join("in").join("b.mov")]);
    }

    #[test]
    fn a_folder_plays_its_clips_that_are_on_the_disk() {
        let run = Kind::Sequence { first: 1, last: 9, missing: Vec::new(), pad: 4 };
        let entries = [entry("sub", Kind::Folder, false), entry("a.mov", Kind::Clip, false), entry("b.mov", Kind::Clip, true), entry("notes.txt", Kind::Other, false), entry("s.0001.exr", run, false), entry("plate.png", Kind::Clip, false)];
        let (plays, left_out) = items(&entries);
        assert_eq!(plays, [Path::new("shots").join("a.mov"), Path::new("shots").join("s.0001.exr")]);
        // a lone still has one frame and never plays to its end: play would stop on it, so it is not in the folder
        assert_eq!(left_out, 1);
    }

    #[test]
    fn a_folder_on_a_network_volume_is_asked_about_once_with_its_count() {
        assert_eq!((ask(Volume::Local, 300, false, false), ask(Volume::Removable, 300, true, false)), (None, None));
        let asks = ask(Volume::Network, 300, false, false).expect("it asks");
        assert!(asks.contains("300 clips") && asks.contains("network") && asks.ends_with("play folder again to go on"), "{asks}");
        // asked, it plays
        assert_eq!(ask(Volume::Network, 300, false, true), None);
        assert!(ask(Volume::Network, 1, false, false).expect("it asks").contains("1 clip,"));
        // with its subfolders, it names that to ask for again
        assert!(ask(Volume::Network, 2, true, false).expect("it asks").ends_with("play with subfolders again to go on"));
    }

    #[test]
    fn at_a_clips_end_the_loop_mode_says_what_comes() {
        use Loop::{All, Off, One};
        // (mode, clips, the one up) -> the clip that takes over
        for (mode, clips, at, next) in [
            (All, 3, 0, Some(1)),
            (All, 3, 2, Some(0)),
            (Off, 3, 0, Some(1)),
            (Off, 3, 2, None),
            (One, 3, 0, None),
            (All, 1, 0, None),
            (Off, 1, 0, None),
        ] {
            assert_eq!(after(mode, clips, at), next, "{mode:?} {clips} {at}");
        }
        // within a clip: one goes round; all goes round only where no other clip takes over, or backward
        assert_eq!((clip_loop(One, 3, false), clip_loop(Off, 3, false), clip_loop(Off, 1, true)), (play::Loop::One, play::Loop::Off, play::Loop::Off));
        assert_eq!((clip_loop(All, 3, false), clip_loop(All, 3, true), clip_loop(All, 1, false)), (play::Loop::Off, play::Loop::One, play::Loop::One));
    }

    #[test]
    fn page_keys_go_round_the_folder() {
        assert_eq!((step(3, 0, 1), step(3, 2, 1), step(3, 0, -1), step(1, 0, 1), step(0, 0, 1)), (Some(1), Some(0), Some(2), None, None));
    }
}
