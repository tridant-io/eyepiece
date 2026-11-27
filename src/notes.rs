//! notes: what eyepiece keeps on a clip between runs (adr 0010). each folder gets one `eyepiece.yaml` that holds, for
//! every clip in it, the clip's groups of marks and a log of the exports made from it. a person can read it and send
//! it on with the folder. it must never lose a note: it is read again before each save and only the clip saved is
//! changed in it; it is written whole to a file beside it, flushed, and swapped in, the version it replaces kept as
//! `eyepiece.yaml.previous`; and a file that cannot be read is never written over. notes that cannot be saved beside
//! their clip (the setting off, a folder that cannot be written, a file that cannot be read) go to eyepiece's own
//! folder, a file there for each folder of clips, kept with the same care.
//!
//! yaml-rust2 reads the file. it writes a list or a mapping only down the page, a number a line, so the text is
//! written here, and each save is read back by yaml-rust2 and compared before it replaces anything.

use crate::markup::{Group, Mark, Shape, INKS};
use std::collections::HashSet;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use std::{fmt::Write as _, fs};
use yaml_rust2::yaml::Hash;
use yaml_rust2::{Yaml, YamlLoader};

/// the file's name, in every folder
pub const FILE: &str = "eyepiece.yaml";
/// the first line of a folder's file
const HEADER: &str = "eyepiece's notes for the clips in this folder";
/// the version of the file's layout this build reads and writes
const VERSION: i64 = 1;
/// the inks' names in the file, as `INKS` has them
const INK_NAMES: [&str; 10] = ["red", "orange", "yellow", "green", "blue", "indigo", "violet", "white", "black", "gray"];
/// how long a change waits to be saved, for the changes that follow it
const QUIET: Duration = Duration::from_millis(500);
/// how long the window waits, as it closes, for the last save: a share that has gone away must not hold it for good
const LAST_SAVE: Duration = Duration::from_secs(10);

/// the clip a set of notes is on
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Clip {
    pub folder: PathBuf,
    /// what the file calls it: the clip's file name; a numbered run's with a # for each digit of its numbers
    /// (name.####.exr), so that frames rendered later leave its notes where they are
    pub name: String,
    /// the number of its first frame as the file counts: a numbered run's first file's, and 0 for anything else. a
    /// group's frames are written counted so, as the run's files are numbered
    pub first: usize,
    /// the file whose size and time say which version the notes were made on: the clip's own, a numbered run's first
    pub stamped: PathBuf,
}

impl Clip {
    /// the clip at `path`, which the player shows as `shown`: for a numbered run that is its name with its range,
    /// name.[0001-0240].exr, whichever of its files it was opened by
    pub fn of(path: &str, shown: &str) -> Clip {
        let path = Path::new(path);
        let folder = path.parent().filter(|folder| !folder.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
        let file = path.file_name().map_or_else(|| shown.to_string(), |name| name.to_string_lossy().into_owned());
        // a run's name is not its file's: what stands between its last brackets is its first and last numbers
        let run = (file != shown).then(|| {
            let (before, rest) = shown.rsplit_once('[')?;
            let (range, after) = rest.split_once(']')?;
            let (first, last) = range.split_once('-')?;
            let digits = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
            (digits(first) && digits(last)).then_some(())?;
            Some((format!("{before}{}{after}", "#".repeat(first.len())), first.parse().ok()?, folder.join(format!("{before}{first}{after}"))))
        });
        match run.flatten() {
            Some((name, first, stamped)) => Clip { folder, name, first, stamped },
            None => Clip { stamped: folder.join(&file), folder, name: file, first: 0 },
        }
    }
}

/// a clip's notes as they were read
#[derive(Debug, Default, PartialEq)]
pub struct Read {
    pub groups: Vec<Group>,
    /// they were made on another version of the clip's file: its size or its time was another then
    pub earlier: bool,
    /// the folder's file can be read, or is not there: the clip's notes may be saved in it
    pub beside: bool,
    /// what there is to say of it: a file that cannot be read, and where
    pub said: Option<String>,
}

/// reads a clip's notes: from its folder's file, and with them those kept in eyepiece's own folder (`own`), where a
/// save that could not be made beside the clip left them. never writes.
pub fn read(clip: &Clip, own: Option<&Path>) -> Read {
    let stamp = stamp_of(&clip.stamped);
    // a clip's notes in one file: none, or what they are and whether they were made on another version of its file
    let notes_in = |file: &Path| -> Result<Option<(Vec<Group>, bool)>, String> {
        let Some(top) = load(file)? else { return Ok(None) };
        let Some(entry) = entry_of(&top, &clip.name) else { return Ok(None) };
        let made_on = entry.get(&key("file")).and_then(stamp_from);
        let groups = groups_of(entry, clip)?;
        // an entry may be there for its exports alone
        Ok((!groups.is_empty()).then(|| (groups, made_on.is_some() && stamp.is_some() && made_on != stamp)))
    };
    let beside = notes_in(&clip.folder.join(FILE));
    let kept = own.map_or(Ok(None), |own| notes_in(&own_file(own, &clip.folder)));
    let mut said = beside.as_ref().err().map(|why| format!("{FILE} {why} · notes are kept in eyepiece's own folder until it can"));
    if let Err(why) = &kept {
        said = Some(format!("eyepiece's own notes for this folder cannot be read · {why}"));
    }
    let can_beside = beside.is_ok();
    // both are shown, the folder's first, a group that is in both once: a save beside the clip then holds them all,
    // and never replaces a note this window has not shown
    let (mut groups, mut earlier) = beside.ok().flatten().unwrap_or_default();
    for (group, made_earlier) in kept.ok().flatten().into_iter().flat_map(|(groups, earlier)| groups.into_iter().map(move |group| (group, earlier))) {
        if !groups.contains(&group) {
            groups.push(group);
            earlier |= made_earlier;
        }
    }
    Read { groups, earlier, beside: can_beside, said }
}

/// where a save put a clip's notes
#[derive(Debug, PartialEq)]
pub enum Kept {
    Beside,
    /// in eyepiece's own folder; and why not beside the clip, when that was asked for
    Own(Option<String>),
}

/// saves a clip's notes: in its folder's file when `beside` says so and that can be done, else in eyepiece's own
/// folder. with no groups the clip's notes are taken out of the file, and a file left with nothing in it goes aside
/// as its `.previous`. notes saved beside the clip are taken out of eyepiece's own folder, where an earlier save may
/// have had to leave them.
pub fn save(clip: &Clip, groups: &[Group], beside: bool, own: Option<&Path>) -> Result<Kept, String> {
    let stamp = stamp_of(&clip.stamped);
    let kept = keep(clip, beside, own, &|top| set_groups(top, clip, groups, stamp.as_ref()))?;
    if let Some(left) = own.map(|own| own_file(own, &clip.folder)).filter(|left| kept == Kept::Beside && left.exists()) {
        // a copy that cannot be read is not read for the clip's notes either: the save itself stands
        if let Err(why) = put(&left, &own_header(&clip.folder), &|file, bytes| file.write_all(bytes), &|top| set_groups(top, clip, &[], None)) {
            eprintln!("eyepiece: {}: {why}", left.display());
        }
    }
    Ok(kept)
}

/// an export made from a clip, for the log its entry keeps: when, the file written (its name, beside the clip, or its
/// whole path elsewhere), the clip's frames it holds, how it was made in a word, and the command that made it, when
/// one did (a copy runs none)
pub struct Export {
    pub when: SystemTime,
    pub file: String,
    pub frames: (usize, usize),
    pub how: String,
    pub command: Option<String>,
}

/// adds an export to its clip's log, with the care a save takes, off the drawing thread.
pub fn log_export(clip: &Clip, export: &Export, beside: bool, own: Option<&Path>) -> Result<Kept, String> {
    let line: Hash = [
        ("when", Some(text(&utc(export.when)))),
        ("file", Some(text(&export.file))),
        ("frames", Some(Yaml::Array(vec![Yaml::Integer((export.frames.0 + clip.first) as i64), Yaml::Integer((export.frames.1 + clip.first) as i64)]))),
        ("how", Some(text(&export.how))),
        ("command", export.command.as_deref().map(text)),
    ]
    .into_iter()
    .filter_map(|(name, value)| Some((key(name), value?)))
    .collect();
    keep(clip, beside, own, &|top| {
        let entry = entry_mut(top, &clip.name)?;
        match entry.entry(key("exports")).or_insert_with(|| Yaml::Array(Vec::new())) {
            Yaml::Array(log) => log.push(Yaml::Hash(line.clone())),
            _ => return Err(format!("the exports of {} are not a list", clip.name)),
        }
        Ok(())
    })
}

/// makes a change to a clip's entry where it is kept: in its folder's file, or, when that is not wanted or cannot be
/// done, in eyepiece's own folder.
fn keep(clip: &Clip, beside: bool, own: Option<&Path>, change: &Change<'_>) -> Result<Kept, String> {
    let plain: &Writer<'_> = &|file, bytes| file.write_all(bytes);
    let mut why_not = None;
    if beside {
        match put(&clip.folder.join(FILE), HEADER, plain, change) {
            Ok(()) => return Ok(Kept::Beside),
            Err(why) => why_not = Some(why),
        }
    }
    let own = own.ok_or_else(|| why_not.clone().unwrap_or_else(|| "the system names no folder for eyepiece's own files".to_string()))?;
    let kept = own_file(own, &clip.folder);
    fs::create_dir_all(kept.parent().unwrap_or(own)).map_err(|e| format!("{}: {e}", kept.display()))?;
    put(&kept, &own_header(&clip.folder), plain, change)?;
    Ok(Kept::Own(why_not))
}

/// the file in eyepiece's own folder that holds the notes for one folder of clips: named for the folder, and for its
/// whole path by a number made of it, so two folders of one name do not share a file
pub(crate) fn own_file(own: &Path, folder: &Path) -> PathBuf {
    let path = folder.to_string_lossy();
    // windows does not tell D:\Shots from d:\shots
    let path = if cfg!(windows) { path.to_lowercase() } else { path.into_owned() };
    // fnv-1a: std's own hasher may change between versions of rust, and this name must not
    let number = path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3));
    let name = folder.file_name().map_or("root".into(), |name| name.to_string_lossy());
    own.join("notes").join(format!("{name}-{number:016x}.yaml"))
}

