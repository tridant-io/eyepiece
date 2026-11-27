//! saving a copy of a clip for the user (adr 0009): its trim, or the clip without its sound, beside it under a name
//! of its own and never over another file; or, asked, in its place, the clip itself to the recycle bin (the trash on a
//! mac), never deleted. the copy is written under a passing name, checked, and only then takes its own; one that
//! fails is kept as `<name>.failed`. it runs on a thread of its own, which `Saving` follows.

use crate::browse::{self, Volume};
use crate::event::Event;
use crate::media::copy;
use crate::sys::files;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

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
                let _ = tell.send(save(&ask, &so_far, &stopped, files::trash));
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
/// the clip. `done` follows how far it has got, in millionths, and setting `stop` ends it.
fn save(ask: &Ask, done: &AtomicU64, stop: &AtomicBool, trash: impl Fn(&Path) -> Result<(), String>) -> Done {
    let folder = ask.path.parent().unwrap_or(Path::new("."));
    let first = name(ask, 1);
    if !ask.sound && !ask.trim && copy::sound_tracks(&ask.path.to_string_lossy()).is_ok_and(|tracks| tracks == 0) {
        return Done { said: "not saved · the clip has no sound to leave out".into(), file: None, replaced: None };
    }
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
    // in the clip's place: only a copy that is what was asked, and only once the clip is in the bin
    let mut held = None;
    if ask.replace {
        held = Some(if let Some(differs) = &differs {
            format!("the copy is not as asked: {differs}")
        } else if browse::volume_of(&ask.path) == Volume::Network {
            format!("a network volume has no {BIN}")
        } else {
            match trash(&ask.path) {
                Err(why) => format!("the original could not go to the {BIN}: {why}"),
                Ok(()) => match files::rename_new(&passing, &ask.path) {
                    Ok(()) => return Done { said: format!("replaced {} · {frames} · the original is in the {BIN}", shown(&ask.path)), file: Some(ask.path.clone()), replaced: Some(ask.path.clone()) },
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
    };
    Done { said, file: Some(named), replaced: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::copy::tests::{pictures, scratch, tidy};
    use std::cell::RefCell;

    fn ask(dir: &Path, clip: &str, frames: (usize, usize), trim: bool, sound: bool, replace: bool) -> Ask {
        Ask { path: dir.join(clip), frames, trim, sound, replace }
    }

    fn never(_: &Path) -> Result<(), String> {
        panic!("nothing is to go to the bin")
    }

    fn saved(ask: &Ask) -> Done {
        save(ask, &AtomicU64::new(0), &AtomicBool::new(false), never)
    }

    #[test]
    fn a_copy_is_named_for_what_it_keeps() {
        let clip = Path::new("shot.v2.mov");
        let names: Vec<String> = [(true, true), (true, false), (false, false)].into_iter().map(|(trim, sound)| name(&Ask { path: clip.into(), frames: (0, 0), trim, sound, replace: false }, 1)).collect();
        assert_eq!(names, ["shot.v2_trim.mov", "shot.v2_trim_nosound.mov", "shot.v2_nosound.mov"]);
        assert_eq!(name(&Ask { path: clip.into(), frames: (0, 0), trim: true, sound: true, replace: false }, 3), "shot.v2_trim 3.mov");
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
        let done = save(&ask(&dir, "prores_64.mov", (1, 2), true, true, false), &AtomicU64::new(0), &AtomicBool::new(true), never);
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
        let done = save(&ask(&dir, "prores_64.mov", (1, 2), true, true, true), &AtomicU64::new(0), &AtomicBool::new(false), to_bin);
        assert_eq!(done, Done { said: format!("replaced prores_64.mov · 2 frames · the original is in the {BIN}"), file: Some(clip.clone()), replaced: Some(clip.clone()) });
        assert_eq!((binned.borrow().as_slice(), pictures(&clip.to_string_lossy())), ([clip.clone()].as_slice(), before[1..=2].to_vec()));
        assert_eq!(pictures(&bin.join("prores_64.mov").to_string_lossy()), before);
        // the bin refuses: the clip stays, and the copy is beside it
        let refuse = |_: &Path| Err("no bin here".to_string());
        let done = save(&ask(&dir, "prores_64.mov", (0, 0), true, true, true), &AtomicU64::new(0), &AtomicBool::new(false), refuse);
        assert_eq!(done.said, format!("not replaced · the original could not go to the {BIN}: no bin here · saved as prores_64_trim.mov"));
        assert_eq!((pictures(&clip.to_string_lossy()).len(), pictures(&dir.join("prores_64_trim.mov").to_string_lossy()).len()), (2, 1));
        std::fs::remove_file(bin.join("prores_64.mov")).unwrap();
        std::fs::remove_dir(&bin).unwrap();
        tidy(&dir);
    }
}
