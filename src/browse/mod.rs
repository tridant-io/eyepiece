//! the data behind the file browser: the sidebar's places, what kind of volume a path is on, and one folder's entries.
//! a folder is read from its listing alone: on a network volume nothing else may be asked (adr 0004).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Volume {
    Local,
    Removable,
    Network,
}

/// the sidebar's groups, in its order
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// the system's own folders: downloads, desktop and the like
    Known,
    /// the folders pinned to quick access in explorer
    Pinned,
    Volumes,
    /// the folders the browser last opened files from
    Recent,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub path: PathBuf,
    pub group: Group,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Clip,
    /// a numbered run of image files shown as one entry: name.0001.exr .. name.0240.exr.
    /// each file's number is written `{n:0pad$}`: pad is the zero-padded width, 0 when the numbers are not padded.
    Sequence { first: u64, last: u64, missing: Vec<u64>, pad: usize },
    Other,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: Kind,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    /// when it was put in its folder: macos keeps this (`read_added`), windows does not
    pub added: Option<SystemTime>,
    /// its data is not on this disk (a cloud placeholder, or moved to offline storage): reading it would fetch it.
    /// for a sequence, any of its files
    pub away: bool,
    /// it stands for another place: a symlink, a junction, a mount point. a cloud folder's own mark is no link
    pub link: bool,
}

/// what the browser shows of the folder it is in: its entries as a column beside its videos as cards, the cards
/// alone (the columns then list folders only), or the column alone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum View {
    #[default]
    Both,
    Gallery,
    List,
}

/// how the browser orders a folder's entries: by what, and which way. folders always come before files.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sort {
    pub by: By,
    pub descending: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum By {
    #[default]
    Name,
    Size,
    Modified,
    Created,
    Added,
}

/// what a folder can be sorted by and which way, in the words the browser's menu and the settings file use
pub const SORTS: [(&str, By); 5] = [("name", By::Name), ("size", By::Size), ("date modified", By::Modified), ("date created", By::Created), ("date added", By::Added)];
pub const WAYS: [(&str, bool); 2] = [("ascending", false), ("descending", true)];

impl By {
    /// the system keeps it: windows records no date an item was put in its folder, so it is not offered there
    pub fn kept(self) -> bool {
        self != By::Added || cfg!(target_os = "macos")
    }
}

/// orders a folder's entries by `sort`, folders first. a value the listing does not have (a folder's size, a date not
/// recorded) comes after the known ones whichever way the sort runs; ties go by name.
pub fn sort(entries: &mut [Entry], sort: Sort) {
    fn known<T: Ord>(a: Option<T>, b: Option<T>, descending: bool) -> Ordering {
        match (a, b) {
            (Some(a), Some(b)) if descending => b.cmp(&a),
            (Some(a), Some(b)) => a.cmp(&b),
            (a, b) => a.is_none().cmp(&b.is_none()),
        }
    }
    let down = sort.descending;
    entries.sort_by(|a, b| {
        let by = match sort.by {
            By::Name if down => natural_cmp(&b.name, &a.name),
            By::Name => Ordering::Equal,
            By::Size => known(a.size, b.size, down),
            By::Modified => known(a.modified, b.modified, down),
            By::Created => known(a.created, b.created, down),
            By::Added => known(a.added, b.added, down),
        };
        (a.kind != Kind::Folder).cmp(&(b.kind != Kind::Folder)).then(by).then_with(|| natural_cmp(&a.name, &b.name))
    });
}

/// how wide the browser's parts are, in the design's px: the sidebar, each folder's column by its place along the
/// path (the first from the root, then the next), the metadata column. each is dragged by its own edge and kept
/// between runs; a double click on an edge puts it back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Widths {
    pub side: f32,
    pub columns: [f32; COLUMNS],
    pub meta: f32,
}

/// the columns that keep a width of their own: one deeper than these is as wide as the last
pub const COLUMNS: usize = 16;
/// a column's width until it is dragged (figs. 27 and 28)
pub const COLUMN: f32 = 168.0;

impl Default for Widths {
    /// the design's (figs. 27 and 28)
    fn default() -> Widths {
        Widths { side: 150.0, columns: [COLUMN; COLUMNS], meta: 316.0 }
    }
}

impl Widths {
    /// within what keeps every part usable: a sidebar row's name, a column's names, the metadata's facts
    pub fn kept(self) -> Widths {
        Widths { side: self.side.clamp(110.0, 360.0), columns: self.columns.map(|w| w.clamp(120.0, 480.0)), meta: self.meta.clamp(240.0, 640.0) }
    }

    /// the width of the column at `depth` along the path
    pub fn column(&self, depth: usize) -> f32 {
        self.columns[depth.min(COLUMNS - 1)]
    }

    pub fn set_column(&mut self, depth: usize, width: f32) {
        self.columns[depth.min(COLUMNS - 1)] = width;
    }
}

const CLIPS: &[&str] = &["mov", "mp4", "m4v", "mxf", "avi", "mkv", "webm"];
const STILLS: &[&str] = &["exr", "dpx", "png", "jpg", "jpeg", "tif", "tiff", "tga"];
/// a numbered run wider than this is unrelated numbers (dates, timestamps), not frames. it also bounds `missing`.
const SPAN: u64 = 1_000_000;