fn own_header(folder: &Path) -> String {
    format!("eyepiece's notes for the clips in {}, kept here and not beside them", folder.display())
}

/// where the version a save replaces is kept
fn previous_of(file: &Path) -> PathBuf {
    let mut name = file.file_name().unwrap_or_default().to_os_string();
    name.push(".previous");
    file.with_file_name(name)
}

/// writes a file's bytes; a test puts one in that fails
type Writer<'a> = dyn Fn(&mut fs::File, &[u8]) -> io::Result<()> + 'a;
/// a change to a notes file's top mapping
type Change<'a> = dyn Fn(&mut Hash) -> Result<(), String> + 'a;

/// one save at a time in this program: the keeper's and an export's log may want the same file
static SAVING: Mutex<()> = Mutex::new(());

/// changes a notes file, as safely as that can be done.
fn put(file: &Path, header: &str, write: &Writer<'_>, change: &Change<'_>) -> Result<(), String> {
    let _one = SAVING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    put_unlocked(file, header, write, change)
}

/// the file is read again and `change` made to what it holds now, so what others saved in it meanwhile stays. the new
/// text is written whole to a file of its own beside it, flushed to the disk, and read back; only then is it swapped
/// in, in one step, and the version it replaces kept as `.previous`. a file that cannot be read is an error, and is
/// left as it is. a file that changed between the read and the swap is read again. a change that changes nothing
/// writes nothing; one that leaves nothing to keep moves the file aside as its `.previous` rather than delete it.
fn put_unlocked(file: &Path, header: &str, write: &Writer<'_>, change: &Change<'_>) -> Result<(), String> {
    let named = |e: io::Error| format!("{}: {e}", file.display());
    // the file's size and time: by them a save from elsewhere is known
    let seen = |file: &Path| fs::metadata(file).ok().map(|m| (m.len(), m.modified().ok()));
    for _ in 0..4 {
        let before = seen(file);
        let was = load(file)?;
        // a file that is there and was not read is never replaced
        if before.is_some() != was.is_some() {
            continue;
        }
        let mut top = was.clone().unwrap_or_default();
        change(&mut top)?;
        if Some(&top) == was.as_ref() || (was.is_none() && nothing_in(&top)) {
            return Ok(());
        }
        if nothing_in(&top) {
            return fs::rename(file, previous_of(file)).map_err(named);
        }
        let text = written(&top, header);
        if parse(text.as_bytes()).ok().as_ref() != Some(&top) {
            return Err(format!("{FILE} would not read back as it was written, so nothing was saved"));
        }
        // a name no other save has, here or on another machine that saves to the same folder
        let unlike = std::hash::BuildHasher::hash_one(&std::collections::hash_map::RandomState::new(), std::process::id());
        let temp = file.with_file_name(format!("{}.{unlike:016x}.tmp", file.file_name().unwrap_or_default().to_string_lossy()));
        let wrote = (|| {
            let mut out = fs::File::create_new(&temp)?;
            write(&mut out, text.as_bytes())?;
            // on the disk before the swap, so not even a power cut leaves half a file
            out.sync_all()?;
            drop(out);
            // and what is on the disk is what was meant: a share may take a write and keep less
            if fs::read(&temp)? != text.as_bytes() {
                return Err(io::Error::other("what was written did not read back whole"));
            }
            if seen(file) != before {
                return Ok(false);
            }
            crate::sys::swap_in(&temp, file, was.is_some().then(|| previous_of(file)).as_deref())?;
            Ok(true)
        })();
        if !matches!(wrote, Ok(true)) {
            // the swap did not take it: it is eyepiece's own file of a moment ago, and goes
            if let Err(e) = fs::remove_file(&temp).or_else(|e| if e.kind() == io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
                eprintln!("eyepiece: {} was left behind: {e}", temp.display());
            }
        }
        match wrote {
            Ok(true) => return Ok(()),
            // another save came between the read and the swap: read again
            Ok(false) => continue,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(named(e)),
        }
    }
    Err(format!("{}: it kept changing while eyepiece saved", file.display()))
}

/// a file with nothing to keep: no clip, and nothing this build does not know
fn nothing_in(top: &Hash) -> bool {
    top.iter().all(|(name, value)| *name == key("version") || (*name == key("clips") && value.as_hash().is_some_and(|clips| clips.is_empty())))
}

/// a notes file's top mapping; none when there is no file. Err says why it cannot be read, and where when that is
/// known.
fn load(file: &Path) -> Result<Option<Hash>, String> {
    match fs::read(file) {
        Ok(bytes) => parse(&bytes).map(Some),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot be read: {e}")),
    }
}

fn parse(bytes: &[u8]) -> Result<Hash, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("cannot be read at line {}: it is not utf-8 there", 1 + bytes[..e.valid_up_to()].iter().filter(|b| **b == b'\n').count()))?;
    // notepad may start the file with a byte order mark
    let mut documents = YamlLoader::load_from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("cannot be read at line {}: {}", e.marker().line(), e.info()))?;
    let top = match (documents.len(), documents.pop()) {
        // nothing, or comments alone
        (0, _) | (1, Some(Yaml::Null)) => return Ok(Hash::new()),
        (1, Some(Yaml::Hash(top))) => top,
        (1, _) => return Err("cannot be read: it is not a mapping of version and clips".to_string()),
        _ => return Err("cannot be read: it holds more than one document".to_string()),
    };
    match top.get(&key("version")) {
        None | Some(Yaml::Integer(VERSION)) => {}
        Some(other) => return Err(format!("cannot be read: it is version {}, and this eyepiece reads version {VERSION}", scalar(other).unwrap_or_else(|| "?".into()))),
    }
    match top.get(&key("clips")) {
        None | Some(Yaml::Null) => {}
        Some(Yaml::Hash(clips)) if clips.values().all(|entry| entry.as_hash().is_some()) => {}
        Some(_) => return Err("cannot be read: its clips are not a mapping of each clip's name to its notes".to_string()),
    }
    // what could not be written back as it is (a mapping for a name, a name used before it is given) is not read
    if !top.iter().all(|(name, value)| scalar(name).is_some() && writable(value)) {
        return Err("cannot be read: it holds yaml eyepiece does not write".to_string());
    }
    Ok(top)
}

