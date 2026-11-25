//! one player per user: a second launch hands its file to the player already open, whose window comes to the
//! front, and exits having made nothing. the players of a session find each other by one name: a mutex in the
//! session's own namespace says whether one runs, and the window class says where it is.

use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::time::{Duration, Instant};
use windows::core::HSTRING;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_ERRORONEXIT, WM_COPYDATA};

/// marks our own WM_COPYDATA, so another program's is ignored ("eyep" in ascii)
const TAG: usize = 0x6579_6570;
/// the longest path windows takes, in utf-16 units
const LONGEST: usize = 32_767;
/// how long a second launch gives the first player to have its window up and take the file
const PATIENCE: Duration = Duration::from_secs(2);

/// the name players find each other by. `EYEPIECE_INSTANCE` makes a group of its own: the tests set it, so their
/// players meet only each other.
pub fn name() -> String {
    match std::env::var("EYEPIECE_INSTANCE") {
        Ok(group) => format!("eyepiece-{group}"),
        Err(_) => "eyepiece".to_string(),
    }
}

/// an update's restart: waits, up to ten seconds, for the build it replaced to close, which holds the one-window name
/// until it has.
pub fn wait_for(pid: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        if let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(process, 10_000);
            let _ = CloseHandle(process);
        }
    }
}

/// true when a player was already running and took `file` (with none, it only came to the front), so this process
/// has nothing left to do. false: start normally, as the first player or because the first did not answer.
pub fn handed_over(file: Option<&str>) -> bool {
    let name = name();
    // a mutex, not a look for the window: making one is atomic, so of two players started at once exactly one finds
    // it new. the handle is never closed; it goes with the process
    let mutex = HSTRING::from(format!("Local\\{name}"));
    let made = unsafe { CreateMutexW(None, false, &mutex) };
    // an error is most likely a mutex this process may not open: an elevated player's, whose window would not take
    // our message either
    if made.is_err() || unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
        return false;
    }
    // resolved here: the first player runs in another folder. not canonicalized, which would open the file
    let path: Vec<u16> = match file.map(std::path::absolute) {
        None => Vec::new(),
        Some(Ok(path)) => path.as_os_str().encode_wide().collect(),
        Some(Err(_)) => return false,
    };
    let data = COPYDATASTRUCT { dwData: TAG, cbData: (path.len() * 2) as u32, lpData: path.as_ptr() as *mut c_void };
    let class = HSTRING::from(name);
    let deadline = Instant::now() + PATIENCE;
    // the first player may still be making its window, or be closing it
    loop {
        if let Ok(window) = unsafe { FindWindowW(&class, None) } {
            let (mut pid, mut answer) = (0, 0);
            unsafe {
                GetWindowThreadProcessId(window, Some(&mut pid));
                // windows lets only the process the user just started give the foreground away, and the first player
                // must have it before it asks
                let _ = AllowSetForegroundWindow(pid);
                let left = deadline.saturating_duration_since(Instant::now()).as_millis().max(1) as u32;
                let sent = SendMessageTimeoutW(window, WM_COPYDATA, WPARAM(0), LPARAM(&data as *const COPYDATASTRUCT as isize), SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT, left, Some(&mut answer));
                if sent.0 != 0 && answer == 1 {
                    return true;
                }
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// what a second launch handed over: a full path, or an empty one when it only wants the window in front. none when
/// the message is not ours or not a path. any program can send one, so nothing in it is trusted.
///
/// safety: `data` is a WM_COPYDATA's, while that message is being handled.
pub unsafe fn received(data: &COPYDATASTRUCT) -> Option<String> {
    let len = data.cbData as usize;
    if data.dwData != TAG || !len.is_multiple_of(2) || len > LONGEST * 2 || (len > 0 && data.lpData.is_null()) {
        return None;
    }
    // read as bytes: the system's copy need not be aligned for utf-16
    let bytes = if len == 0 { &[][..] } else { std::slice::from_raw_parts(data.lpData as *const u8, len) };
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect();
    String::from_utf16(&units).ok().filter(|path| !path.contains('\0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(tag: usize, bytes: &[u8]) -> Option<String> {
        let data = COPYDATASTRUCT { dwData: tag, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut c_void };
        unsafe { received(&data) }
    }

    fn wide(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn a_path_is_taken_as_sent() {
        assert_eq!(message(TAG, &wide("C:\\clips\\a b.mov")).as_deref(), Some("C:\\clips\\a b.mov"));
        // one byte in, so the text is not aligned for utf-16
        let unaligned: Vec<u8> = std::iter::once(0).chain(wide("C:\\é.mov")).collect();
        assert_eq!(message(TAG, &unaligned[1..]).as_deref(), Some("C:\\é.mov"));
        assert_eq!(message(TAG, &wide(&"a".repeat(LONGEST))).map(|p| p.len()), Some(LONGEST));
    }

    #[test]
    fn nothing_sent_only_brings_the_window_forward() {
        assert_eq!(message(TAG, &[]).as_deref(), Some(""));
    }

    #[test]
    fn anything_else_is_ignored() {
        assert_eq!(message(TAG + 1, &wide("C:\\a.mov")), None, "another program's message");
        assert_eq!(message(TAG, &wide("C:\\a.mov")[..5]), None, "half a character");
        assert_eq!(message(TAG, &[0x00, 0xd8]), None, "a lone surrogate");
        assert_eq!(message(TAG, &wide("C:\\a\0.mov")), None, "a nul inside");
        assert_eq!(message(TAG, &wide(&"a".repeat(LONGEST + 1))), None, "longer than any path");
        let missing = COPYDATASTRUCT { dwData: TAG, cbData: 8, lpData: std::ptr::null_mut() };
        assert_eq!(unsafe { received(&missing) }, None, "a length with no data");
    }
}
