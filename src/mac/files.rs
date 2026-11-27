//! what saving a copy asks of the system: a name taken only when it is free, and the trash.

use std::ffi::{c_char, c_int, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

extern "C" {
    fn renamex_np(from: *const c_char, to: *const c_char, flags: u32) -> c_int;
}
const RENAME_EXCL: u32 = 0x4;
const ENOTSUP: i32 = 45;

/// gives `from` the name `to`, only when no file has it: one that does is never written over (AlreadyExists).
pub fn rename_new(from: &Path, to: &Path) -> std::io::Result<()> {
    let (a, b) = (CString::new(from.as_os_str().as_bytes())?, CString::new(to.as_os_str().as_bytes())?);
    if unsafe { renamex_np(a.as_ptr(), b.as_ptr(), RENAME_EXCL) } == 0 {
        return Ok(());
    }
    let refused = std::io::Error::last_os_error();
    // a volume that cannot refuse a taken name in the same step (some shares, exfat): looked at first, then moved
    if refused.raw_os_error() != Some(ENOTSUP) {
        return Err(refused);
    }
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    std::fs::rename(from, to)
}

/// moves `file` to the trash, where it can be put back from. Err, with the file where it was, when it cannot go there.
pub fn trash(file: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
    unsafe { NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, None) }.map_err(|e| e.localizedDescription().to_string())
}
