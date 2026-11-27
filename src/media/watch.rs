//! noticing that the open file changed on disk. after a rename-over or a delete and recreate an open handle goes on
//! reading the old file (gate 5), so this looks at the path every time and never at a handle.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// how long a new size and time must hold still before they count, so a render still writing is left alone
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    None,
    /// the file is a different one now and has stopped changing: reopen it by its path
    Changed,
    /// the file has not been seen for half a second: moved, deleted, or its volume lost
    Gone,
}

/// what one look at the path shows
#[derive(Clone, Copy, PartialEq, Eq)]
struct Look {
    len: u64,
    modified: SystemTime,
    /// which file the name points at. a folder listing does not say, so on a network volume it is none
    id: Option<(u64, u64)>,
}

pub struct Watch {
    path: PathBuf,
    network: bool,
    /// the file as it was when watching began, or as last reported; none while it cannot be seen
    known: Option<Look>,
    /// a look unlike `known`, and when it was first seen
    seen: Option<(Option<Look>, Instant)>,
}

impl Watch {
    /// `network` says the file is on a network volume: then only what a folder listing gives is used
    /// (name, size, modified time), and the file itself is never opened or asked about.
    pub fn new(path: &Path, network: bool) -> Watch {
        Watch { path: path.to_path_buf(), network, known: look(path, network), seen: None }
    }

    /// call a few times a second. `Changed` is reported once, after the file has stopped changing:
    /// its size and time must hold still across two looks at least half a second apart, so a render
    /// that is still writing is left alone until it stops. `Gone` waits the same half second, so the
    /// moment between a delete and a recreate is not reported, and is also reported once.
    pub fn poll(&mut self) -> Change {
        self.poll_at(Instant::now())
    }

    fn poll_at(&mut self, now: Instant) -> Change {
        let look = look(&self.path, self.network);
        if look == self.known {
            self.seen = None;
            return Change::None;
        }
        match self.seen {
            Some((seen, since)) if seen == look => {
                if now.duration_since(since) < SETTLE {
                    return Change::None;
                }
                self.known = look;
                self.seen = None;
                if look.is_some() {
                    Change::Changed
                } else {
                    Change::Gone
                }
            }
            _ => {
                self.seen = Some((look, now));
                Change::None
            }
        }
    }
}

/// a file's size and when it was last written, as its folder lists them: the file itself is not opened or asked about
pub fn listed_stamp(path: &Path) -> Option<(u64, SystemTime)> {
    listed(path).map(|look| (look.len, look.modified))
}

/// none when the file cannot be seen, for whatever reason: missing, or its volume unreachable
fn look(path: &Path, network: bool) -> Option<Look> {
    if network {
        listed(path)
    } else {
        local(path)
    }
}

/// from the parent folder's listing alone, asked for this one name: an entry's size and time come with the listing, so
/// the file is not opened and not asked about (an open over smb changes its ctime on the owner's nas, adr 0004), and
/// a look costs the same whatever the folder holds.
#[cfg(windows)]
fn listed(path: &Path) -> Option<Look> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{FindClose, FindFirstFileW, WIN32_FIND_DATAW};
    let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut entry = WIN32_FIND_DATAW::default();
    let found = unsafe { FindFirstFileW(PCWSTR(name.as_ptr()), &mut entry) }.ok()?;
    let _ = unsafe { FindClose(found) };
    let whole = |high: u32, low: u32| ((high as u64) << 32) | low as u64;
    // the time is in 100 ns steps from 1601
    let since_1970 = whole(entry.ftLastWriteTime.dwHighDateTime, entry.ftLastWriteTime.dwLowDateTime).checked_sub(116_444_736_000_000_000)?;
    Some(Look { len: whole(entry.nFileSizeHigh, entry.nFileSizeLow), modified: SystemTime::UNIX_EPOCH + Duration::from_nanos(since_1970.checked_mul(100)?), id: None })
}

/// from the parent folder's listing alone.
#[cfg(not(windows))]
fn listed(path: &Path) -> Option<Look> {
    let name = path.file_name()?;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    // names on windows and on smb shares ignore case
    let entry = std::fs::read_dir(dir).ok()?.flatten().find(|e| e.file_name().eq_ignore_ascii_case(name))?;
    let m = entry.metadata().ok()?;
    Some(Look { len: m.len(), modified: m.modified().ok()?, id: None })
}

#[cfg(windows)]
fn local(path: &Path) -> Option<Look> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    // no access asked at all: a writer's sharing mode cannot refuse it, even one that shares nothing, and it holds
    // nothing up. rust's own file index is not stable yet, hence the system call
    let file = std::fs::OpenOptions::new().access_mode(0).open(path).ok()?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.ok()?;
    let m = file.metadata().ok()?;
    let index = ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64;
    Some(Look { len: m.len(), modified: m.modified().ok()?, id: Some((info.dwVolumeSerialNumber as u64, index)) })
}