fn key(name: &str) -> Yaml {
    Yaml::String(name.to_string())
}

fn text(words: &str) -> Yaml {
    Yaml::String(words.to_string())
}

/// a clip's entry in a file's top mapping
fn entry_of<'a>(top: &'a Hash, name: &str) -> Option<&'a Hash> {
    top.get(&key("clips"))?.as_hash()?.get(&key(name))?.as_hash()
}

/// a clip's entry to change, made when there is none
fn entry_mut<'a>(top: &'a mut Hash, name: &str) -> Result<&'a mut Hash, String> {
    if !top.contains_key(&key("version")) {
        top.insert(key("version"), Yaml::Integer(VERSION));
        top.to_front(&key("version"));
    }
    let clips = top.entry(key("clips")).or_insert_with(|| Yaml::Hash(Hash::new()));
    if clips.is_null() {
        *clips = Yaml::Hash(Hash::new());
    }
    let clips = clips.as_mut_hash().ok_or("its clips are not a mapping")?;
    clips.entry(key(name)).or_insert_with(|| Yaml::Hash(Hash::new())).as_mut_hash().ok_or_else(|| format!("the entry of {name} is not a mapping"))
}

/// puts a clip's groups in a file's top mapping, with the size and time of the file they were made on; with none,
/// takes them out, and the clip's entry with them when nothing else is in it. notes that are there and cannot be
/// read are not replaced.
fn set_groups(top: &mut Hash, clip: &Clip, groups: &[Group], stamp: Option<&Stamp>) -> Result<(), String> {
    if let Some(entry) = entry_of(top, &clip.name) {
        groups_of(entry, clip)?;
    } else if groups.is_empty() {
        return Ok(());
    }
    let entry = entry_mut(top, &clip.name)?;
    // the file and the groups first, then what else the entry holds, as it was
    let rest: Vec<(Yaml, Yaml)> = entry.iter().filter(|(name, _)| **name != key("file") && **name != key("groups")).map(|(name, value)| (name.clone(), value.clone())).collect();
    entry.clear();
    if !groups.is_empty() {
        if let Some((bytes, modified)) = stamp {
            entry.insert(key("file"), Yaml::Hash([(key("bytes"), Yaml::Integer(*bytes as i64)), (key("modified"), text(modified))].into_iter().collect()));
        }
        entry.insert(key("groups"), Yaml::Array(groups.iter().map(|group| group_yaml(group, clip.first)).collect()));
    }
    entry.extend(rest);
    if entry.is_empty() {
        if let Some(clips) = top.get_mut(&key("clips")).and_then(Yaml::as_mut_hash) {
            clips.remove(&key(&clip.name));
        }
    }
    Ok(())
}

/// a file's size and when it was written, to the second, as the file has them
type Stamp = (u64, String);

/// the clip's file as its folder lists it now: the file itself is not opened (adr 0004)
fn stamp_of(file: &Path) -> Option<Stamp> {
    crate::media::watch::listed_stamp(file).map(|(bytes, modified)| (bytes, utc(modified)))
}

fn stamp_from(file: &Yaml) -> Option<Stamp> {
    let file = file.as_hash()?;
    Some((file.get(&key("bytes"))?.as_i64()? as u64, file.get(&key("modified"))?.as_str()?.to_string()))
}

/// a time as the file writes it: 2026-11-26T19:40:12Z (howard hinnant's days-to-civil)
fn utc(time: SystemTime) -> String {
    let seconds = time.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |since| since.as_secs()) as i64;
    let z = seconds.div_euclid(86_400) + 719_468;
    let (era, day_of_era) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let (day, month) = (day_of_year - (153 * shifted + 2) / 5 + 1, if shifted < 10 { shifted + 3 } else { shifted - 9 });
    let year = year_of_era + era * 400 + (month <= 2) as i64;
    let of_day = seconds.rem_euclid(86_400);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", of_day / 3600, of_day / 60 % 60, of_day % 60)
}

/// a number of the file's pixels, kept to a hundredth of one
fn number(value: f32) -> Yaml {
    let value = (value as f64 * 100.0).round() / 100.0;
    if value.fract() == 0.0 { Yaml::Integer(value as i64) } else { Yaml::Real(value.to_string()) }
}

fn numbers(values: impl IntoIterator<Item = f32>) -> Yaml {
    Yaml::Array(values.into_iter().map(number).collect())
}

/// a shape's box, from its upper left corner: left, top, width, height
fn box_of((a, b): (crate::markup::Point, crate::markup::Point)) -> Yaml {
    let (left, top) = (number(a.0.min(b.0)), number(a.1.min(b.1)));
    let side = |near: &Yaml, far: f32| number(far - near.as_f64().or(near.as_i64().map(|n| n as f64)).unwrap_or(0.0) as f32);
    let (width, height) = (side(&left, a.0.max(b.0)), side(&top, a.1.max(b.1)));
    Yaml::Array(vec![left, top, width, height])
}

fn group_yaml(group: &Group, first: usize) -> Yaml {
    let frames = Yaml::Array(vec![Yaml::Integer((group.frames.0 + first) as i64), Yaml::Integer((group.frames.1 + first) as i64)]);
    let marks = group.marks.iter().map(|mark| {
        let ink = INKS.iter().position(|ink| *ink == mark.ink).map_or_else(|| format!("#{:02x}{:02x}{:02x}", mark.ink[0], mark.ink[1], mark.ink[2]), |i| INK_NAMES[i].to_string());
        let mut parts = match &mark.shape {
            Shape::Rectangle(a, b) => vec![("rect", box_of((*a, *b)))],
            Shape::Circle(a, b) => vec![("circle", box_of((*a, *b)))],
            Shape::Pen(points) => vec![("pen", numbers(points.iter().flat_map(|p| [p.0, p.1])))],
            // a size reads as a number with its point: 1.0
            Shape::Text(at, words, size) => vec![("text", text(words)), ("at", numbers([at.0, at.1])), ("size", Yaml::Real(format!("{:?}", (*size as f64 * 100.0).round() / 100.0)))],
        };
        parts.push(("ink", text(&ink)));
        Yaml::Hash(parts.into_iter().map(|(name, value)| (key(name), value)).collect())
    });
    Yaml::Hash([(key("frames"), frames), (key("marks"), Yaml::Array(marks.collect()))].into_iter().collect())
}

