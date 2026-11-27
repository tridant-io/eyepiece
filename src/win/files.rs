//! what saving a copy asks of the system: a name taken only when it is free, and the recycle bin.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::{HSTRING, PCWSTR};

/// gives `from` the name `to`, only when no file has it: one that does is never written over (AlreadyExists).
pub fn rename_new(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};
    // without MOVEFILE_REPLACE_EXISTING the system refuses a name that is taken, in the same step as the move
    unsafe { MoveFileExW(&HSTRING::from(from.as_os_str()), &HSTRING::from(to.as_os_str()), MOVE_FILE_FLAGS(0)) }.map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xffff))
}

/// moves `file` to the recycle bin, where it can be put back from. a file its drive's bin will not take (the bin is
/// off, or the file is larger than it holds) is refused before the system is asked: asked, the system deletes such a
/// file in place of recycling it, or (with FOF_WANTNUKEWARNING) asks the user first. Ok only once the file has left
/// its place and the bin holds one more item than before.
pub fn trash(file: &Path) -> Result<(), String> {
    use windows::Win32::Storage::FileSystem::GetVolumePathNameW;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{SHFileOperationW, SHQueryRecycleBinW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW, SHQUERYRBINFO};
    let size = std::fs::metadata(file).map_err(|e| e.to_string())?.len();
    let mut root = [0u16; 261];
    unsafe { GetVolumePathNameW(&HSTRING::from(file.as_os_str()), &mut root) }.map_err(|e| e.message())?;
    match bin_of(&root) {
        Some((false, _)) => return Err("its drive's recycle bin is turned off".into()),
        Some((true, holds)) if size > holds => return Err(format!("it is larger than its drive's recycle bin holds, {} GB", holds >> 30)),
        _ => {}
    }
    let items = || {
        let mut bin = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
        unsafe { SHQueryRecycleBinW(PCWSTR(root.as_ptr()), &mut bin) }.ok().map(|_| bin.i64NumItems)
    };
    let before = items();
    // the shell takes a list of names, each ended by a nul, and the list by another
    let names: Vec<u16> = file.as_os_str().encode_wide().chain([0, 0]).collect();
    let flags = FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_WANTNUKEWARNING;
    let mut asked = SHFILEOPSTRUCTW { wFunc: FO_DELETE, pFrom: PCWSTR(names.as_ptr()), fFlags: flags.0 as u16, ..Default::default() };
    let failed = unsafe {
        // the shell wants com on the thread that asks it
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let failed = SHFileOperationW(&mut asked);
        if com {
            CoUninitialize();
        }
        failed
    };
    if failed != 0 || asked.fAnyOperationsAborted.as_bool() || std::fs::symlink_metadata(file).is_ok() {
        return Err(if failed != 0 { format!("the system said {failed:#x}") } else { "it was not moved".into() });
    }
    match (before, items()) {
        (Some(before), Some(after)) if after <= before => Err("it is gone, and the recycle bin holds nothing more: the system deleted it".into()),
        _ => Ok(()),
    }
}

/// how the recycle bin of the volume mounted at `root` is set for this user: whether it takes files, and how many bytes
/// it holds. None where the system keeps no setting for it and its defaults stand.
fn bin_of(root: &[u16]) -> Option<(bool, u64)> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut volume = [0u16; 64];
    unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(root.as_ptr()), &mut volume) }.ok()?;
    let volume = String::from_utf16_lossy(&volume[..volume.iter().position(|&c| c == 0)?]);
    let guid = &volume[volume.find('{')?..=volume.find('}')?];
    let key = HSTRING::from(format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume\{guid}"));
    let value = |name: &str| {
        let (mut value, mut size) = (0u32, 4u32);
        let read = unsafe { RegGetValueW(HKEY_CURRENT_USER, &key, &HSTRING::from(name), RRF_RT_REG_DWORD, None, Some(&mut value as *mut u32 as *mut std::ffi::c_void), Some(&mut size)) };
        (read == ERROR_SUCCESS).then_some(value)
    };
    // its size is kept in megabytes
    Some((value("NukeOnDelete")? == 0, value("MaxCapacity")? as u64 * (1 << 20)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the setting the system keeps for the system drive's recycle bin, where it keeps one: a size in megabytes
    #[test]
    fn the_recycle_bin_of_a_drive_is_read() {
        let root: Vec<u16> = std::ffi::OsStr::new(r"C:\").encode_wide().chain([0]).collect();
        assert!(bin_of(&root).is_none_or(|(_, holds)| holds >= 1 << 20 && holds % (1 << 20) == 0));
    }

    /// a name that is taken is refused, and the file keeps its own
    #[test]
    fn a_taken_name_is_refused() {
        let dir = std::env::temp_dir().join(format!("eyepiece-files-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (from, to) = (dir.join("a.part"), dir.join("a.mov"));
        std::fs::write(&from, b"new").unwrap();
        std::fs::write(&to, b"someone's").unwrap();
        assert_eq!(rename_new(&from, &to).map_err(|e| e.kind()), Err(std::io::ErrorKind::AlreadyExists));
        assert_eq!((std::fs::read(&from).unwrap(), std::fs::read(&to).unwrap()), (b"new".to_vec(), b"someone's".to_vec()));
        std::fs::remove_file(&to).unwrap();
        rename_new(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"new");
        std::fs::remove_file(&to).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