#[cfg(unix)]
fn local(path: &Path) -> Option<Look> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some(Look { len: m.len(), modified: m.modified().ok()?, id: Some((m.dev(), m.ino())) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    /// a folder of its own for one case
    fn scratch(case: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eyepiece-watch-{case}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// removes the clip every case ends with, then the empty folder. a failed case leaves them for a look
    fn tidy(dir: &Path) {
        fs::remove_file(dir.join("clip.mov")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    /// runs a case on a local file, then again as if the folder were on a network volume
    fn both(case: &str, run: impl Fn(&Path, bool)) {
        for network in [false, true] {
            let dir = scratch(&format!("{case}-{network}"));
            run(&dir.join("clip.mov"), network);
            tidy(&dir);
        }
    }

    #[test]
    fn overwrite_in_place() {
        both("overwrite", |clip, network| {
            fs::write(clip, b"render one").unwrap();
            let mut watch = Watch::new(clip, network);
            assert_eq!(watch.poll(), Change::None);
            let t = Instant::now();
            fs::write(clip, b"render two, longer").unwrap();
            assert_eq!(watch.poll_at(t), Change::None);
            assert_eq!(watch.poll_at(at(t, 400)), Change::None);
            assert_eq!(watch.poll_at(at(t, 500)), Change::Changed);
            assert_eq!(watch.poll_at(at(t, 1000)), Change::None);
        });
    }

    #[test]
    fn rename_over_with_the_same_size_and_time() {
        let dir = scratch("rename");
        let (clip, tmp) = (dir.join("clip.mov"), dir.join("clip.tmp"));
        fs::write(&clip, b"render one").unwrap();
        let mut watch = Watch::new(&clip, false);
        let before = fs::metadata(&clip).unwrap();
        fs::write(&tmp, b"render two").unwrap();
        fs::File::options().write(true).open(&tmp).unwrap().set_modified(before.modified().unwrap()).unwrap();
        fs::rename(&tmp, &clip).unwrap();
        let after = fs::metadata(&clip).unwrap();
        // only which file the name points at tells the two apart
        assert_eq!((after.len(), after.modified().unwrap()), (before.len(), before.modified().unwrap()));
        let t = Instant::now();
        assert_eq!(watch.poll_at(t), Change::None);
        assert_eq!(watch.poll_at(at(t, 500)), Change::Changed);
        tidy(&dir);
    }

    #[test]
    fn delete_and_recreate() {
        both("recreate", |clip, network| {
            fs::write(clip, b"render one").unwrap();
            let mut watch = Watch::new(clip, network);
            let t = Instant::now();
            fs::remove_file(clip).unwrap();
            assert_eq!(watch.poll_at(t), Change::None);
            fs::write(clip, b"render two, longer").unwrap();
            // it was missing for less than half a second, which is not reported
            assert_eq!(watch.poll_at(at(t, 300)), Change::None);
            assert_eq!(watch.poll_at(at(t, 700)), Change::None);
            assert_eq!(watch.poll_at(at(t, 800)), Change::Changed);
        });
    }

    #[test]
    fn a_growing_file_is_left_alone_until_it_stops() {
        both("growing", |clip, network| {
            fs::write(clip, b"").unwrap();
            let mut watch = Watch::new(clip, network);
            let t = Instant::now();
            // looks further apart than half a second, with the file grown before each
            for i in 0..4 {
                fs::OpenOptions::new().append(true).open(clip).unwrap().write_all(b"frame").unwrap();
                assert_eq!(watch.poll_at(at(t, 600 * i)), Change::None);
            }
            assert_eq!(watch.poll_at(at(t, 1800 + 400)), Change::None);
            assert_eq!(watch.poll_at(at(t, 1800 + 500)), Change::Changed);
        });
    }

    #[test]
    fn a_file_that_goes_away_and_comes_back() {
        both("gone", |clip, network| {
            fs::write(clip, b"render one").unwrap();
            let mut watch = Watch::new(clip, network);
            let t = Instant::now();
            fs::remove_file(clip).unwrap();
            assert_eq!(watch.poll_at(t), Change::None);
            assert_eq!(watch.poll_at(at(t, 500)), Change::Gone);
            assert_eq!(watch.poll_at(at(t, 1000)), Change::None);
            fs::write(clip, b"render one").unwrap();
            assert_eq!(watch.poll_at(at(t, 1100)), Change::None);
            assert_eq!(watch.poll_at(at(t, 1600)), Change::Changed);
        });
    }
}