/// a clip's groups as its entry has them. Err, naming the clip, when the entry holds what this build does not know:
/// its notes are then not shown, and never replaced.
fn groups_of(entry: &Hash, clip: &Clip) -> Result<Vec<Group>, String> {
    let unread = |what: String| format!("cannot be read: the notes on {} hold {what}", clip.name);
    let groups = match entry.get(&key("groups")) {
        None | Some(Yaml::Null) => return Ok(Vec::new()),
        Some(Yaml::Array(groups)) => groups,
        Some(_) => return Err(unread("groups that are not a list".into())),
    };
    let float = |value: &Yaml| value.as_f64().or(value.as_i64().map(|n| n as f64)).map(|n| n as f32);
    let floats = |value: &Yaml, count: Option<usize>| -> Option<Vec<f32>> {
        let values: Vec<f32> = value.as_vec()?.iter().map(float).collect::<Option<_>>()?;
        (count.is_none_or(|count| values.len() == count) && values.len().is_multiple_of(2)).then_some(values)
    };
    let only = |map: &Hash, names: &[&str]| map.keys().all(|name| name.as_str().is_some_and(|name| names.contains(&name)));
    let mark_of = |mark: &Yaml| -> Option<Mark> {
        let mark = mark.as_hash()?;
        let get = |name: &str| mark.get(&key(name));
        let ink = get("ink")?.as_str()?;
        let ink = INK_NAMES.iter().position(|name| *name == ink).map(|i| INKS[i]).or_else(|| crate::settings::color(ink))?;
        let corners = |at: &Yaml| floats(at, Some(4)).map(|b| ((b[0], b[1]), (b[0] + b[2], b[1] + b[3])));
        let shape = if let Some((a, b)) = get("rect").and_then(corners).filter(|_| only(mark, &["rect", "ink"])) {
            Shape::Rectangle(a, b)
        } else if let Some((a, b)) = get("circle").and_then(corners).filter(|_| only(mark, &["circle", "ink"])) {
            Shape::Circle(a, b)
        } else if let Some(points) = get("pen").and_then(|pen| floats(pen, None)).filter(|_| only(mark, &["pen", "ink"])) {
            Shape::Pen(points.chunks(2).map(|p| (p[0], p[1])).collect())
        } else if only(mark, &["text", "at", "size", "ink"]) {
            let at = floats(get("at")?, Some(2))?;
            Shape::Text((at[0], at[1]), get("text")?.as_str()?.to_string(), get("size").map_or(Some(1.0), float)?)
        } else {
            return None;
        };
        Some(Mark { shape, ink })
    };
    let mut read = Vec::with_capacity(groups.len());
    for (g, group) in groups.iter().enumerate() {
        let group = group.as_hash().filter(|group| only(group, &["frames", "marks"])).ok_or_else(|| unread(format!("a group eyepiece does not know (group {})", g + 1)))?;
        let frames = group.get(&key("frames")).and_then(Yaml::as_vec).map(|frames| frames.iter().map(|f| f.as_i64().and_then(|f| usize::try_from(f).ok()?.checked_sub(clip.first))).collect::<Option<Vec<_>>>());
        let Some(Some(&[first, last])) = frames.as_ref().map(|frames| frames.as_deref()) else { return Err(unread(format!("a group with no frames of this clip's (group {})", g + 1))) };
        if first > last {
            return Err(unread(format!("a group that ends before it starts (group {})", g + 1)));
        }
        let marks = match group.get(&key("marks")) {
            None | Some(Yaml::Null) => &[][..],
            Some(Yaml::Array(marks)) => &marks[..],
            Some(_) => return Err(unread(format!("marks that are not a list (group {})", g + 1))),
        };
        let marks = marks.iter().enumerate().map(|(m, mark)| mark_of(mark).ok_or_else(|| unread(format!("a mark eyepiece does not know (group {}, mark {})", g + 1, m + 1))));
        read.push(Group { frames: (first, last), marks: marks.collect::<Result<_, _>>()? });
    }
    Ok(read)
}

/// the file's text: a line of comment, then the top mapping. mappings and lists run down the page, two spaces in for
/// each level, but for those that hold only plain values, which are written on one line: a mark, a pen's points, a
/// file's size and time.
fn written(top: &Hash, header: &str) -> String {
    let mut out = format!("# {header}\n");
    mapping(&mut out, top, 0, 0);
    out
}

/// true when a value can be written at all: every name in it is plain, and nothing in it is unresolved
fn writable(value: &Yaml) -> bool {
    match value {
        Yaml::Hash(map) => map.iter().all(|(name, value)| scalar(name).is_some() && writable(value)),
        Yaml::Array(items) => items.iter().all(writable),
        other => scalar(other).is_some(),
    }
}

/// a plain value as the file writes it. none for a list, a mapping, and what a file should not hold
fn scalar(value: &Yaml) -> Option<String> {
    Some(match value {
        Yaml::Integer(n) => n.to_string(),
        Yaml::Real(digits) => digits.clone(),
        Yaml::Boolean(yes) => yes.to_string(),
        Yaml::Null => "~".to_string(),
        Yaml::String(words) => quoted(words),
        _ => return None,
    })
}

/// a string as the file writes it: bare when it is one word that yaml reads as nothing but that string (a name, an
/// ink, a time), else between double quotes with what needs it escaped
fn quoted(words: &str) -> String {
    let bare = words.chars().next().is_some_and(char::is_alphanumeric)
        && words.chars().all(|c| c.is_alphanumeric() || "_-.+():#".contains(c))
        && !words.ends_with(':')
        && matches!(Yaml::from_str(words), Yaml::String(_));
    if bare {
        return words.to_string();
    }
    let mut out = String::from("\"");
    for c in words.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // what yaml may take for the end of a line, or not print
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{feff}') => _ = write!(out, "\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out + "\""
}

/// a value on one line, when it is one a line holds: a plain value, a list of them, or a mapping of those
fn line(value: &Yaml) -> Option<String> {
    let plain_list = |items: &[Yaml]| Some(format!("[{}]", items.iter().map(scalar).collect::<Option<Vec<_>>>()?.join(", ")));
    match value {
        Yaml::Array(items) => plain_list(items),
        Yaml::Hash(map) if map.is_empty() => Some("{}".to_string()),
        Yaml::Hash(map) => {
            let parts = map.iter().map(|(name, value)| {
                let value = match value {
                    Yaml::Array(items) => plain_list(items)?,
                    other => scalar(other)?,
                };
                Some(format!("{}: {value}", scalar(name)?))
            });
            Some(format!("{{ {} }}", parts.collect::<Option<Vec<_>>>()?.join(", ")))
        }
        other => scalar(other),
    }
}

/// a mapping down the page, `indent` spaces in. `depth` counts the mappings it is inside: the top one, the clips and a
/// clip's entry are never put on a line, however little they hold
fn mapping(out: &mut String, map: &Hash, indent: usize, depth: usize) {
    for (name, value) in map {
        let name = scalar(name).unwrap_or_default();
        let on_a_line = match value {
            Yaml::Hash(inner) if depth < 2 && !inner.is_empty() => None,
            _ => line(value),
        };
        match (on_a_line, value) {
            (Some(text), _) => _ = writeln!(out, "{:indent$}{name}: {text}", ""),
            (None, Yaml::Hash(inner)) => {
                _ = writeln!(out, "{:indent$}{name}:", "");
                mapping(out, inner, indent + 2, depth + 1);
            }
            (None, Yaml::Array(items)) => {
                _ = writeln!(out, "{:indent$}{name}:", "");
                list(out, items, indent + 2, depth + 1);
            }
            (None, _) => {}
        }
    }
}

/// a list down the page: each item after a dash, a mapping's first line on the dash's own
fn list(out: &mut String, items: &[Yaml], indent: usize, depth: usize) {
    for item in items {
        match (line(item), item) {
            (Some(text), _) => _ = writeln!(out, "{:indent$}- {text}", ""),
            (None, Yaml::Hash(inner)) => {
                let mut under = String::new();
                mapping(&mut under, inner, indent + 2, depth + 1);
                _ = write!(out, "{:indent$}- {}", "", &under[indent + 2..]);
            }
            (None, Yaml::Array(inner)) => {
                _ = writeln!(out, "{:indent$}-", "");
                list(out, inner, indent + 2, depth + 1);
            }
            (None, _) => {}
        }
    }
}

/// what the keeper has to tell the window
#[derive(Debug, PartialEq)]
pub enum Came {
    /// a clip's notes, read
    Read(Clip, Read),
    /// something to say, for a moment
    Said(String),
}

enum Job {
    Read(Clip),
    Keep(Clip, Vec<Group>, bool),
    Flush,
}

/// reads and saves notes on a thread of its own, in the order asked, so the window never waits on a disk or a share.
/// a save waits a moment for the changes that follow it; a read of a clip, a flush and the keeper's own end write
/// what is waiting first.
pub struct Keeper {
    jobs: Option<Sender<Job>>,
    came: Receiver<Came>,
    done: Receiver<()>,
}

