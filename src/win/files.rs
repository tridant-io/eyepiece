//! what saving a copy asks of the system: a name taken only when it is free, and the recycle bin, which must never
//! turn into a delete.

use std::ffi::c_void;
use std::path::Path;
use windows::core::{Interface, GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, PCWSTR};
use windows::Win32::Foundation::{E_FAIL, E_NOINTERFACE, S_OK};
use windows::Win32::UI::Shell::{IFileOperationProgressSink, IFileOperationProgressSink_Vtbl, TSF_DELETE_RECYCLE_IF_POSSIBLE};

/// gives `from` the name `to`, only when no file has it: one that does is never written over (AlreadyExists).
pub fn rename_new(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};
    // without MOVEFILE_REPLACE_EXISTING the system refuses a name that is taken, in the same step as the move
    unsafe { MoveFileExW(&HSTRING::from(from.as_os_str()), &HSTRING::from(to.as_os_str()), MOVE_FILE_FLAGS(0)) }.map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xffff))
}

/// why `file` cannot go to its drive's recycle bin, known before anything is asked of the system: the bin is off, the
/// file is larger than it holds, or the system keeps no setting for the drive, so neither can be known. None when it
/// can go.
pub fn refusal(file: &Path) -> Option<String> {
    use windows::Win32::Storage::FileSystem::GetVolumePathNameW;
    let size = match std::fs::metadata(file) {
        Ok(m) => m.len(),
        Err(e) => return Some(e.to_string()),
    };
    let mut root = [0u16; 261];
    let bin = unsafe { GetVolumePathNameW(&HSTRING::from(file.as_os_str()), &mut root) }.ok().and_then(|_| bin_of(&root));
    refused(size, bin)
}

/// why a file of `size` bytes cannot go to a bin set as `bin_of` reads it.
fn refused(size: u64, bin: Option<(bool, u64)>) -> Option<String> {
    match bin {
        None => Some("its drive's recycle bin cannot be confirmed to take it".into()),
        Some((false, _)) => Some("its drive's recycle bin is turned off".into()),
        Some((true, holds)) if size > holds => Some(format!("it is larger than its drive's recycle bin holds, {} GB", holds >> 30)),
        Some(_) => None,
    }
}

/// moves `file` to the recycle bin, where it can be put back from. it is never deleted in its place: what the bin will
/// not take is refused before the system is asked (`refusal`), and the shell's own delete, asked to recycle, is
/// stopped at any item it would delete outright instead (`OnlyRecycle`). Err leaves the file where it was.
pub fn trash(file: &Path) -> Result<(), String> {
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName, FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO, FOF_NO_UI};
    if let Some(why) = refusal(file) {
        return Err(why);
    }
    let asked = || -> windows::core::Result<bool> {
        let operation: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }?;
        unsafe { operation.SetOperationFlags(FOF_NO_UI | FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE) }?;
        let item: IShellItem = unsafe { SHCreateItemFromParsingName(&HSTRING::from(file.as_os_str()), None) }?;
        let guard = OnlyRecycle { table: &ONLY_RECYCLE };
        // the shell holds it no longer than the operation, which goes before it does: its counts are not kept
        let sink = unsafe { IFileOperationProgressSink::from_raw(&guard as *const OnlyRecycle as *mut c_void) };
        unsafe { operation.DeleteItem(&item, &sink) }?;
        let done = unsafe { operation.PerformOperations() };
        let aborted = unsafe { operation.GetAnyOperationsAborted() }.map(|b| b.as_bool()).unwrap_or(true);
        drop((operation, item, sink));
        done.map(|()| !aborted)
    };
    let moved = unsafe {
        // the shell wants com on the thread that asks it
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let moved = asked();
        if com {
            CoUninitialize();
        }
        moved
    };
    match moved {
        Ok(true) if std::fs::symlink_metadata(file).is_err() => Ok(()),
        Ok(_) => Err("the recycle bin did not take it".into()),
        Err(e) => Err(format!("the recycle bin did not take it: {}", e.message())),
    }
}

/// how the recycle bin of the volume mounted at `root` is set for this user: whether it takes files, and how many bytes
/// it holds. None where the system keeps no setting for it.
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
        let read = unsafe { RegGetValueW(HKEY_CURRENT_USER, &key, &HSTRING::from(name), RRF_RT_REG_DWORD, None, Some(&mut value as *mut u32 as *mut c_void), Some(&mut size)) };
        (read == ERROR_SUCCESS).then_some(value)
    };
    // its size is kept in megabytes
    Some((value("NukeOnDelete")? == 0, value("MaxCapacity")? as u64 * (1 << 20)))
}

/// what the shell's delete reports to as it goes, made by hand as com lays it out (a pointer to a table of functions):
/// before each item it says whether the item can be recycled (TSF_DELETE_RECYCLE_IF_POSSIBLE), and an item that cannot
/// is refused, which stops the whole operation, as chromium's move to the trash does. everything else it is told is
/// let pass. it is kept for the length of one `trash`, so its counts of who holds it are not kept.
#[repr(C)]
struct OnlyRecycle {
    table: *const IFileOperationProgressSink_Vtbl,
}

