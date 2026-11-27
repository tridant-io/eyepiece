//! saving a copy of a clip for the user (adr 0009): its trim, or the clip without its sound, beside it under a name
//! of its own and never over another file; or, asked, in its place, the clip itself to the recycle bin (the trash on a
//! mac), never deleted. the copy is written under a passing name, checked, and only then takes its own; one that
//! fails is kept as `<name>.failed`. a copy saved is logged on its clip's notes, and takes the clip's groups on its
//! frames as its own notes, or none (adr 0010). it runs on a thread of its own, which `Saving` follows.

use crate::browse::{self, Volume};
use crate::event::Event;
use crate::markup::Group;
use crate::media::copy::{self, Cut};
use crate::notes;
use crate::sys::files;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::SystemTime;

/// where a clip's original goes when a copy takes its place
pub const BIN: &str = if cfg!(target_os = "macos") { "trash" } else { "recycle bin" };

/// a copy asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub path: PathBuf,
    /// the frames kept, both ends among them
    pub frames: (usize, usize),
    /// they are a trim, not the whole clip
    pub trim: bool,
    pub sound: bool,
    /// the copy takes the clip's place
    pub replace: bool,
    pub notes: Noting,
}

/// what a copy saved writes in the notes, as the settings say: the clip's groups, which go with the copy as its own
/// notes, or none when they are left out; beside the clip, or not; and eyepiece's own folder, where they go when not,
/// and when they cannot
#[derive(Clone, Debug, PartialEq)]
pub struct Noting {
    pub groups: Option<Vec<Group>>,
    pub beside: bool,
    pub own: Option<PathBuf>,
}

/// what came of a copy: what to say, the file it names, and the clip whose place a copy took.
#[derive(Debug, PartialEq)]
pub struct Done {
    pub said: String,
    pub file: Option<PathBuf>,
    pub replaced: Option<PathBuf>,
}

/// a copy being saved: how far it has got, a way to stop it, and what came of it once it ends. dropped, it is stopped
/// and waited for, so what it wrote is set aside as `.failed` and never left under a passing name.
pub struct Saving {
    /// the copy's name, as it is first tried
    pub name: String,
    done: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    result: Receiver<Done>,
}

impl Saving {
    pub fn start(ask: Ask, wake: Sender<Event>) -> Saving {
        let (done, stop) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
        let (tell, result) = mpsc::channel();
        let name = name(&ask, 1);
        let (so_far, stopped) = (done.clone(), stop.clone());
        let worker = std::thread::Builder::new()
            .name("saving".into())
            .spawn(move || {
                let _ = tell.send(save(&ask, &so_far, &stopped, (replace_refused, files::trash)));
                let _ = wake.send(Event::Wake);
            })
            .ok();
        Saving { name, done, stop, worker, result }
    }