impl Keeper {
    /// `own` is eyepiece's own folder, where notes that cannot be beside their clip are kept; `wake` wakes the window
    /// for what came
    pub fn start(own: Option<PathBuf>, wake: impl Fn() + Send + 'static) -> Keeper {
        let (jobs, asked) = mpsc::channel();
        let (tell, came) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let work = move || {
            let say = |what: Came| {
                let _ = tell.send(what);
                wake();
            };
            // the saves waiting, the newest of each clip, and when they are due; the folders that could not be written,
            // each said once
            let mut waiting: Vec<(Clip, Vec<Group>, bool)> = Vec::new();
            let mut due: Option<Instant> = None;
            let mut told: HashSet<PathBuf> = HashSet::new();
            let write = |waiting: &mut Vec<(Clip, Vec<Group>, bool)>, told: &mut HashSet<PathBuf>| {
                for (clip, groups, beside) in waiting.drain(..) {
                    match save(&clip, &groups, beside, own.as_deref()) {
                        Ok(Kept::Own(Some(why))) if told.insert(clip.folder.clone()) => say(Came::Said(format!("notes cannot be saved beside {} · {why} · kept in eyepiece's own folder", clip.name))),
                        Ok(_) => {}
                        Err(why) => say(Came::Said(format!("the notes on {} were not saved · {why}", clip.name))),
                    }
                }
            };
            loop {
                let job = match due {
                    Some(at) => asked.recv_timeout(at.saturating_duration_since(Instant::now())),
                    None => asked.recv().map_err(|_| RecvTimeoutError::Disconnected),
                };
                match job {
                    Ok(Job::Keep(clip, groups, beside)) => {
                        waiting.retain(|(other, ..)| *other != clip);
                        waiting.push((clip, groups, beside));
                        due.get_or_insert(Instant::now() + QUIET);
                    }
                    Ok(Job::Read(clip)) => {
                        write(&mut waiting, &mut told);
                        due = None;
                        let read = read(&clip, own.as_deref());
                        if let Some(said) = read.said.clone().filter(|_| told.insert(clip.folder.clone())) {
                            say(Came::Said(said));
                        }
                        say(Came::Read(clip, read));
                    }
                    Ok(Job::Flush) | Err(RecvTimeoutError::Timeout) => {
                        write(&mut waiting, &mut told);
                        due = None;
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        write(&mut waiting, &mut told);
                        let _ = finished.send(());
                        return;
                    }
                }
            }
        };
        let started = std::thread::Builder::new().name("notes".into()).spawn(work);
        if let Err(e) = &started {
            eprintln!("eyepiece: notes are neither read nor saved, their thread did not start: {e}");
        }
        let jobs = started.ok().map(|_| jobs);
        Keeper { jobs, came, done }
    }

    fn ask(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(job);
        }
    }

    /// asks for a clip's notes: they come by `take`
    pub fn read(&self, clip: Clip) {
        self.ask(Job::Read(clip));
    }

    /// a clip's notes as they are now, to be saved a moment from now: beside the clip, or in eyepiece's own folder
    pub fn keep(&self, clip: Clip, groups: Vec<Group>, beside: bool) {
        self.ask(Job::Keep(clip, groups, beside));
    }

    /// saves what is waiting now: a tab closed
    pub fn flush(&self) {
        self.ask(Job::Flush);
    }

    /// what has come since the last look
    pub fn take(&self) -> Option<Came> {
        self.came.try_recv().ok()
    }
}