/// what kind of volume a path is on. a unc path is network; anything that cannot be told is network too
/// (unknown counts as network: the careful choice).
#[cfg(windows)]
pub fn volume_of(path: &Path) -> Volume {
    use std::path::{Component, Prefix};
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows::Win32::System::WindowsProgramming::{DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOVABLE};
    // a unc path, or any prefix but a drive letter, is decided here without asking the system
    let Some(Component::Prefix(prefix)) = path.components().next() else { return Volume::Network };
    let (Prefix::Disk(letter) | Prefix::VerbatimDisk(letter)) = prefix.kind() else { return Volume::Network };
    // the root needs its backslash: "C:" alone is not a root
    match unsafe { GetDriveTypeW(&HSTRING::from(format!("{}:\\", letter as char))) } {
        DRIVE_FIXED | DRIVE_RAMDISK => Volume::Local,
        DRIVE_REMOVABLE | DRIVE_CDROM => Volume::Removable,
        // a mapped share, an unknown type, or no such drive
        _ => Volume::Network,
    }
}

/// on macos: the volume whose mount point holds the path, by the system's own table of what is mounted, which it
/// keeps and asks no server for. a path no mount point holds counts as network.
#[cfg(target_os = "macos")]
pub fn volume_of(path: &Path) -> Volume {
    mounts().into_iter().filter(|(at, _)| path.starts_with(at)).max_by_key(|(at, _)| at.as_os_str().len()).map_or(Volume::Network, |(_, kind)| kind)
}

/// every mounted volume: where it is mounted and its kind. asked with no wait, so no volume is itself asked.
#[cfg(target_os = "macos")]
fn mounts() -> Vec<(PathBuf, Volume)> {
    use std::os::unix::ffi::OsStrExt;
    // the system's `struct statfs`, 2168 bytes
    #[repr(C)]
    struct Mount {
        sizes: [u64; 6],
        fsid: [i32; 2],
        owner: u32,
        kind: u32,
        flags: u32,
        subtype: u32,
        type_name: [u8; 16],
        on: [u8; 1024],
        from: [u8; 1024],
        more: [u32; 8],
    }
    extern "C" {
        // on intel the plain name is the call for the old, shorter struct
        #[cfg_attr(target_arch = "x86_64", link_name = "getfsstat$INODE64")]
        fn getfsstat(mounts: *mut Mount, bytes: i32, flags: i32) -> i32;
    }
    const NO_WAIT: i32 = 2;
    const LOCAL: u32 = 0x1000;
    const REMOVABLE: u32 = 0x200;
    let count = unsafe { getfsstat(std::ptr::null_mut(), 0, NO_WAIT) };
    if count <= 0 {
        return Vec::new();
    }
    // room for a few more, should one be mounted between the two calls
    let mut list: Vec<Mount> = Vec::with_capacity(count as usize + 8);
    let filled = unsafe { getfsstat(list.as_mut_ptr(), (list.capacity() * std::mem::size_of::<Mount>()) as i32, NO_WAIT) };
    if filled <= 0 {
        return Vec::new();
    }
    unsafe { list.set_len((filled as usize).min(list.capacity())) };
    list.iter()
        .map(|m| {
            let at = &m.on[..m.on.iter().position(|b| *b == 0).unwrap_or(m.on.len())];
            let kind = match (m.flags & LOCAL != 0, m.flags & REMOVABLE != 0) {
                (false, _) => Volume::Network,
                (true, true) => Volume::Removable,
                (true, false) => Volume::Local,
            };
            (PathBuf::from(std::ffi::OsStr::from_bytes(at)), kind)
        })
        .collect()
}

/// linux comes in phase 5: until then every path counts as network there.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn volume_of(_path: &Path) -> Volume {
    Volume::Network
}

/// the sidebar's shortcuts, as the design's fig. 27 has them: downloads first (the browser opens there), then
/// desktop, documents, videos and pictures, from the system's known folders, not guessed paths; then every drive.
/// the folders pinned in explorer are read apart from these (`pinned`).
#[cfg(windows)]
pub fn places() -> Vec<Place> {
    use windows::core::GUID;
    use windows::Win32::Storage::FileSystem::GetLogicalDrives;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Pictures, FOLDERID_Videos, FOLDERID_Windows, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };
    let known_path = |id: GUID| {
        // without KF_FLAG_DONT_VERIFY the call fails when the folder is not there, so no probe of our own
        let at = unsafe { SHGetKnownFolderPath(&id, KF_FLAG_DEFAULT, None) }.ok()?;
        let path = PathBuf::from(unsafe { at.to_hstring() }.to_os_string());
        unsafe { CoTaskMemFree(Some(at.0 as _)) };
        Some(path)
    };
    let known = [
        ("downloads", FOLDERID_Downloads),
        ("desktop", FOLDERID_Desktop),
        ("documents", FOLDERID_Documents),
        ("videos", FOLDERID_Videos),
        ("pictures", FOLDERID_Pictures),
    ];
    let mut places: Vec<Place> = known.into_iter().filter_map(|(name, id)| Some(Place { name: name.into(), path: known_path(id)?, group: Group::Known })).collect();
    let system = known_path(FOLDERID_Windows).and_then(|p| p.to_str()?.chars().next());
    let used = unsafe { GetLogicalDrives() };
    places.extend((0..26u8).filter(|i| used & (1 << i) != 0).map(|i| {
        let letter = (b'A' + i) as char;
        let root = format!("{letter}:\\");
        // a share's label would be asked of the share itself (adr 0004), and a removable drive with nothing in it may
        // be slow to answer or ask for a disk, so only a local disk's label is read
        let label = if volume_of(Path::new(&root)) == Volume::Local { label(&root) } else { None };
        Place { name: drive_name(letter, label.as_deref(), system == Some(letter)), path: root.into(), group: Group::Volumes }
    }));
    places
}

