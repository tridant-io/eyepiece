//! putting a file written whole in the place of another, for the notes (`crate::notes`): there is no moment with
//! half a file under the name, or none.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MOVE_FILE_FLAGS, REPLACE_FILE_FLAGS};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// renames `new` to `file`, and returns once that is on the disk
fn rename(new: &Path, file: &Path, flags: MOVE_FILE_FLAGS) -> io::Result<()> {
    // the system's own error, which says a name was taken as AlreadyExists
    unsafe { MoveFileExW(PCWSTR(wide(new).as_ptr()), PCWSTR(wide(file).as_ptr()), flags | MOVEFILE_WRITE_THROUGH) }.map_err(|_| io::Error::last_os_error())
}

/// puts `new` in `file`'s place. with `previous`, `file` is there and has been read: it is replaced in one step, and
/// kept under that name. with none, `file` is not there: if it has come meanwhile nothing is replaced, and this fails
/// with AlreadyExists.
pub fn swap_in(new: &Path, file: &Path, previous: Option<&Path>) -> io::Result<()> {
    let Some(previous) = previous else { return rename(new, file, MOVE_FILE_FLAGS(0)) };
    let replaced = unsafe { ReplaceFileW(PCWSTR(wide(file).as_ptr()), PCWSTR(wide(new).as_ptr()), PCWSTR(wide(previous).as_ptr()), REPLACE_FILE_FLAGS(0), None, None) };
    replaced.or_else(|_| in_two_steps(new, file, previous))
}

/// for a volume that does not replace a file and keep the old one in one step (some shares): the old version is
/// copied aside first, then the new one is renamed over it, which is one step. a replace that failed halfway may have
/// left the old version under its `previous` name already, with no `file`.
fn in_two_steps(new: &Path, file: &Path, previous: &Path) -> io::Result<()> {
    if file.exists() {
        std::fs::copy(file, previous)?;
    }
    rename(new, file, MOVEFILE_REPLACE_EXISTING)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// a folder of the test's own with a new file, the file it is to replace, and where the old one is kept
    fn scratch(test: &str) -> [std::path::PathBuf; 4] {
        let dir = std::env::temp_dir().join(format!("eyepiece-swap-{test}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("new"), "new").unwrap();
        [dir.join("new"), dir.join("file"), dir.join("file.previous"), dir]
    }

    /// removes what the test made, a file at a time, then the folder, which must then be empty
    fn tidy([new, file, previous, dir]: [std::path::PathBuf; 4]) {
        for made in [new, file, previous].iter().filter(|made| made.exists()) {
            fs::remove_file(made).unwrap();
        }
        fs::remove_dir(dir).unwrap();
    }

    /// a file that is there is replaced whole and kept as its previous version, by either way of doing it; an older
    /// previous version gives way.
    #[test]
    fn a_file_is_replaced_whole_and_the_old_one_kept() {
        for (way, swap) in [("one step", swap_in as fn(&Path, &Path, Option<&Path>) -> io::Result<()>), ("two steps", |new, file, previous| in_two_steps(new, file, previous.unwrap()))] {
            let made = scratch(&way.replace(' ', "-"));
            let [new, file, previous, _] = &made;
            fs::write(file, "old").unwrap();
            fs::write(previous, "older").unwrap();
            swap(new, file, Some(previous)).unwrap_or_else(|e| panic!("{way}: {e}"));
            assert_eq!((fs::read_to_string(file).unwrap(), fs::read_to_string(previous).unwrap(), new.exists()), ("new".to_string(), "old".to_string(), false), "{way}");
            tidy(made);
        }
    }

    /// with no file there the new one takes the name; a file that came meanwhile is not replaced.
    #[test]
    fn a_new_file_takes_a_free_name_and_never_one_that_is_taken() {
        let made = scratch("free");
        let [new, file, previous, _] = &made;
        swap_in(new, file, None).unwrap();
        assert_eq!((fs::read_to_string(file).unwrap(), new.exists(), previous.exists()), ("new".to_string(), false, false));
        fs::write(new, "newer").unwrap();
        let taken = swap_in(new, file, None);
        assert_eq!(taken.map_err(|e| e.kind()), Err(io::ErrorKind::AlreadyExists));
        assert_eq!((fs::read_to_string(file).unwrap(), fs::read_to_string(new).unwrap()), ("new".to_string(), "newer".to_string()));
        tidy(made);
    }
}