impl Drop for Keeper {
    /// the window closes: what is waiting is saved first, and waited for, but not for ever
    fn drop(&mut self) {
        if self.jobs.take().is_some() {
            let _ = self.done.recv_timeout(LAST_SAVE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const RED: [u8; 3] = INKS[0];

    /// a folder of its own for one test, with a clip in it that is a few bytes (notes never open a clip), and a folder
    /// to stand for eyepiece's own. nothing is ever written beside the repo's test media.
    struct Scratch {
        folder: PathBuf,
        own: PathBuf,
        clips: Vec<&'static str>,
    }

    impl Scratch {
        fn new(test: &str) -> Scratch {
            let root = std::env::temp_dir().join(format!("eyepiece-notes-{test}-{}", std::process::id()));
            let scratch = Scratch { folder: root.join("shots"), own: root.join("own"), clips: Vec::new() };
            fs::create_dir_all(&scratch.folder).unwrap();
            scratch
        }

        fn clip(&mut self, name: &'static str) -> Clip {
            fs::write(self.folder.join(name), b"not a real clip").unwrap();
            self.clips.push(name);
            Clip::of(&self.folder.join(name).to_string_lossy(), name)
        }

        fn file(&self) -> PathBuf {
            self.folder.join(FILE)
        }

        fn previous(&self) -> PathBuf {
            self.folder.join(format!("{FILE}.previous"))
        }

        fn text(&self) -> String {
            fs::read_to_string(self.file()).unwrap().replace("\r\n", "\n")
        }

        /// removes what the test made, a file at a time by its name, then the folders, which must then be empty: a
        /// file left behind (a temporary one, say) fails the test. a failed test leaves everything for a look
        fn tidy(self) {
            let kept = own_file(&self.own, &self.folder);
            let made = self.clips.iter().map(|name| self.folder.join(name)).chain([self.file(), self.previous(), kept.clone(), previous_of(&kept)]);
            for file in made.filter(|file| file.exists()) {
                fs::remove_file(&file).unwrap();
            }
            for dir in [self.folder.clone(), self.own.join("notes"), self.own.clone(), self.folder.parent().unwrap().to_path_buf()] {
                if dir.exists() {
                    fs::remove_dir(&dir).unwrap_or_else(|e| panic!("{} is not empty: {e}", dir.display()));
                }
            }
        }
    }

    fn boxed(x: f32) -> Mark {
        Mark { shape: Shape::Rectangle((x, x), (x + 10.0, x + 20.0)), ink: RED }
    }

    fn group(frames: (usize, usize), marks: &[Mark]) -> Group {
        Group { frames, marks: marks.to_vec() }
    }

    /// the file as the owner accepted it, less the empty `exports`, which is left out until there is an export
    const EXAMPLE: &str = r#"# eyepiece's notes for the clips in this folder
version: 1
clips:
  pillars_v012.mov:
    file: { bytes: 15, modified: 2026-11-26T19:40:12Z }
    groups:
      - frames: [55, 129]
        marks:
          - { circle: [812, 440, 120, 120], ink: red }
          - { rect: [100, 100, 300, 200], ink: yellow }
          - { pen: [10, 10, 12, 14, 15.5, 20.25], ink: red }
          - { text: "this flickers", at: [900, 300], size: 1.0, ink: white }
"#;

    fn example() -> Vec<Group> {
        let marks = [
            Mark { shape: Shape::Circle((812.0, 440.0), (932.0, 560.0)), ink: INKS[0] },
            Mark { shape: Shape::Rectangle((100.0, 100.0), (400.0, 300.0)), ink: INKS[2] },
            Mark { shape: Shape::Pen(vec![(10.0, 10.0), (12.0, 14.0), (15.5, 20.25)]), ink: INKS[0] },
            Mark { shape: Shape::Text((900.0, 300.0), "this flickers".into(), 1.0), ink: INKS[7] },
        ];
        vec![group((55, 129), &marks)]
    }

    /// when a file was last written, set for a test
    fn set_modified(file: &Path, seconds: u64) {
        let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
        fs::File::options().write(true).open(file).unwrap().set_modified(at).unwrap();
    }

    /// 2026-11-26T19:40:12Z
    const WHEN: u64 = 1_795_722_012;

    /// the file is as simple as the owner accepted it: what is saved is that text to the letter, and reading it gives
    /// the marks back, each in the file's pixels and its ink by name.
    #[test]
    fn the_file_is_written_as_accepted_and_reads_back_the_same() {
        let mut scratch = Scratch::new("round-trip");
        let clip = scratch.clip("pillars_v012.mov");
        set_modified(&clip.stamped, WHEN);
        assert_eq!(utc(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000)), "2001-09-09T01:46:40Z");
        assert_eq!(save(&clip, &example(), true, Some(&scratch.own)), Ok(Kept::Beside));
        assert_eq!(scratch.text(), EXAMPLE);
        assert_eq!(read(&clip, Some(&scratch.own)), Read { groups: example(), earlier: false, beside: true, said: None });
        assert!(!scratch.own.exists(), "nothing is kept in eyepiece's own folder when it is saved beside the clip");
        scratch.tidy();
    }

    /// names and text that yaml would read as something else come back as they were: each is written so that the
    /// loader reads the same string. a mark drawn from its far corner is written from its near one; an ink that is
    /// not one of the ten keeps its red, green and blue; a place is kept to a hundredth of a pixel.
    #[test]
    fn awkward_names_text_and_numbers_come_back_as_they_were() {
        let mut scratch = Scratch::new("awkward");
        let names = ["a: b.mov", "# not a comment.mov", "- dash.mov", "it's \"quoted\".mov", "été 日本.mov", "true", "123", "1e3", "~", "null", " lead and trail ", "tab\there", "{flow}, [seq].mov", "@at `tick` %pct !bang &and *star |pipe >gt ?q.mov", "shot 12 (final) v2+fix.mov", "ends with colon:", "v#2.mov", "a:#b", "shot_010.####.exr", "a:b:c"];
        let texts = ["", "plain", "two\nlines", "a: b # c", "\"", "\\", "- x", "🙂 ok", "yes", "2026-11-26T19:40:12Z", "\u{2028}"];
        let marks: Vec<Mark> = texts.iter().map(|text| Mark { shape: Shape::Text((1.0, 2.0), text.to_string(), 1.5), ink: INKS[4] }).collect();
        let odd = [
            Mark { shape: Shape::Rectangle((400.0, 300.0), (100.126, 100.0)), ink: [0x3d, 0xf0, 0xff] },
            Mark { shape: Shape::Circle((-5.5, 0.004), (10.0, 20.0)), ink: INKS[9] },
            Mark { shape: Shape::Pen(vec![(0.1, 0.25), (1234.567, -2.0)]), ink: INKS[8] },
        ];
        let read_back = [
            Mark { shape: Shape::Rectangle((100.13, 100.0), (400.0, 300.0)), ink: [0x3d, 0xf0, 0xff] },
            Mark { shape: Shape::Circle((-5.5, 0.0), (10.0, 20.0)), ink: INKS[9] },
            Mark { shape: Shape::Pen(vec![(0.1, 0.25), (1234.57, -2.0)]), ink: INKS[8] },
        ];
        for (i, name) in names.iter().enumerate() {
            let clip = Clip { name: name.to_string(), ..scratch.clip("clip.mov") };
            let groups = [group((i, i + 3), &marks), group((0, 0), &odd)];
            assert_eq!(save(&clip, &groups, true, Some(&scratch.own)), Ok(Kept::Beside), "{name}");
            let read = read(&clip, Some(&scratch.own));
            assert_eq!((read.groups.len(), read.said), (2, None), "{name}");
            assert_eq!((&read.groups[0], &read.groups[1].marks[..]), (&groups[0], &read_back[..]), "{name}");
        }
        // every name is in the one file, each with its own notes
        assert_eq!(parse(&fs::read(scratch.file()).unwrap()).unwrap().get(&key("clips")).and_then(Yaml::as_hash).map(Hash::len), Some(names.len()));
        scratch.tidy();
    }

    /// before each save the file is read again and only the clip saved is changed: another clip's notes, an export
    /// logged for this one, and keys this build does not know all stay as they were.
    #[test]
    fn a_save_changes_only_its_clip_and_keeps_what_it_does_not_know() {
        let mut scratch = Scratch::new("merge");
        let (ours, theirs) = (scratch.clip("ours.mov"), scratch.clip("theirs.mov"));
        let by_hand = "version: 1\nfrom_a_newer_build: [1, 2]\nclips:\n  theirs.mov:\n    groups:\n      - frames: [3, 9]\n        marks:\n          - { rect: [1, 2, 3, 4], ink: green }\n    reviewed: yes\n  ours.mov:\n    exports:\n      - { when: 2026-11-26T19:40:12Z, file: ours_trim.mov, frames: [0, 9], how: copy, command: \"ffmpeg -i ours.mov\" }\n";
        fs::write(scratch.file(), by_hand).unwrap();
        let their_groups = read(&theirs, None).groups;
        assert_eq!(their_groups, [group((3, 9), &[Mark { shape: Shape::Rectangle((1.0, 2.0), (4.0, 6.0)), ink: INKS[3] }])]);
        assert_eq!(save(&ours, &[group((0, 4), &[boxed(1.0)])], true, None), Ok(Kept::Beside));
        let text = scratch.text();
        for kept in ["from_a_newer_build: [1, 2]\n", "    reviewed: yes\n", "      - { when: 2026-11-26T19:40:12Z, file: ours_trim.mov, frames: [0, 9], how: copy, command: \"ffmpeg -i ours.mov\" }\n", "          - { rect: [1, 2, 3, 4], ink: green }\n"] {
            assert!(text.contains(kept), "{kept} is not in:\n{text}");
        }
        assert_eq!((read(&theirs, None).groups, read(&ours, None).groups), (their_groups, vec![group((0, 4), &[boxed(1.0)])]));
        // and the other way: their save leaves ours
        assert_eq!(save(&theirs, &[], true, None), Ok(Kept::Beside));
        assert_eq!((read(&theirs, None).groups, read(&ours, None).groups), (vec![], vec![group((0, 4), &[boxed(1.0)])]));
        assert!(scratch.text().contains("    reviewed: yes\n"), "their entry stays for what else it holds:\n{}", scratch.text());
        scratch.tidy();
    }

    /// a file eyepiece cannot read is never written over, whatever is wrong with it: its bytes are as they were after
    /// a save, the notes are in eyepiece's own folder instead, and reading says which line.
    #[test]
    fn a_file_that_cannot_be_read_is_never_written_over() {
        let cases: [(&str, &[u8], &str); 7] = [
            ("syntax", b"version: 1\nclips:\n  a.mov: b: c\n  b.mov: {}\n", "line 3"),
            ("newer", b"version: 2\nclips: {}\n", "version 2"),
            ("utf8", b"version: 1\n# \xff\xfe\nclips: {}\n", "line 2"),
            ("shape", b"version: 1\nclips: [a, b]\n", "clips"),
            ("list", b"- a\n- b\n", "mapping"),
            ("two", b"version: 1\n---\nversion: 1\n", "more than one"),
            ("entry", b"version: 1\nclips:\n  clip.mov:\n    groups:\n      - frames: [0, 4]\n        marks:\n          - { star: [1, 2], ink: red }\n", "clip.mov"),
        ];
        for (case, bytes, says) in cases {
            let mut scratch = Scratch::new(&format!("unreadable-{case}"));
            let clip = scratch.clip("clip.mov");
            fs::write(scratch.file(), bytes).unwrap();
            let before = read(&clip, Some(&scratch.own));
            assert!(!before.beside && before.groups.is_empty(), "{case}: {before:?}");
            assert!(before.said.as_ref().is_some_and(|said| said.contains(says) && said.starts_with(FILE)), "{case}: {:?}", before.said);
            let groups = [group((0, 4), &[boxed(1.0)])];
            let kept = save(&clip, &groups, true, Some(&scratch.own));
            assert!(matches!(&kept, Ok(Kept::Own(Some(why))) if why.contains(says)), "{case}: {kept:?}");
            assert_eq!(fs::read(scratch.file()).unwrap(), bytes, "{case}: the file was changed");
            assert!(!scratch.previous().exists(), "{case}");
            let after = read(&clip, Some(&scratch.own));
            assert_eq!((after.groups, after.beside), (groups.to_vec(), false), "{case}");
            scratch.tidy();
        }
    }

    /// a save whose write fails midway leaves the file as it was, to the byte, and no half-written file beside it;
    /// so does one that eyepiece could not read back.
    #[test]
    fn a_write_that_fails_midway_leaves_the_file_whole() {
        let mut scratch = Scratch::new("midway");
        let clip = scratch.clip("clip.mov");
        save(&clip, &[group((0, 4), &[boxed(1.0)])], true, None).unwrap();
        let before = fs::read(scratch.file()).unwrap();
        let half = |file: &mut fs::File, bytes: &[u8]| {
            use std::io::Write;
            file.write_all(&bytes[..bytes.len() / 2])?;
            Err(std::io::Error::other("the disk went away"))
        };
        let change = |top: &mut Hash| set_groups(top, &clip, &[group((7, 9), &[boxed(2.0)])], None);
        let failed = put(&scratch.file(), HEADER, &half, &change);
        assert!(failed.as_ref().is_err_and(|why| why.contains("the disk went away")), "{failed:?}");
        assert_eq!(fs::read(scratch.file()).unwrap(), before);
        assert!(!scratch.previous().exists(), "nothing was replaced, so nothing was set aside");
        // a writer that writes something else: what is on disk is checked before it is swapped in
        let wrong = |file: &mut fs::File, bytes: &[u8]| {
            use std::io::Write;
            file.write_all(&bytes[..bytes.len() - 20])
        };
        let failed = put(&scratch.file(), HEADER, &wrong, &change);
        assert!(failed.is_err(), "{failed:?}");
        assert_eq!(fs::read(scratch.file()).unwrap(), before);
        // the folder holds the clip and the file and nothing else: `tidy` fails on a temporary file left behind
        scratch.tidy();
    }

    /// each save keeps the version it replaced as `eyepiece.yaml.previous`; the first, which replaces nothing, keeps
    /// none.
    #[test]
    fn the_version_a_save_replaces_is_kept_as_previous() {
        let mut scratch = Scratch::new("previous");
        let clip = scratch.clip("clip.mov");
        save(&clip, &[group((0, 4), &[boxed(1.0)])], true, None).unwrap();
        assert!(!scratch.previous().exists());
        let first = fs::read(scratch.file()).unwrap();
        save(&clip, &[group((0, 4), &[boxed(1.0), boxed(2.0)])], true, None).unwrap();
        assert_eq!(fs::read(scratch.previous()).unwrap(), first);
        let second = fs::read(scratch.file()).unwrap();
        assert_ne!(second, first);
        save(&clip, &[group((2, 4), &[boxed(3.0)])], true, None).unwrap();
        assert_eq!(fs::read(scratch.previous()).unwrap(), second);
        // a save that changes nothing writes nothing
        let third = (fs::read(scratch.file()).unwrap(), fs::metadata(scratch.file()).unwrap().modified().unwrap());
        save(&clip, &[group((2, 4), &[boxed(3.0)])], true, None).unwrap();
        assert_eq!((fs::read(scratch.file()).unwrap(), fs::metadata(scratch.file()).unwrap().modified().unwrap()), third);
        assert_eq!(fs::read(scratch.previous()).unwrap(), second);
        scratch.tidy();
    }

    /// opening a clip never makes the file, nor does saving no notes where there are none. a clip whose last note is
    /// taken off loses its entry; a file left with no clip in it is not deleted: it goes aside as its `.previous`.
    #[test]
    fn the_file_is_there_only_while_there_is_a_note_to_keep() {
        let mut scratch = Scratch::new("only-notes");
        let (a, b) = (scratch.clip("a.mov"), scratch.clip("b.mov"));
        assert_eq!(read(&a, Some(&scratch.own)), Read { groups: vec![], earlier: false, beside: true, said: None });
        assert_eq!(save(&a, &[], true, Some(&scratch.own)), Ok(Kept::Beside));
        assert!(!scratch.file().exists() && !scratch.own.exists(), "nothing to keep, nothing written");
        save(&a, &[group((0, 4), &[boxed(1.0)])], true, Some(&scratch.own)).unwrap();
        save(&b, &[group((1, 5), &[boxed(2.0)])], true, Some(&scratch.own)).unwrap();
        save(&a, &[], true, Some(&scratch.own)).unwrap();
        assert!(!scratch.text().contains("a.mov") && scratch.text().contains("b.mov"), "{}", scratch.text());
        let last = fs::read(scratch.file()).unwrap();
        save(&b, &[], true, Some(&scratch.own)).unwrap();
        assert!(!scratch.file().exists(), "no clip left in it");
        assert_eq!(fs::read(scratch.previous()).unwrap(), last, "its last version is kept aside, not deleted");
        assert_eq!(read(&b, Some(&scratch.own)).groups, vec![]);
        scratch.tidy();
    }

    /// a folder that cannot be written (a read-only share, a camera card) keeps its notes in eyepiece's own folder,
    /// and says why; they are read from there. once the folder can be written they are saved beside the clip, and
    /// the copy in eyepiece's own folder goes.
    #[test]
    fn a_folder_that_cannot_be_written_keeps_its_notes_in_eyepieces_own_folder() {
        let mut scratch = Scratch::new("unwritable");
        let real = scratch.clip("clip.mov");
        // a folder that is not there cannot be written: no file can be made in it
        let away = Clip { folder: scratch.folder.join("away"), ..real.clone() };
        let groups = [group((0, 4), &[boxed(1.0)])];
        let kept = save(&away, &groups, true, Some(&scratch.own));
        assert!(matches!(&kept, Ok(Kept::Own(Some(_)))), "{kept:?}");
        assert_eq!(read(&away, Some(&scratch.own)), Read { groups: groups.to_vec(), earlier: false, beside: true, said: None });
        assert!(save(&away, &groups, true, None).is_err(), "with no folder of eyepiece's own there is nowhere to keep them");
        // with the setting off they are kept in eyepiece's own folder, and nothing is written beside the clip
        assert_eq!(save(&real, &groups, false, Some(&scratch.own)), Ok(Kept::Own(None)));
        assert!(!scratch.file().exists());
        let kept_in = own_file(&scratch.own, &scratch.folder);
        assert!(fs::read_to_string(&kept_in).unwrap().starts_with(&format!("# eyepiece's notes for the clips in {}", scratch.folder.display())));
        assert_eq!(read(&real, Some(&scratch.own)).groups, groups);
        // saved beside the clip after all: its copy in eyepiece's own folder has been overtaken, and goes aside
        let more = [group((0, 4), &[boxed(1.0), boxed(2.0)])];
        assert_eq!(save(&real, &more, true, Some(&scratch.own)), Ok(Kept::Beside));
        assert!(scratch.file().exists() && !kept_in.exists());
        assert_eq!(read(&real, Some(&scratch.own)).groups, more);
        // (the folder that is not there has a file of its own in eyepiece's folder)
        let theirs = own_file(&scratch.own, &away.folder);
        fs::remove_file(&theirs).unwrap();
        scratch.tidy();
    }

    /// notes say which version of the clip's file they were made on, by its size and when it was written, to the
    /// second. a file that is another one now still shows them, labeled; a save makes them this version's.
    #[test]
    fn notes_made_on_an_earlier_version_of_the_file_are_labeled() {
        let mut scratch = Scratch::new("earlier");
        let clip = scratch.clip("clip.mov");
        set_modified(&clip.stamped, WHEN);
        let groups = [group((0, 4), &[boxed(1.0)])];
        save(&clip, &groups, true, None).unwrap();
        assert!(scratch.text().contains("    file: { bytes: 15, modified: 2026-11-26T19:40:12Z }\n"), "{}", scratch.text());
        assert!(!read(&clip, None).earlier);
        set_modified(&clip.stamped, WHEN + 60);
        assert_eq!(read(&clip, None), Read { groups: groups.to_vec(), earlier: true, beside: true, said: None });
        set_modified(&clip.stamped, WHEN);
        fs::write(&clip.stamped, b"rendered again, longer").unwrap();
        set_modified(&clip.stamped, WHEN);
        assert!(read(&clip, None).earlier, "another size");
        save(&clip, &[group((0, 4), &[boxed(1.0), boxed(2.0)])], true, None).unwrap();
        assert!(!read(&clip, None).earlier && scratch.text().contains("bytes: 22"), "{}", scratch.text());
        scratch.tidy();
    }

    /// another eyepiece that saves between this one's read of the file and its swap is not written over: the file is
    /// found changed, read again, and both saves are in it.
    #[test]
    fn a_save_that_another_overtakes_reads_the_file_again() {
        let mut scratch = Scratch::new("overtaken");
        let (ours, theirs) = (scratch.clip("ours.mov"), scratch.clip("theirs.mov"));
        save(&ours, &[group((0, 4), &[boxed(1.0)])], true, None).unwrap();
        let overtaken = std::cell::Cell::new(false);
        // the other's save lands while ours is being written
        let slow = |file: &mut fs::File, bytes: &[u8]| {
            use std::io::Write;
            if !overtaken.replace(true) {
                save(&theirs, &[group((5, 9), &[boxed(5.0)])], true, None).unwrap();
            }
            file.write_all(bytes)
        };
        put_unlocked(&scratch.file(), HEADER, &slow, &|top: &mut Hash| set_groups(top, &ours, &[group((0, 4), &[boxed(2.0)])], None)).unwrap();
        assert_eq!((read(&ours, None).groups, read(&theirs, None).groups), (vec![group((0, 4), &[boxed(2.0)])], vec![group((5, 9), &[boxed(5.0)])]));
        scratch.tidy();
    }

    /// a numbered run is one clip whichever of its files it was opened by, named without its range so that frames
    /// rendered later change nothing, and its groups' frames are written as its files are numbered.
    #[test]
    fn a_numbered_run_is_named_without_its_range_and_counted_as_its_files_are() {
        let mut scratch = Scratch::new("run");
        let first = scratch.clip("seq.1001.exr");
        scratch.clip("seq.1002.exr");
        let path = scratch.folder.join("seq.1002.exr");
        let run = Clip::of(&path.to_string_lossy(), "seq.[1001-1002].exr");
        assert_eq!(run, Clip { folder: scratch.folder.clone(), name: "seq.####.exr".into(), first: 1001, stamped: first.stamped.clone() });
        assert_eq!(Clip::of(&scratch.folder.join("seq.1001.exr").to_string_lossy(), "seq.[1001-1240].exr"), run, "more frames, the same clip");
        assert_eq!(Clip::of("b.[1-10].png", "b.[1-10].png").name, "b.[1-10].png", "a file named like a run is itself");
        assert_eq!(Clip::of("shots/a.9.png", "a.[8-10].png"), Clip { folder: "shots".into(), name: "a.#.png".into(), first: 8, stamped: Path::new("shots").join("a.8.png") });
        assert_eq!(first, Clip { folder: scratch.folder.clone(), name: "seq.1001.exr".into(), first: 0, stamped: scratch.folder.join("seq.1001.exr") });
        save(&run, &[group((0, 1), &[boxed(1.0)])], true, None).unwrap();
        assert!(scratch.text().contains("  seq.####.exr:\n") && scratch.text().contains("      - frames: [1001, 1002]\n"), "{}", scratch.text());
        assert_eq!(read(&run, None).groups, [group((0, 1), &[boxed(1.0)])]);
        // a run that now starts later: a frame before its first cannot be shown, so the group is not read as nothing
        let later = Clip { first: 1002, ..run.clone() };
        assert!(read(&later, None).said.is_some_and(|said| said.contains("seq.####.exr")));
        scratch.tidy();
    }

    /// an export is logged in its clip's entry with the care a save takes, and stays through saves of the clip's
    /// notes, and when the last of them is taken off. one that ran no command (a copy) has none in its line.
    #[test]
    fn an_export_is_logged_and_stays_through_saves() {
        let mut scratch = Scratch::new("exports");
        let clip = scratch.clip("clip.mov");
        let export = Export { when: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(WHEN), file: "clip_trim.mov".into(), frames: (55, 129), how: "copy".into(), command: Some("ffmpeg -ss 2.2 -i \"clip.mov\" -c copy".into()) };
        assert_eq!(log_export(&clip, &export, true, None), Ok(Kept::Beside));
        let line = "    exports:\n      - { when: 2026-11-26T19:40:12Z, file: clip_trim.mov, frames: [55, 129], how: copy, command: \"ffmpeg -ss 2.2 -i \\\"clip.mov\\\" -c copy\" }\n";
        assert!(scratch.text().ends_with(line), "{}", scratch.text());
        save(&clip, &[group((0, 4), &[boxed(1.0)])], true, None).unwrap();
        log_export(&clip, &Export { file: "clip_trim (2).mov".into(), how: "copy, no sound".into(), command: None, ..export }, true, None).unwrap();
        assert!(scratch.text().contains("      - { when: 2026-11-26T19:40:12Z, file: \"clip_trim (2).mov\", frames: [55, 129], how: \"copy, no sound\" }\n"), "{}", scratch.text());
        save(&clip, &[], true, None).unwrap();
        let text = scratch.text();
        assert!(text.contains("clip_trim.mov") && text.contains("clip_trim (2).mov") && !text.contains("groups"), "{text}");
        assert!(scratch.file().exists(), "an export is something to keep");
        scratch.tidy();
    }

    /// an export logged beside the clip leaves the clip's notes in eyepiece's own folder where they are: only a save
    /// of the notes themselves moves them beside it.
    #[test]
    fn an_export_logged_beside_the_clip_leaves_its_notes_in_eyepieces_own_folder() {
        let mut scratch = Scratch::new("export-own");
        let clip = scratch.clip("clip.mov");
        let groups = [group((0, 4), &[boxed(1.0)])];
        assert_eq!(save(&clip, &groups, false, Some(&scratch.own)), Ok(Kept::Own(None)));
        let export = Export { when: SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(WHEN), file: "clip_trim.mov".into(), frames: (0, 4), how: "copy".into(), command: None };
        assert_eq!(log_export(&clip, &export, true, Some(&scratch.own)), Ok(Kept::Beside));
        assert_eq!(read(&clip, Some(&scratch.own)).groups, groups);
        assert!(scratch.text().contains("clip_trim.mov") && !scratch.text().contains("groups"), "{}", scratch.text());
        scratch.tidy();
    }

    /// the keeper saves off the drawing thread, a moment after a change, and reads the same way: a read asked for
    /// after a change finds that change, and what is waiting when the window closes is written before it goes. so
    /// notes drawn on a clip are back when it opens again.
    #[test]
    fn notes_are_back_when_the_clip_opens_again() {
        let mut scratch = Scratch::new("keeper");
        let clip = scratch.clip("clip.mov");
        let woke = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = woke.clone();
        let keeper = Keeper::start(Some(scratch.own.clone()), move || _ = count.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let came = |keeper: &Keeper| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                if let Some(came) = keeper.take() {
                    return came;
                }
                assert!(std::time::Instant::now() < until, "the keeper did not answer");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        keeper.read(clip.clone());
        assert_eq!(came(&keeper), Came::Read(clip.clone(), Read { groups: vec![], earlier: false, beside: true, said: None }));
        assert!(!scratch.file().exists(), "opening a clip writes nothing");
        let groups = vec![group((0, 4), &[boxed(1.0)])];
        keeper.keep(clip.clone(), vec![group((0, 4), &[])], true);
        keeper.keep(clip.clone(), groups.clone(), true);
        assert!(!scratch.file().exists(), "a save waits a moment for more changes");
        // the tab closes and the clip is opened again at once
        keeper.read(clip.clone());
        assert_eq!(came(&keeper), Came::Read(clip.clone(), Read { groups: groups.clone(), earlier: false, beside: true, said: None }));
        assert!(!scratch.previous().exists(), "the two changes were one save");
        assert!(woke.load(std::sync::atomic::Ordering::Relaxed) >= 2, "the window is woken for what was read");
        // a change left waiting when the window closes is written first
        let more = vec![group((0, 4), &[boxed(1.0), boxed(2.0)])];
        keeper.keep(clip.clone(), more.clone(), true);
        drop(keeper);
        assert_eq!(read(&clip, Some(&scratch.own)).groups, more);
        scratch.tidy();
    }

    /// a save that lands in eyepiece's own folder for a folder that cannot be written says so once for the folder,
    /// not at every save.
    #[test]
    fn the_keeper_says_once_that_a_folder_cannot_be_written() {
        let mut scratch = Scratch::new("says-once");
        let away = Clip { folder: scratch.folder.join("away"), ..scratch.clip("clip.mov") };
        let keeper = Keeper::start(Some(scratch.own.clone()), || {});
        for x in [1.0, 2.0] {
            keeper.keep(away.clone(), vec![group((0, 4), &[boxed(x)])], true);
            keeper.flush();
        }
        keeper.read(away.clone());
        let mut said = Vec::new();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match keeper.take() {
                Some(Came::Said(words)) => said.push(words),
                Some(Came::Read(_, read)) => break assert_eq!(read.groups, [group((0, 4), &[boxed(2.0)])]),
                None => {
                    assert!(std::time::Instant::now() < until, "the keeper did not answer");
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("notes cannot be saved beside clip.mov · ") && said[0].ends_with(" · kept in eyepiece's own folder"), "{}", said[0]);
        drop(keeper);
        fs::remove_file(own_file(&scratch.own, &away.folder)).unwrap();
        fs::remove_file(previous_of(&own_file(&scratch.own, &away.folder))).unwrap();
        scratch.tidy();
    }
}