/// the folders pinned to quick access in explorer, in its order, under their own names. read through the shell,
/// which lists them with explorer's frequent folders: only each item's parsing name and its pinned mark are asked
/// for, nothing of the folder itself, which may be on a share (adr 0004). it is too slow for the drawing thread, and
/// sets up com for itself: call it on a thread of its own.
#[cfg(windows)]
pub fn pinned() -> Vec<Place> {
    use std::path::Component;
    use windows::core::{w, Interface, Result};
    use windows::Win32::Storage::EnhancedStorage::PKEY_Home_IsPinned;
    use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{BHID_EnumItems, IEnumShellItems, IShellItem, IShellItem2, SHCreateItemFromParsingName, SIGDN_DESKTOPABSOLUTEPARSING};
    let read = || -> Result<Vec<Place>> {
        let frequent: IShellItem = unsafe { SHCreateItemFromParsingName(w!("shell:::{3936E9E4-D92C-4EEE-A85A-BC16D5EA0819}"), None)? };
        let items: IEnumShellItems = unsafe { frequent.BindToHandler(None, &BHID_EnumItems)? };
        let mut found = Vec::new();
        let mut item = [None];
        while unsafe { items.Next(&mut item, None) }.is_ok() {
            let Some(item) = item[0].take() else { break };
            if !unsafe { item.cast::<IShellItem2>()?.GetBool(&PKEY_Home_IsPinned) }.is_ok_and(|pinned| pinned.as_bool()) {
                continue;
            }
            let name = unsafe { item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)? };
            let path = PathBuf::from(unsafe { name.to_hstring() }.to_os_string());
            unsafe { CoTaskMemFree(Some(name.0 as _)) };
            // a drive or a share: the shell's own places (a library, this pc) are no folder to list
            if matches!(path.components().next(), Some(Component::Prefix(_))) {
                found.push(Place { name: name_of(&path), path, group: Group::Pinned });
            }
        }
        Ok(found)
    };
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let found = read().unwrap_or_default();
    if com {
        unsafe { CoUninitialize() };
    }
    found
}

/// explorer's alone: finder's favorites have no supported way in.
#[cfg(not(windows))]
pub fn pinned() -> Vec<Place> {
    Vec::new()
}

/// a local disk's label, as its file system keeps it
#[cfg(windows)]
fn label(root: &str) -> Option<String> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;
    let mut name = [0u16; 261];
    unsafe { GetVolumeInformationW(&HSTRING::from(root), Some(&mut name), None, None, None, None) }.ok()?;
    let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    Some(String::from_utf16_lossy(&name[..len]))
}

/// a drive as the sidebar names it: "system (C:)" for the one windows is on, else its label and its letter
/// ("media (D:)"), or its letter alone when there is no label to give
#[cfg(windows)]
pub fn drive_name(letter: char, label: Option<&str>, system: bool) -> String {
    match label.filter(|l| !l.is_empty()) {
        _ if system => format!("system ({letter}:)"),
        Some(label) => format!("{label} ({letter}:)"),
        None => format!("{letter}:"),
    }
}

/// a folder's own name, or the whole path for the top of a drive or share
pub fn name_of(folder: &Path) -> String {
    folder.file_name().map_or_else(|| folder.to_string_lossy().trim_end_matches('\\').to_string(), |n| n.to_string_lossy().into_owned())
}

/// on macos: the user's standard folders that are there, in fig. 27's order, then the startup disk and what is
/// mounted under /Volumes. nothing is asked of a volume: its name is its mount point's.
#[cfg(target_os = "macos")]
pub fn places() -> Vec<Place> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let folders = [("downloads", "Downloads"), ("desktop", "Desktop"), ("documents", "Documents"), ("movies", "Movies"), ("pictures", "Pictures")];
    let folders = home.into_iter().flat_map(|home| folders.map(|(name, folder)| Place { name: name.into(), path: home.join(folder), group: Group::Known })).filter(|place| place.path.is_dir());
    let volumes = mounts().into_iter().filter_map(|(at, _)| Some(Place { name: at.strip_prefix("/Volumes").ok()?.to_str().filter(|name| !name.is_empty())?.to_string(), path: at, group: Group::Volumes }));
    folders.chain(std::iter::once(Place { name: "/".into(), path: "/".into(), group: Group::Volumes })).chain(volumes).collect()
}

/// linux comes in phase 5.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn places() -> Vec<Place> {
    Vec::new()
}

/// one folder's entries: folders first, then files, each in natural name order. hidden and system files
/// are left out. image files that form a numbered run of two or more become one Sequence entry, with its
/// gaps recorded. sizes and dates come from the listing alone.
pub fn list(dir: &Path) -> io::Result<Vec<Entry>> {
    let (mut folders, mut files) = (Vec::new(), Vec::new());
    for item in fs::read_dir(dir)? {
        let item = item?;
        // on windows this comes with the listing; fs::metadata would open the file (phase 0, gate 6)
        let meta = item.metadata()?;
        let name = item.file_name().to_string_lossy().into_owned();
        let (folder, hidden, away) = flags(&name, &meta);
        if hidden {
            continue;
        }
        let (modified, created) = (meta.modified().ok(), meta.created().ok());
        // from the listing too: on windows a reparse point that names another place (onedrive's and dropbox's
        // folders are reparse points that do not)
        let link = meta.file_type().is_symlink();
        if folder {
            folders.push(Entry { name, path: item.path(), kind: Kind::Folder, size: None, modified, created, added: None, away, link });
        } else {
            let kind = if is_media(&name) { Kind::Clip } else { Kind::Other };
            files.push(Entry { name, path: item.path(), kind, size: Some(meta.len()), modified, created, added: None, away, link });
        }
    }
    let mut entries = fold_sequences(files);
    entries.append(&mut folders);
    sort(&mut entries, Sort::default());
    Ok(entries)
}