    /// how far the copy has got, 0 to 1.
    pub fn part(&self) -> f32 {
        self.done.load(Ordering::Relaxed) as f32 / 1e6
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// what came of it, once it has ended.
    pub fn take(&mut self) -> Option<Done> {
        self.result.try_recv().ok()
    }
}

impl Drop for Saving {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// why a copy cannot take the place of the clip at `path`, known before anything is written: the clip could not go
/// to a bin, where it can be put back from (a network volume has none). None when it can.
pub fn replace_refused(path: &Path) -> Option<String> {
    if browse::volume_of(path) == Volume::Network {
        return Some(format!("a network volume has no {BIN}"));
    }
    files::refusal(path)
}

/// the copy's `n`th name to try: the clip's name, what it keeps of it, and a number from the second on.
/// `wall.mov` trimmed is `wall_trim.mov`, then `wall_trim 2.mov`; without sound `wall_nosound.mov`, both `wall_trim_nosound.mov`.
fn name(ask: &Ask, n: usize) -> String {
    let stem = ask.path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    let ext = ask.path.extension().map_or_else(String::new, |e| format!(".{}", e.to_string_lossy()));
    let kept = match (ask.trim, ask.sound) {
        (true, true) => "_trim",
        (true, false) => "_trim_nosound",
        (false, false) => "_nosound",
        (false, true) => "_copy",
    };
    let number = if n > 1 { format!(" {n}") } else { String::new() };
    format!("{stem}{kept}{number}{ext}")
}

/// the name the user sees of a path
fn shown(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// the groups of a clip that a copy of its frames `first` to `last` holds, counted as the copy counts: from 0 at
/// `first`, each cut to the copy's length. a group wholly outside it is left out.
fn carried(groups: &[Group], (first, last): (usize, usize)) -> Vec<Group> {
    groups.iter().filter(|g| g.frames.0 <= last && g.frames.1 >= first).map(|g| Group { frames: (g.frames.0.max(first) - first, g.frames.1.min(last) - first), marks: g.marks.clone() }).collect()
}

/// logs the copy at `file`, cut as `cut` is, on its clip's notes, then gives it the clip's groups on its frames as its
/// own notes, or none: in the clip's place, the groups that were on its name go. the log goes first, so the groups a
/// copy in the clip's place replaces are kept in eyepiece.yaml.previous. what to add to what is said when either
/// could not be kept.
fn noted(ask: &Ask, file: &Path, cut: &Cut) -> String {
    let Noting { groups, beside, own } = &ask.notes;
    let clip = notes::Clip::of(&ask.path.to_string_lossy(), &shown(&ask.path));
    let how = if ask.sound { "copy" } else { "copy, no sound" };
    let export = notes::Export { when: SystemTime::now(), file: shown(file), frames: (cut.first, cut.last), how: how.into(), command: None };
    // named for the file now at `file`, whose size and time its notes are made on
    let copy = notes::Clip::of(&file.to_string_lossy(), &shown(file));
    let carried = groups.as_deref().map_or_else(Vec::new, |groups| carried(groups, (cut.first, cut.last)));
    let kept = notes::log_export(&clip, &export, *beside, own.as_deref()).map_err(|why| format!("not logged · {why}"));
    match kept.and_then(|_| notes::save(&copy, &carried, *beside, own.as_deref()).map_err(|why| format!("its notes were not saved · {why}"))) {
        Ok(_) => String::new(),
        Err(why) => format!(" · {why}"),
    }
}

/// gives the copy at `written` the first of its names that no file has, beside the clip.
fn take_name(ask: &Ask, written: &Path) -> Result<PathBuf, String> {
    let folder = ask.path.parent().unwrap_or(Path::new("."));
    for n in 1..10_000 {
        let to = folder.join(name(ask, n));
        match files::rename_new(written, &to) {
            Ok(()) => return Ok(to),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("every name for it is taken".into())
}

/// writes the copy `ask` names, checks it, and gives it its name, or puts it in the clip's place with `trash` taking
/// the clip, unless `refuse` says up front why the clip cannot go (`replace_refused`): then both are kept. `done`
/// follows how far it has got, in millionths, and setting `stop` ends it.
fn save(ask: &Ask, done: &AtomicU64, stop: &AtomicBool, (refuse, trash): (impl Fn(&Path) -> Option<String>, impl Fn(&Path) -> Result<(), String>)) -> Done {
    let folder = ask.path.parent().unwrap_or(Path::new("."));
    let first = name(ask, 1);
    if !ask.sound && !ask.trim && copy::sound_tracks(&ask.path.to_string_lossy()).is_ok_and(|tracks| tracks == 0) {
        return Done { said: "not saved · the clip has no sound to leave out".into(), file: None, replaced: None };
    }
    let refused = ask.replace.then(|| refuse(&ask.path)).flatten();
    // written under a passing name of its own in the same folder, so it takes its final name in one step, and no file
    // with that name is ever half written
    let (passing, file) = match (1..1000).map(|n| folder.join(if n == 1 { format!("{first}.part") } else { format!("{first}.{n}.part") })).find_map(|p| match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
        Ok(file) => Some(Ok((p, file))),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => None,
        Err(e) => Some(Err(format!("{}: {e}", folder.display()))),
    }) {
        Some(Ok(made)) => made,
        Some(Err(why)) => return Done { said: format!("not saved · {why}"), file: None, replaced: None },
        None => return Done { said: "not saved · every passing name for it is taken".into(), file: None, replaced: None },
    };
    let (source, written) = (ask.path.to_string_lossy(), passing.to_string_lossy().into_owned());
    let made = copy::copy(&source, ask.frames, ask.sound, file, &first, done, stop).and_then(|cut| copy::check(&written, &source, &cut, ask.sound, stop).map(|differs| (cut, differs)));
    let (cut, differs) = match made {
        Ok(made) => made,
        Err(why) => {
            // what was written is kept, under a name that says it failed (an earlier failure's goes)
            let failed = folder.join(format!("{first}.failed"));
            let kept = std::fs::rename(&passing, &failed).is_ok();
            let said = if kept { format!("not saved · {why} · what was written is kept as {}", shown(&failed)) } else { format!("not saved · {why}") };
            return Done { said, file: kept.then_some(failed), replaced: None };
        }
    };
    // where a copy that starts at a key frame, or ends where frames stored out of order are whole, is not the trim
    let mut frames = format!("{} frames", cut.frames());
    if ask.trim {
        let (early, late) = (ask.frames.0.saturating_sub(cut.first), cut.last.saturating_sub(ask.frames.1));
        if early > 0 {
            frames += &format!(", from the key frame {early} before the in point");
        }
        if late > 0 {
            frames += &format!(", to {late} past the out point");
        }
    }
    // and what of the clip's tracks it leaves out
    if let Some(left) = &cut.left {
        frames += &format!(" · {left}");
    }
    // in the clip's place: only a copy that is what was asked, and only once the clip is in the bin
    let mut held = refused;
    if ask.replace && held.is_none() {
        held = Some(if let Some(differs) = &differs {
            format!("the copy is not as asked: {differs}")
        } else {
            match trash(&ask.path) {
                Err(why) => format!("the original could not go to the {BIN}: {why}"),
                Ok(()) => match files::rename_new(&passing, &ask.path) {
                    Ok(()) => {
                        let said = format!("replaced {} · {frames} · the original is in the {BIN}", shown(&ask.path)) + &noted(ask, &ask.path, &cut);
                        return Done { said, file: Some(ask.path.clone()), replaced: Some(ask.path.clone()) };
                    }
                    Err(e) => format!("the original is in the {BIN}, and the copy could not take its name: {e}"),
                },
            }
        });
    }
    let named = match take_name(ask, &passing) {
        Ok(named) => named,
        Err(why) => return Done { said: format!("saved, but not named · {why} · it is {}", shown(&passing)), file: Some(passing), replaced: None },
    };
    let said = match (held, differs) {
        (Some(why), _) => format!("not replaced · {why} · saved as {}", shown(&named)),
        (None, Some(differs)) => format!("saved, not as asked · {} · {differs}", shown(&named)),
        (None, None) => format!("saved beside the clip · {} · {frames}", shown(&named)),
    } + &noted(ask, &named, &cut);
    Done { said, file: Some(named), replaced: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup::{Mark, Shape, INKS};
    use crate::media::copy::tests::{pictures, scratch, tidy};
    use std::cell::RefCell;

    /// a copy's log and notes beside its clip, in the case's own folder; `groups` go with it
    fn beside(groups: Option<Vec<Group>>) -> Noting {
        Noting { groups, beside: true, own: None }
    }

    fn ask(dir: &Path, clip: &str, frames: (usize, usize), trim: bool, sound: bool, replace: bool) -> Ask {
        Ask { path: dir.join(clip), frames, trim, sound, replace, notes: beside(None) }
    }

    fn group(frames: (usize, usize), x: f32) -> Group {
        Group { frames, marks: vec![Mark { shape: Shape::Rectangle((x, x), (x + 10.0, x + 20.0)), ink: INKS[0] }] }
    }

    fn notes_of(dir: &Path, clip: &str) -> notes::Read {
        notes::read(&notes::Clip::of(&dir.join(clip).to_string_lossy(), clip), None)
    }

    fn yaml(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(notes::FILE)).unwrap_or_default().replace("\r\n", "\n")
    }

    /// a copy of frames 8 to 13 holds the groups on those frames, counted from 0 at 8 and cut to its 6 frames; a group
    /// wholly before or after it is left out, one that touches it by a frame keeps that frame
    #[test]
    fn a_copy_holds_the_groups_on_its_frames_counted_as_it_counts() {
        let groups = [(0, 7), (0, 8), (5, 9), (9, 12), (12, 20), (13, 13), (14, 20), (0, 30)].map(|frames| group(frames, frames.0 as f32));
        let kept: Vec<(usize, usize)> = carried(&groups, (8, 13)).iter().map(|g| g.frames).collect();
        assert_eq!(kept, [(0, 0), (0, 1), (1, 4), (4, 5), (5, 5), (0, 5)]);
        assert_eq!(carried(&groups, (8, 13))[2].marks, groups[3].marks, "the marks go as they are");
        assert_eq!(carried(&groups, (0, 0)).iter().map(|g| g.frames).collect::<Vec<_>>(), [(0, 0), (0, 0), (0, 0)]);
        assert!(carried(&[], (8, 13)).is_empty());
    }

    /// a saved copy is logged on its clip, beside it: when, its name, the frames it really holds (from the key frame
    /// before the in point), how, and no command. the clip's groups on those frames go with it as its own notes,
    /// counted as it counts and stamped with its own size and time; left out, it has none, and is logged all the same
    #[test]
    fn a_saved_copy_is_logged_and_takes_the_notes_on_its_frames() {
        let dir = scratch("logged", &["av_tc_64.mov"]);
        let groups = vec![group((0, 2), 1.0), group((2, 5), 2.0), group((6, 10), 3.0)];
        let trim = Ask { notes: beside(Some(groups.clone())), ..ask(&dir, "av_tc_64.mov", (5, 7), true, true, false) };
        assert_eq!(saved(&trim).file, Some(dir.join("av_tc_64_trim.mov")));
        let text = yaml(&dir);
        assert!(text.contains("  av_tc_64.mov:\n    exports:\n      - { when: 20") && text.contains("Z, file: av_tc_64_trim.mov, frames: [4, 7], how: copy }\n"), "{text}");
        assert_eq!(notes_of(&dir, "av_tc_64_trim.mov"), notes::Read { groups: vec![group((0, 1), 2.0), group((2, 3), 3.0)], earlier: false, beside: true, said: None });
        assert!(notes_of(&dir, "av_tc_64.mov").groups.is_empty(), "the clip's own notes are not touched");
        // left out: the copy has no entry, and its log line is there beside the first
        let without = Ask { notes: beside(None), ..ask(&dir, "av_tc_64.mov", (4, 7), true, false, false) };
        assert_eq!(saved(&without).file, Some(dir.join("av_tc_64_trim_nosound.mov")));
        let text = yaml(&dir);
        assert!(text.contains("Z, file: av_tc_64_trim_nosound.mov, frames: [4, 7], how: \"copy, no sound\" }\n") && !text.contains("av_tc_64_trim_nosound.mov:"), "{text}");
        tidy(&dir);
    }

    /// a copy that fails, is stopped or has nothing to copy logs nothing and gives no file notes: the folder's file is
    /// not even made
    #[test]
    fn a_copy_that_is_not_saved_logs_nothing() {
        let dir = scratch("unlogged", &["prores_64.mov"]);
        let notes = beside(Some(vec![group((0, 3), 1.0)]));
        let stopped = save(&Ask { notes: notes.clone(), ..ask(&dir, "prores_64.mov", (1, 2), true, true, false) }, &AtomicU64::new(0), &AtomicBool::new(true), (fits, never));
        assert!(stopped.said.starts_with("not saved · stopped"), "{}", stopped.said);
        assert!(saved(&Ask { notes: notes.clone(), ..ask(&dir, "prores_64.mov", (0, 3), false, false, false) }).said.starts_with("not saved"));
        assert!(saved(&Ask { notes, ..ask(&dir, "prores_64.mov", (2, 9), true, true, false) }).said.starts_with("not saved"));
        assert!(!dir.join(notes::FILE).exists(), "{}", yaml(&dir));
        tidy(&dir);
    }

    /// a log that cannot be kept anywhere is said, and the copy stays saved
    #[test]
    fn a_log_that_cannot_be_kept_is_said_and_the_copy_stays() {
        let dir = scratch("nowhere", &["prores_64.mov"]);
        let trim = Ask { notes: Noting { groups: None, beside: false, own: None }, ..ask(&dir, "prores_64.mov", (1, 2), true, true, false) };
        let done = saved(&trim);
        assert_eq!(done.file, Some(dir.join("prores_64_trim.mov")));
        assert_eq!(done.said, "saved beside the clip · prores_64_trim.mov · 2 frames · not logged · the system names no folder for eyepiece's own files");
        tidy(&dir);
    }

    /// a copy in the clip's place is logged on the clip as it was named, and its notes are the groups shifted to its
    /// frames: the frames changed. the groups it replaced are in eyepiece.yaml.previous. left out, it has none
    #[test]
    fn a_copy_in_the_clips_place_has_the_shifted_groups_or_none() {
        let dir = scratch("replace-notes", &["prores_64.mov"]);
        let bin = dir.join("bin");
        std::fs::create_dir(&bin).unwrap();
        let to_bin = |file: &Path| std::fs::rename(file, bin.join(file.file_name().unwrap())).map_err(|e| e.to_string());
        let clip = notes::Clip::of(&dir.join("prores_64.mov").to_string_lossy(), "prores_64.mov");
        let groups = vec![group((0, 0), 1.0), group((1, 3), 2.0)];
        notes::save(&clip, &groups, true, None).unwrap();
        let replace = Ask { notes: beside(Some(groups.clone())), ..ask(&dir, "prores_64.mov", (2, 3), true, true, true) };
        let done = save(&replace, &AtomicU64::new(0), &AtomicBool::new(false), (fits, to_bin));
        assert_eq!(done.replaced, Some(dir.join("prores_64.mov")), "{}", done.said);
        assert_eq!(notes_of(&dir, "prores_64.mov"), notes::Read { groups: vec![group((0, 1), 2.0)], earlier: false, beside: true, said: None });
        assert!(yaml(&dir).contains("Z, file: prores_64.mov, frames: [2, 3], how: copy }\n"), "{}", yaml(&dir));
        let previous = std::fs::read_to_string(dir.join(format!("{}.previous", notes::FILE))).unwrap();
        assert!(previous.contains("- frames: [1, 3]") && previous.contains("exports"), "{previous}");
        std::fs::remove_file(bin.join("prores_64.mov")).unwrap();
        // left out: the groups on the clip's name go, as they were on frames it no longer has
        let replace = Ask { notes: beside(None), ..ask(&dir, "prores_64.mov", (1, 1), true, true, true) };
        let done = save(&replace, &AtomicU64::new(0), &AtomicBool::new(false), (fits, to_bin));
        assert_eq!(done.replaced, Some(dir.join("prores_64.mov")), "{}", done.said);
        assert!(notes_of(&dir, "prores_64.mov").groups.is_empty() && yaml(&dir).contains("frames: [1, 1], how: copy }\n"), "{}", yaml(&dir));
        std::fs::remove_file(bin.join("prores_64.mov")).unwrap();
        std::fs::remove_dir(&bin).unwrap();
        tidy(&dir);
    }

    fn never(_: &Path) -> Result<(), String> {
        panic!("nothing is to go to the bin")
    }

    /// a bin that takes the clip
    fn fits(_: &Path) -> Option<String> {
        None
    }

    fn saved(ask: &Ask) -> Done {
        save(ask, &AtomicU64::new(0), &AtomicBool::new(false), (fits, never))
    }

    #[test]
    fn a_copy_is_named_for_what_it_keeps() {
        let clip = Path::new("shot.v2.mov");
        let names: Vec<String> = [(true, true), (true, false), (false, false)].into_iter().map(|(trim, sound)| name(&Ask { path: clip.into(), frames: (0, 0), trim, sound, replace: false, notes: beside(None) }, 1)).collect();
        assert_eq!(names, ["shot.v2_trim.mov", "shot.v2_trim_nosound.mov", "shot.v2_nosound.mov"]);
        assert_eq!(name(&Ask { path: clip.into(), frames: (0, 0), trim: true, sound: true, replace: false, notes: beside(None) }, 3), "shot.v2_trim 3.mov");
    }

    /// a copy goes beside the clip, and never over a file there: the next gets a number. nothing is left under a
    /// passing name
    #[test]
    fn a_copy_never_writes_over_a_file() {
        let dir = scratch("names", &["prores_64.mov", "av_tc_64.mov"]);
        std::fs::write(dir.join("prores_64_trim.mov"), b"someone's").unwrap();
        let trim = ask(&dir, "prores_64.mov", (1, 2), true, true, false);
        let done = saved(&trim);
        assert_eq!(done, Done { said: "saved beside the clip · prores_64_trim 2.mov · 2 frames".into(), file: Some(dir.join("prores_64_trim 2.mov")), replaced: None });
        assert_eq!(saved(&trim).file, Some(dir.join("prores_64_trim 3.mov")));
        assert_eq!(std::fs::read(dir.join("prores_64_trim.mov")).unwrap(), b"someone's");
        assert_eq!(saved(&ask(&dir, "av_tc_64.mov", (0, 11), false, false, false)).file, Some(dir.join("av_tc_64_nosound.mov")));
        assert_eq!(saved(&ask(&dir, "av_tc_64.mov", (4, 7), true, false, false)).file, Some(dir.join("av_tc_64_trim_nosound.mov")));
        // a trim that is not where a copy can start says where it does start
        assert_eq!(saved(&ask(&dir, "av_tc_64.mov", (5, 7), true, false, false)).said, "saved beside the clip · av_tc_64_trim_nosound 2.mov · 4 frames, from the key frame 1 before the in point");
        // a clip with no sound has none to leave out
        assert_eq!(saved(&ask(&dir, "prores_64.mov", (0, 3), false, false, false)), Done { said: "not saved · the clip has no sound to leave out".into(), file: None, replaced: None });
        let left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".part")).collect();
        assert!(left.is_empty(), "{left:?}");
        tidy(&dir);
    }

    /// a copy that fails is kept as <name>.failed, never deleted, and nothing takes the copy's name; a stop is one
    #[test]
    fn a_copy_that_fails_is_set_aside() {
        let dir = scratch("failed", &["prores_64.mov"]);
        let done = save(&ask(&dir, "prores_64.mov", (1, 2), true, true, false), &AtomicU64::new(0), &AtomicBool::new(true), (fits, never));
        assert_eq!(done.file, Some(dir.join("prores_64_trim.mov.failed")));
        assert!(done.said.starts_with("not saved · stopped · what was written is kept as prores_64_trim.mov.failed"), "{}", done.said);
        assert!(!dir.join("prores_64_trim.mov").exists());
        tidy(&dir);
    }

    /// in the clip's place: the clip goes to the bin first, then the copy takes its name. when the bin will not take
    /// it, both are kept and that is said
    #[test]
    fn a_copy_takes_the_clips_place_only_once_the_clip_is_in_the_bin() {
        let dir = scratch("replace", &["prores_64.mov"]);
        let bin = dir.join("bin");
        std::fs::create_dir(&bin).unwrap();
        let clip = dir.join("prores_64.mov");
        let before = pictures(&clip.to_string_lossy());
        let binned = RefCell::new(Vec::new());
        let to_bin = |file: &Path| {
            binned.borrow_mut().push(file.to_path_buf());
            std::fs::rename(file, bin.join(file.file_name().unwrap())).map_err(|e| e.to_string())
        };
        let done = save(&ask(&dir, "prores_64.mov", (1, 2), true, true, true), &AtomicU64::new(0), &AtomicBool::new(false), (fits, to_bin));
        assert_eq!(done, Done { said: format!("replaced prores_64.mov · 2 frames · the original is in the {BIN}"), file: Some(clip.clone()), replaced: Some(clip.clone()) });
        assert_eq!((binned.borrow().as_slice(), pictures(&clip.to_string_lossy())), ([clip.clone()].as_slice(), before[1..=2].to_vec()));
        assert_eq!(pictures(&bin.join("prores_64.mov").to_string_lossy()), before);
        // the bin refuses: the clip stays, and the copy is beside it
        let refuse = |_: &Path| Err("no bin here".to_string());
        let done = save(&ask(&dir, "prores_64.mov", (0, 0), true, true, true), &AtomicU64::new(0), &AtomicBool::new(false), (fits, refuse));
        assert_eq!(done.said, format!("not replaced · the original could not go to the {BIN}: no bin here · saved as prores_64_trim.mov"));
        assert_eq!((pictures(&clip.to_string_lossy()).len(), pictures(&dir.join("prores_64_trim.mov").to_string_lossy()).len()), (2, 1));
        std::fs::remove_file(bin.join("prores_64.mov")).unwrap();
        std::fs::remove_dir(&bin).unwrap();
        tidy(&dir);
    }

    /// a clip whose bin cannot be shown to take it is never asked to go there: the copy is saved beside it, both are
    /// kept, and the one line says why. what it leaves out of the clip's tracks is said too
    #[test]
    fn a_clip_the_bin_cannot_take_is_refused_up_front() {
        let dir = scratch("refused", &["av_tc_64.mov"]);
        let cannot = |_: &Path| Some("its drive's recycle bin cannot be confirmed to take it".to_string());
        let done = save(&ask(&dir, "av_tc_64.mov", (4, 7), true, true, true), &AtomicU64::new(0), &AtomicBool::new(false), (cannot, never));
        assert_eq!(done, Done { said: "not replaced · its drive's recycle bin cannot be confirmed to take it · saved as av_tc_64_trim.mov".into(), file: Some(dir.join("av_tc_64_trim.mov")), replaced: None });
        assert_eq!((pictures(&dir.join("av_tc_64.mov").to_string_lossy()).len(), pictures(&dir.join("av_tc_64_trim.mov").to_string_lossy()).len()), (12, 4));
        tidy(&dir);
    }
}