static ONLY_RECYCLE: IFileOperationProgressSink_Vtbl = IFileOperationProgressSink_Vtbl {
    base__: IUnknown_Vtbl { QueryInterface: query, AddRef: held, Release: held },
    StartOperations: pass,
    FinishOperations: finished,
    PreRenameItem: named,
    PostRenameItem: renamed,
    PreMoveItem: moving,
    PostMoveItem: moved,
    PreCopyItem: moving,
    PostCopyItem: moved,
    PreDeleteItem: deleting,
    PostDeleteItem: deleted,
    PreNewItem: named,
    PostNewItem: made,
    UpdateProgress: progress,
    ResetTimer: pass,
    PauseTimer: pass,
    ResumeTimer: pass,
};

/// an item the shell would delete outright, not recycle, is refused
unsafe extern "system" fn deleting(_: *mut c_void, flags: u32, _: *mut c_void) -> HRESULT {
    if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 != 0 {
        S_OK
    } else {
        E_FAIL
    }
}

unsafe extern "system" fn query(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if *iid == IUnknown::IID || *iid == IFileOperationProgressSink::IID {
        *out = this;
        S_OK
    } else {
        *out = std::ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn held(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn pass(_: *mut c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn finished(_: *mut c_void, _: HRESULT) -> HRESULT {
    S_OK
}

unsafe extern "system" fn named(_: *mut c_void, _: u32, _: *mut c_void, _: PCWSTR) -> HRESULT {
    S_OK
}

unsafe extern "system" fn renamed(_: *mut c_void, _: u32, _: *mut c_void, _: PCWSTR, _: HRESULT, _: *mut c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn moving(_: *mut c_void, _: u32, _: *mut c_void, _: *mut c_void, _: PCWSTR) -> HRESULT {
    S_OK
}

unsafe extern "system" fn moved(_: *mut c_void, _: u32, _: *mut c_void, _: *mut c_void, _: PCWSTR, _: HRESULT, _: *mut c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn deleted(_: *mut c_void, _: u32, _: *mut c_void, _: HRESULT, _: *mut c_void) -> HRESULT {
    S_OK
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn made(_: *mut c_void, _: u32, _: *mut c_void, _: PCWSTR, _: PCWSTR, _: u32, _: HRESULT, _: *mut c_void) -> HRESULT {
    S_OK
}

unsafe extern "system" fn progress(_: *mut c_void, _: u32, _: u32) -> HRESULT {
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the setting the system keeps for the system drive's recycle bin, where it keeps one: a size in megabytes
    #[test]
    fn the_recycle_bin_of_a_drive_is_read() {
        use std::os::windows::ffi::OsStrExt;
        let root: Vec<u16> = std::ffi::OsStr::new(r"C:\").encode_wide().chain([0]).collect();
        assert!(bin_of(&root).is_none_or(|(_, holds)| holds >= 1 << 20 && holds % (1 << 20) == 0));
    }

    /// what the bin will not take is refused before the system is asked: a drive with no setting (none can say the
    /// bin takes it), a bin that is off, a file larger than the bin holds. what fits goes
    #[test]
    fn what_the_bin_will_not_take_is_refused_before_it_is_asked() {
        let gb = 1u64 << 30;
        assert_eq!(refused(1, None).as_deref(), Some("its drive's recycle bin cannot be confirmed to take it"));
        assert_eq!(refused(1, Some((false, 100 * gb))).as_deref(), Some("its drive's recycle bin is turned off"));
        assert_eq!(refused(51 * gb, Some((true, 50 * gb))).as_deref(), Some("it is larger than its drive's recycle bin holds, 50 GB"));
        assert_eq!(refused(50 * gb, Some((true, 50 * gb))), None);
        // a file that is not there is not taken either, and trash says why, with nothing asked of the shell
        let gone = std::env::temp_dir().join(format!("eyepiece-no-such-clip-{}.mov", std::process::id()));
        assert!(refusal(&gone).is_some() && trash(&gone).is_err());
    }

    /// the shell's delete is refused at an item it would delete outright rather than recycle, and let through one it
    /// recycles; the guard answers to what it is and nothing else
    #[test]
    fn an_item_the_shell_would_delete_outright_is_refused() {
        let flags = TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32;
        unsafe {
            assert_eq!((deleting(std::ptr::null_mut(), 0, std::ptr::null_mut()), deleting(std::ptr::null_mut(), flags, std::ptr::null_mut())), (E_FAIL, S_OK));
            let guard = OnlyRecycle { table: &ONLY_RECYCLE };
            let sink = IFileOperationProgressSink::from_raw(&guard as *const OnlyRecycle as *mut c_void);
            assert!(sink.cast::<IUnknown>().is_ok() && sink.cast::<windows::Win32::UI::Shell::IFileOperation>().is_err());
            let none = None::<&windows::Win32::UI::Shell::IShellItem>;
            assert!(sink.PreDeleteItem(0, none).is_err() && sink.PreDeleteItem(flags, none).is_ok());
        }
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