/// on macos: fills in when each entry was put in `dir` (a sequence's by its first frame), as its file system keeps it,
/// from one more listing of the folder, in bulk: nothing is asked of an entry itself. an item the file system keeps no
/// such date for (a share's may not) has none. the browser's alone: a walk of a tree does without.
#[cfg(target_os = "macos")]
pub fn read_added(dir: &Path, entries: &mut [Entry]) {
    let added = added(dir);
    for entry in entries {
        entry.added = entry.path.file_name().and_then(|name| added.get(name)).copied();
    }
}

/// windows keeps no date an item was put in its folder.
#[cfg(not(target_os = "macos"))]
pub fn read_added(_dir: &Path, _entries: &mut [Entry]) {}

/// when each item of `dir` was put there, by name, as getattrlistbulk gives it (sys/attr.h)
#[cfg(target_os = "macos")]
fn added(dir: &Path) -> HashMap<std::ffi::OsString, SystemTime> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::time::{Duration, UNIX_EPOCH};
    /// `struct attrlist`: the attributes asked for, a set of bits for each group
    #[repr(C)]
    struct Asked {
        count: u16,
        reserved: u16,
        common: u32,
        volume: u32,
        folder: u32,
        file: u32,
        fork: u32,
    }
    extern "C" {
        fn getattrlistbulk(folder: i32, asked: *const Asked, into: *mut u8, size: usize, options: u64) -> i32;
    }
    const NAME: u32 = 0x0000_0001;
    const ADDED: u32 = 0x1000_0000;
    const ERROR: u32 = 0x2000_0000;
    const RETURNED: u32 = 0x8000_0000;
    let mut found = HashMap::new();
    let Ok(folder) = fs::File::open(dir) else { return found };
    let asked = Asked { count: 5, reserved: 0, common: RETURNED | ERROR | NAME | ADDED, volume: 0, folder: 0, file: 0, fork: 0 };
    let mut buffer = vec![0u8; 64 * 1024];
    let word = |b: &[u8], at: usize| -> Option<u32> { Some(u32::from_ne_bytes(b.get(at..at + 4)?.try_into().ok()?)) };
    let long = |b: &[u8], at: usize| -> Option<i64> { Some(i64::from_ne_bytes(b.get(at..at + 8)?.try_into().ok()?)) };
    loop {
        let count = unsafe { getattrlistbulk(folder.as_raw_fd(), &asked, buffer.as_mut_ptr(), buffer.len(), 0) };
        if count <= 0 {
            return found;
        }
        let mut start = 0;
        for _ in 0..count {
            // each item: its length, the five groups of attributes returned, then those asked for in sys/attr.h's
            // order, each only when returned: an error, the name's reference, the time added
            let Some(length) = word(&buffer, start).map(|n| n as usize).filter(|n| *n >= 4) else { break };
            let item = buffer.get(start..start + length).unwrap_or_default();
            start += length;
            let Some(returned) = word(item, 4) else { continue };
            let mut at = 4 + 5 * 4;
            if returned & ERROR != 0 {
                if word(item, at) != Some(0) {
                    continue;
                }
                at += 4;
            }
            if returned & NAME == 0 || returned & ADDED == 0 {
                continue;
            }
            // the name's place counts from its reference, and its length takes in the nul that ends it
            let (Some(offset), Some(len)) = (word(item, at), word(item, at + 4)) else { continue };
            let from = at + offset as usize;
            let Some(name) = item.get(from..from + (len as usize).saturating_sub(1)) else { continue };
            let (Some(secs), Some(nanos)) = (long(item, at + 8), long(item, at + 16)) else { continue };
            if let (Ok(secs), Ok(nanos)) = (u64::try_from(secs), u32::try_from(nanos)) {
                found.insert(std::ffi::OsStr::from_bytes(name).to_os_string(), UNIX_EPOCH + Duration::new(secs, nanos));
            }
        }
    }
}

/// (folder, hidden, away), from what the listing says.
#[cfg(windows)]
fn flags(name: &str, meta: &Metadata) -> (bool, bool, bool) {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN, FILE_ATTRIBUTE_SYSTEM};
    let a = meta.file_attributes();
    // onedrive, dropbox and the like mark a file they have not downloaded with the recall bits; older tools with
    // offline. only a folder listing shows the recall bits, which is where this comes from
    let away = a & (FILE_ATTRIBUTE_OFFLINE.0 | FILE_ATTRIBUTE_RECALL_ON_OPEN.0 | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS.0) != 0;
    // the directory bit also marks junctions and folder links, which std's is_dir() counts as symlinks.
    // a leading dot hides the ._ and .DS_Store files a mac leaves on shared drives
    (a & FILE_ATTRIBUTE_DIRECTORY.0 != 0, a & (FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0) != 0 || name.starts_with('.'), away)
}

#[cfg(target_os = "macos")]
fn flags(name: &str, meta: &Metadata) -> (bool, bool, bool) {
    use std::os::macos::fs::MetadataExt;
    let (hidden, away) = marks(meta.st_flags());
    (meta.is_dir(), hidden || name.starts_with('.'), away)
}

