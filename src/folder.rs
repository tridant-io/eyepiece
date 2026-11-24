//! a played folder: the clips of one folder that a tab plays one after another, in name order (the design's
//! fig. 14). the rules here are the prototype's (`step` and `item` in docs/design/plan.html).

use crate::browse::{self, Entry, Kind, Volume};
use crate::play;
use crate::settings::Loop;
use std::path::PathBuf;

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
/// any other volume, or a folder that has just been asked about (`asked`).
pub fn ask(volume: Volume, clips: usize, asked: bool) -> Option<String> {
    let opens = if clips == 1 { "its 1 clip, and nothing else".to_string() } else { format!("its {clips} clips, each when it plays") };
    (volume == Volume::Network && !asked).then(|| format!("network folder · playing it opens {opens}. play folder again to go on"))
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
    use std::path::Path;

    fn entry(name: &str, kind: Kind, away: bool) -> Entry {
        Entry { name: name.to_string(), path: Path::new("shots").join(name), kind, size: Some(1), modified: None, away }
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
        assert_eq!((ask(Volume::Local, 300, false), ask(Volume::Removable, 300, false)), (None, None));
        let asks = ask(Volume::Network, 300, false).expect("it asks");
        assert!(asks.contains("300 clips") && asks.contains("network"), "{asks}");
        // asked, it plays
        assert_eq!(ask(Volume::Network, 300, true), None);
        assert!(ask(Volume::Network, 1, false).expect("it asks").contains("1 clip,"));
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
