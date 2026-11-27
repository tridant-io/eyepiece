//! putting a file written whole in the place of another, for the notes (`crate::notes`): there is no moment with
//! half a file under the name, or none.

use std::ffi::{c_char, CString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

extern "C" {
    fn renamex_np(from: *const c_char, to: *const c_char, flags: u32) -> i32;
}

/// `renamex_np` fails when the name is taken (sys/stdio.h)
const RENAME_EXCL: u32 = 0x4;
/// the volume has no such rename (errno.h)
const ENOTSUP: i32 = 45;

/// puts `new` in `file`'s place. with `previous`, `file` is there and has been read: it is copied aside under that
/// name, and the rename over it is one step. with none, `file` is not there: if it has come meanwhile nothing is
/// replaced, and this fails with AlreadyExists.
pub fn swap_in(new: &Path, file: &Path, previous: Option<&Path>) -> io::Result<()> {
    match previous {
        Some(previous) => {
            std::fs::copy(file, previous)?;
            std::fs::rename(new, file)?;
        }
        None => {
            let c = |path: &Path| CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other);
            let (from, to) = (c(new)?, c(file)?);
            if unsafe { renamex_np(from.as_ptr(), to.as_ptr(), RENAME_EXCL) } != 0 {
                let why = io::Error::last_os_error();
                // a volume that cannot rename so (some shares): a look, then the rename
                if why.raw_os_error() != Some(ENOTSUP) {
                    return Err(why);
                }
                if file.exists() {
                    return Err(io::ErrorKind::AlreadyExists.into());
                }
                std::fs::rename(new, file)?;
            }
        }
    }
    // the folder's record of the rename, on the disk too, where the volume does that
    if let Some(folder) = file.parent() {
        let _ = std::fs::File::open(folder).and_then(|folder| folder.sync_all());
    }
    Ok(())
}