/// (hidden, away), from a file's flags on macos: finder's hidden mark, and the mark of a file whose data is not on
/// the disk (icloud drive and the like keep such files as names only; reading one downloads it).
#[cfg(target_os = "macos")]
fn marks(flags: u32) -> (bool, bool) {
    const HIDDEN: u32 = 0x8000;
    const DATALESS: u32 = 0x4000_0000;
    (flags & HIDDEN != 0, flags & DATALESS != 0)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn flags(name: &str, meta: &Metadata) -> (bool, bool, bool) {
    (meta.is_dir(), name.starts_with('.'), false)
}

pub fn is_media(name: &str) -> bool {
    has_ext(name, CLIPS) || has_ext(name, STILLS)
}

/// a still image by its extension: exr, dpx, png, jpeg, tiff, tga
pub fn is_still(name: &str) -> bool {
    has_ext(name, STILLS)
}

fn has_ext(name: &str, exts: &[&str]) -> bool {
    Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// one still that may belong to a run
struct Frame<'a> {
    n: u64,
    digits: &'a str,
    /// its place in the folder's files
    at: usize,
}

/// a still's name split at the last run of digits before its extension: ((before, extension with its dot), frame)
fn numbered(name: &str, at: usize) -> Option<((&str, &str), Frame<'_>)> {
    if !has_ext(name, STILLS) {
        return None;
    }
    let (stem, ext) = name.split_at(name.rfind('.')?);
    let before = stem.trim_end_matches(|c: char| c.is_ascii_digit());
    let digits = &stem[before.len()..];
    // no digits, or too many for a u64: a name, not a frame number
    Some(((before, ext), Frame { n: digits.parse().ok()?, digits, at }))
}

/// a numbered still's name split at its frame number: (the text before it, the number, the extension with its dot).
/// frame n of its sequence is `{before}{n:0pad$}{ext}`, in the same folder.
pub fn frame_number(name: &str) -> Option<(&str, u64, &str)> {
    let ((before, ext), frame) = numbered(name, 0)?;
    Some((before, frame.n, ext))
}

/// the width that writes every frame's digits back as `{n:0pad$}`: the padded width when a number is zero-padded
/// or all share one width (1001-1240), else 0. None when no one width does: a.1.exr beside a.01.exr.
fn pad_of(run: &[Frame]) -> Option<usize> {
    let zero_led = |f: &&Frame| f.digits.len() > 1 && f.digits.starts_with('0');
    let width = run.iter().map(|f| f.digits.len()).min()?;
    if run.iter().filter(zero_led).any(|f| f.digits.len() != width) {
        return None;
    }
    let padded = run.iter().any(|f| zero_led(&f)) || (width > 1 && run.iter().all(|f| f.digits.len() == width));
    Some(if padded { width } else { 0 })
}

/// folds each numbered run of stills into one entry; everything else stays as it is.
fn fold_sequences(files: Vec<Entry>) -> Vec<Entry> {
    let mut runs: HashMap<(&str, &str), Vec<Frame>> = HashMap::new();
    for (at, file) in files.iter().enumerate() {
        if let Some((key, frame)) = numbered(&file.name, at) {
            runs.entry(key).or_default().push(frame);
        }
    }
    let mut taken = vec![false; files.len()];
    let mut out = Vec::new();
    for ((before, ext), run) in runs {
        let parts = if pad_of(&run).is_some() {
            vec![run]
        } else {
            // mixed padding: each width is its own run
            let mut widths: HashMap<usize, Vec<Frame>> = HashMap::new();
            for frame in run {
                widths.entry(frame.digits.len()).or_default().push(frame);
            }
            widths.into_values().collect()
        };
        for mut part in parts {
            if let Some(seq) = sequence(before, ext, &mut part, &files) {
                for f in &part {
                    taken[f.at] = true;
                }
                out.push(seq);
            }
        }
    }
    out.extend(files.into_iter().zip(taken).filter(|(_, taken)| !taken).map(|(file, _)| file));
    out
}

/// one run as a single entry, or None when it is not a sequence after all: one frame, or numbers too far apart.
fn sequence(before: &str, ext: &str, run: &mut [Frame], files: &[Entry]) -> Option<Entry> {
    let pad = pad_of(run)?;
    run.sort_unstable_by_key(|f| f.n);
    let (first, last) = (run.first()?.n, run.last()?.n);
    if run.len() < 2 || last - first >= SPAN {
        return None;
    }
    let missing = run.windows(2).flat_map(|p| p[0].n + 1..p[1].n).collect();
    let members = || run.iter().map(|f| &files[f.at]);
    Some(Entry {
        name: format!("{before}[{first:0pad$}-{last:0pad$}]{ext}"),
        path: files[run[0].at].path.clone(),
        kind: Kind::Sequence { first, last, missing, pad },
        size: Some(members().filter_map(|f| f.size).sum()),
        modified: members().filter_map(|f| f.modified).max(),
        created: members().filter_map(|f| f.created).min(),
        added: None,
        away: members().any(|f| f.away),
        link: false,
    })
}

/// names in the order a person expects: numbers by value (v9 before v10), letters without case.
/// names that differ only in case or leading zeros still get a fixed order.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a, b);
    while let (Some(p), Some(q)) = (x.chars().next(), y.chars().next()) {
        let order = if p.is_ascii_digit() && q.is_ascii_digit() {
            let (m, rest) = split_digits(x);
            x = rest;
            let (n, rest) = split_digits(y);
            y = rest;
            let (m, n) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
            // length first: a run of any length compares without becoming an integer
            m.len().cmp(&n.len()).then(m.cmp(n))
        } else {
            x = &x[p.len_utf8()..];
            y = &y[q.len_utf8()..];
            p.to_lowercase().cmp(q.to_lowercase())
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    y.is_empty().cmp(&x.is_empty()).then_with(|| a.cmp(b))
}

fn split_digits(s: &str) -> (&str, &str) {
    s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// a folder under the temp folder that removes what it made one item at a time, never recursively
    pub(crate) struct Scratch {
        pub(crate) dir: PathBuf,
        /// (path, is a folder), in the order made
        pub(crate) made: Vec<(PathBuf, bool)>,
    }

    impl Scratch {
        pub(crate) fn new(tag: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("eyepiece-browse-{tag}-{}", std::process::id()));
            fs::create_dir(&dir).unwrap();
            Scratch { made: vec![(dir.clone(), true)], dir }
        }

        fn file(&mut self, name: &str, bytes: usize) {
            let path = self.dir.join(name);
            fs::write(&path, vec![0u8; bytes]).unwrap();
            self.made.push((path, false));
        }

        /// files of ten bytes, `name` a path inside the scratch folder ("dome/a.mov")
        pub(crate) fn files(&mut self, names: &[&str]) {
            names.iter().for_each(|name| self.file(name, 10));
        }

        pub(crate) fn folder(&mut self, name: &str) {
            let path = self.dir.join(name);
            fs::create_dir(&path).unwrap();
            self.made.push((path, true));
        }

        fn names(&self) -> Vec<String> {
            list(&self.dir).unwrap().into_iter().map(|e| e.name).collect()
        }

        fn entry(&self, name: &str) -> Entry {
            list(&self.dir).unwrap().into_iter().find(|e| e.name == name).unwrap_or_else(|| panic!("no entry {name}"))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for (path, folder) in self.made.iter().rev() {
                let removed = if *folder { fs::remove_dir(path) } else { fs::remove_file(path) };
                if let Err(why) = removed {
                    // a second panic while a failed test unwinds would abort the run, so then it is only reported
                    if std::thread::panicking() {
                        eprintln!("left behind {}: {why}", path.display());
                    } else {
                        panic!("could not remove {}: {why}", path.display());
                    }
                }
            }
        }
    }

    fn seq(first: u64, last: u64, missing: &[u64], pad: usize) -> Kind {
        Kind::Sequence { first, last, missing: missing.to_vec(), pad }
    }

    #[test]
    fn folders_first_then_files_in_natural_order() {
        let mut s = Scratch::new("order");
        for name in ["dir10", "b_dir", "dir9", "A_dir"] {
            s.folder(name);
        }
        s.file("z.txt", 3);
        s.file("clip_v10.mov", 7);
        s.files(&["clip_v9.mov", "B.png", "a.mov"]);
        assert_eq!(s.names(), ["A_dir", "b_dir", "dir9", "dir10", "a.mov", "B.png", "clip_v9.mov", "clip_v10.mov", "z.txt"]);
        let dir = s.entry("dir9");
        assert_eq!((dir.kind, dir.size, dir.path), (Kind::Folder, None, s.dir.join("dir9")));
        let clip = s.entry("clip_v10.mov");
        assert_eq!((clip.kind, clip.size), (Kind::Clip, Some(7)));
        assert!(clip.modified.is_some());
        assert_eq!(s.entry("B.png").kind, Kind::Clip);
        assert_eq!(s.entry("z.txt").kind, Kind::Other);
    }

    #[cfg(windows)]
    #[test]
    fn hidden_and_system_files_are_left_out() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
        let mut s = Scratch::new("hidden");
        s.files(&["shown.mov", ".DS_Store", "._shown.mov"]);
        for (name, attributes) in [("hidden.mov", FILE_ATTRIBUTE_HIDDEN), ("system.mov", FILE_ATTRIBUTE_SYSTEM)] {
            let path = s.dir.join(name);
            fs::OpenOptions::new().write(true).create_new(true).attributes(attributes.0).open(&path).unwrap();
            s.made.push((path, false));
        }
        assert_eq!(s.names(), ["shown.mov"]);
    }

    #[cfg(windows)]
    #[test]
    fn a_file_whose_data_is_not_on_the_disk_says_so() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_OFFLINE;
        let mut s = Scratch::new("away");
        s.files(&["here.mov", "run.0001.png", "run.0002.png"]);
        for name in ["cloud.mov", "run.0003.png"] {
            let path = s.dir.join(name);
            fs::OpenOptions::new().write(true).create_new(true).attributes(FILE_ATTRIBUTE_OFFLINE.0).open(&path).unwrap();
            s.made.push((path, false));
        }
        assert!(!s.entry("here.mov").away);
        assert!(s.entry("cloud.mov").away);
        // a sequence with one file away is away
        assert!(s.entry("run.[0001-0003].png").away);
    }

    #[test]
    fn a_numbered_run_of_stills_is_one_sequence_with_its_gaps() {
        let mut s = Scratch::new("sequence");
        s.files(&["shot_010.0001.exr", "shot_010.0002.exr", "shot_010.0004.exr", "shot_010.0005.exr"]);
        // beside it: another run, other separators, another extension, a lone numbered still, numbered clips
        s.files(&["shot_020_0001.exr", "shot_020_0002.exr", "plate-1001.dpx", "plate-1002.dpx", "ref1.png", "ref2.png"]);
        s.files(&["shot_010.0003.jpg", "frame_0042.png", "take_v001.mov", "take_v002.mov"]);
        assert_eq!(
            s.names(),
            [
                "frame_0042.png",
                "plate-[1001-1002].dpx",
                "ref[1-2].png",
                "shot_010.0003.jpg",
                "shot_010.[0001-0005].exr",
                "shot_020_[0001-0002].exr",
                "take_v001.mov",
                "take_v002.mov",
            ]
        );
        let run = s.entry("shot_010.[0001-0005].exr");
        assert_eq!(run.kind, seq(1, 5, &[3], 4));
        assert_eq!((run.size, run.path), (Some(40), s.dir.join("shot_010.0001.exr")));
        assert!(run.modified.is_some());
        // one width throughout counts as padded to it
        assert_eq!(s.entry("plate-[1001-1002].dpx").kind, seq(1001, 1002, &[], 4));
        assert_eq!(s.entry("frame_0042.png").kind, Kind::Clip);
        assert_eq!(s.entry("shot_010.0003.jpg").kind, Kind::Clip);
        assert_eq!(s.entry("take_v001.mov").kind, Kind::Clip);
    }

    #[test]
    fn padding_is_the_width_that_writes_every_number_back() {
        let mut s = Scratch::new("padding");
        // not padded, one sequence
        s.files(&["a.8.exr", "a.9.exr", "a.10.exr"]);
        // padded to four, with a number that outgrew it
        s.files(&["b.0999.exr", "b.10000.exr"]);
        // two paddings side by side: two sequences, never one with frame 1 twice
        s.files(&["c.1.exr", "c.2.exr", "c.01.exr", "c.02.exr"]);
        assert_eq!(s.names(), ["a.[8-10].exr", "b.[0999-10000].exr", "c.[01-02].exr", "c.[1-2].exr"]);
        assert_eq!(s.entry("a.[8-10].exr").kind, seq(8, 10, &[], 0));
        let Kind::Sequence { first, last, missing, pad } = s.entry("b.[0999-10000].exr").kind else { panic!("not a sequence") };
        assert_eq!((first, last, missing.len(), pad), (999, 10000, 9000, 4));
        assert_eq!(s.entry("c.[1-2].exr").kind, seq(1, 2, &[], 0));
        assert_eq!(s.entry("c.[01-02].exr").kind, seq(1, 2, &[], 2));
    }

    #[test]
    fn numbers_too_far_apart_are_not_a_sequence() {
        let mut s = Scratch::new("span");
        s.files(&["scan_1.png", "scan_20261123.png"]);
        assert_eq!(s.names(), ["scan_1.png", "scan_20261123.png"]);
        assert_eq!(s.entry("scan_1.png").kind, Kind::Clip);
    }

    #[test]
    fn natural_order() {
        let mut names = vec!["v10", "x100", "V9", "file2", "v9b", "B", "file02", "a", "x20", "file1", "n100000000000000000000", "n99999999999999999999"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["a", "B", "file1", "file02", "file2", "n99999999999999999999", "n100000000000000000000", "V9", "v9b", "v10", "x20", "x100"]);
        assert_eq!(natural_cmp("a", "A"), "a".cmp("A"));
    }

    /// every key both ways: folders stay before files, each group in the sort's order; what the listing does not have
    /// (a folder's size, a date not recorded) comes last whichever way it runs; ties and name go in natural order.
    #[test]
    fn a_folder_sorts_by_each_key_both_ways() {
        let at = |secs: Option<u64>| secs.map(|s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s));
        // name, kind, size, modified, created, added
        let made = |name: &str, kind: Kind, size: Option<u64>, [modified, created, added]: [Option<u64>; 3]| Entry {
            name: name.into(),
            path: PathBuf::from(name),
            kind,
            size,
            modified: at(modified),
            created: at(created),
            added: at(added),
            away: false,
            link: false,
        };
        let entries = || {
            vec![
                made("notes.txt", Kind::Other, None, [Some(5), Some(10), Some(40)]),
                made("b_dir", Kind::Folder, None, [Some(30), Some(10), None]),
                made("clip 10.mov", Kind::Clip, Some(300), [None, Some(30), Some(10)]),
                made("plate.[1001-1002].dpx", seq(1001, 1002, &[], 4), Some(200), [Some(40), Some(5), Some(30)]),
                made("a_dir", Kind::Folder, None, [Some(20), None, Some(5)]),
                made("clip 2.mov", Kind::Clip, Some(100), [Some(20), Some(20), None]),
                made("a.mov", Kind::Clip, Some(100), [Some(10), None, Some(20)]),
            ]
        };
        let plate = "plate.[1001-1002].dpx";
        let cases: [(By, bool, [&str; 7]); 10] = [
            (By::Name, false, ["a_dir", "b_dir", "a.mov", "clip 2.mov", "clip 10.mov", "notes.txt", plate]),
            (By::Name, true, ["b_dir", "a_dir", plate, "notes.txt", "clip 10.mov", "clip 2.mov", "a.mov"]),
            (By::Size, false, ["a_dir", "b_dir", "a.mov", "clip 2.mov", plate, "clip 10.mov", "notes.txt"]),
            (By::Size, true, ["a_dir", "b_dir", "clip 10.mov", plate, "a.mov", "clip 2.mov", "notes.txt"]),
            (By::Modified, false, ["a_dir", "b_dir", "notes.txt", "a.mov", "clip 2.mov", plate, "clip 10.mov"]),
            (By::Modified, true, ["b_dir", "a_dir", plate, "clip 2.mov", "a.mov", "notes.txt", "clip 10.mov"]),
            (By::Created, false, ["b_dir", "a_dir", plate, "notes.txt", "clip 2.mov", "clip 10.mov", "a.mov"]),
            (By::Created, true, ["b_dir", "a_dir", "clip 10.mov", "clip 2.mov", "notes.txt", plate, "a.mov"]),
            (By::Added, false, ["a_dir", "b_dir", "clip 10.mov", "a.mov", plate, "notes.txt", "clip 2.mov"]),
            (By::Added, true, ["a_dir", "b_dir", "notes.txt", plate, "a.mov", "clip 10.mov", "clip 2.mov"]),
        ];
        for (by, descending, wanted) in cases {
            let mut sorted = entries();
            sort(&mut sorted, Sort { by, descending });
            assert_eq!(sorted.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), wanted, "by {by:?}, descending {descending}");
        }
        // the menu offers date added where the system keeps it
        assert_eq!(SORTS.iter().filter(|(_, by)| by.kept()).count(), if cfg!(target_os = "macos") { 5 } else { 4 });
    }

    /// the listing says when each entry was made (on windows from the listing's own data, on macos its birth time),
    /// and on macos when it was put in the folder, read with a listing in bulk. windows keeps no date added. a wrong
    /// place in what the system returns would give a date far from now.
    #[test]
    fn the_listing_says_when_each_entry_was_made_and_on_macos_added() {
        let mut s = Scratch::new("dates");
        let since = SystemTime::now() - std::time::Duration::from_secs(5);
        s.folder("dome");
        s.files(&["a.mov", "run.0001.png", "run.0002.png"]);
        let until = SystemTime::now() + std::time::Duration::from_secs(5);
        let mut entries = list(&s.dir).unwrap();
        read_added(&s.dir, &mut entries);
        assert_eq!(entries.len(), 3);
        let now = |t: Option<SystemTime>| t.is_some_and(|t| t >= since && t <= until);
        for entry in &entries {
            assert!(now(entry.created), "{}: made {:?}", entry.name, entry.created);
            assert_eq!(now(entry.added), cfg!(target_os = "macos"), "{}: added {:?}", entry.name, entry.added);
        }
    }

    #[test]
    fn media_by_extension_without_case() {
        assert!(is_media("a.MOV") && is_media("b.Exr") && is_media("c.webm") && is_media("d.jpeg"));
        assert!(!is_media("e.pdf") && !is_media("mov") && !is_media(".mov"));
    }

    /// a file whose data is in the cloud and not on the disk is away, by the mark the system gives it (SF_DATALESS
    /// in sys/stat.h), and finder's hidden mark hides; a file made here has neither.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_file_not_on_the_disk_is_away() {
        assert_eq!((marks(0), marks(0x4000_0000), marks(0x8000), marks(0x4000_8040)), ((false, false), (false, true), (true, false), (true, true)));
        let listed = list(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/media"))).unwrap();
        assert!(listed.iter().any(|entry| entry.name == "hap_64.mov") && listed.iter().all(|entry| !entry.away));
    }

    /// the startup volume and what is on it are local; what no mount point holds is taken for network.
    #[cfg(target_os = "macos")]
    #[test]
    fn volume_of_the_startup_disk_and_of_what_is_nowhere() {
        assert_eq!(volume_of(Path::new("/")), Volume::Local);
        assert_eq!(volume_of(Path::new(env!("CARGO_MANIFEST_DIR"))), Volume::Local);
        assert_eq!(volume_of(Path::new("/no/such/folder")), Volume::Local);
        assert_eq!(volume_of(Path::new("relative")), Volume::Network);
        assert!(mounts().iter().any(|(at, _)| at == Path::new("/")));
        assert!(places().iter().any(|place| place.path == Path::new("/") && place.group == Group::Volumes));
    }

    #[cfg(windows)]
    #[test]
    fn volume_of_drives_and_shares() {
        use windows::Win32::Storage::FileSystem::GetLogicalDrives;
        assert_eq!(volume_of(Path::new(r"C:\")), Volume::Local);
        // a folder that does not exist is on the drive its path names
        assert_eq!(volume_of(Path::new(r"C:\no\such\folder")), Volume::Local);
        // decided from the string alone: nothing is asked of a network
        assert_eq!(volume_of(Path::new(r"\\server\share\x")), Volume::Network);
        assert_eq!(volume_of(Path::new(r"\\?\UNC\server\share\x")), Volume::Network);
        assert_eq!(volume_of(Path::new("relative")), Volume::Network);
        let used = unsafe { GetLogicalDrives() };
        let free = (0..26u8).find(|i| used & (1 << i) == 0).expect("a drive letter nothing is on");
        assert_eq!(volume_of(&PathBuf::from(format!("{}:\\x", (b'A' + free) as char))), Volume::Network);
    }

    /// this pc's own: what fails names no folder or label of it, only counts and places in the list
    #[cfg(windows)]
    #[test]
    fn places_are_the_known_folders_then_the_drives() {
        let places = places();
        let at = |group| places.iter().enumerate().filter(move |(_, p)| p.group == group);
        assert_eq!(at(Group::Known).count() + at(Group::Volumes).count(), places.len());
        assert!(places.windows(2).all(|p| p[0].group <= p[1].group), "the groups are out of order");
        let known: Vec<&str> = at(Group::Known).map(|(_, p)| p.name.as_str()).collect();
        assert_eq!(known.first(), Some(&"downloads"));
        let order = ["downloads", "desktop", "documents", "videos", "pictures"];
        assert_eq!(known, order.into_iter().filter(|n| known.contains(n)).collect::<Vec<_>>());
        for (i, drive) in at(Group::Volumes) {
            let root = drive.path.to_string_lossy();
            assert!(root.len() == 3 && root.ends_with(":\\"), "volume {i} is not a drive's root");
            assert!(drive.name == root[..2] || drive.name.ends_with(&format!(" ({}:)", &root[..1])), "volume {i}: its name does not end with its letter");
        }
        assert!(at(Group::Volumes).any(|(_, p)| p.path == Path::new(r"C:\") && p.name == "system (C:)"));
    }

    /// this pc's own pins, if it has any: a folder on a drive or a share, under its own name
    #[cfg(windows)]
    #[test]
    fn a_folder_pinned_in_explorer_is_a_path_under_its_own_name() {
        for (i, pin) in pinned().iter().enumerate() {
            assert!(matches!(pin.path.components().next(), Some(std::path::Component::Prefix(_))), "pinned folder {i} is no path");
            assert!(pin.group == Group::Pinned && pin.name == name_of(&pin.path), "pinned folder {i}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_is_named_by_its_label_and_letter() {
        assert_eq!(drive_name('C', Some("OS"), true), "system (C:)");
        assert_eq!(drive_name('D', Some("media"), false), "media (D:)");
        // no label, or one not read (a share's)
        assert_eq!((drive_name('E', Some(""), false), drive_name('Z', None, false)), ("E:".to_string(), "Z:".to_string()));
        assert_eq!((name_of(Path::new(r"D:\renders_out")), name_of(Path::new(r"D:\")), name_of(Path::new(r"\\nas\projects\"))), ("renders_out".to_string(), "D:".to_string(), r"\\nas\projects".to_string()));
    }
}
