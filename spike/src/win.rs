//! a bare borderless window and the clocks the gates read. windows only.

use windows::core::{w, Result};
use windows::Win32::Foundation::{FILETIME, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;

fn ticks(f: FILETIME) -> u64 {
    (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64
}

/// milliseconds since the os created this process. this is the honest zero for a cold start: it includes loading our dlls.
pub fn ms_since_process_start() -> f64 {
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).expect("process times");
        (ticks(GetSystemTimePreciseAsFileTime()) - ticks(c)) as f64 / 10_000.0
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// a borderless, black, visible window of exactly w x h physical pixels at (x, y).
pub fn window(x: i32, y: i32, w: i32, h: i32) -> Result<HWND> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let inst = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: w!("eyepiece-gate"),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(WS_EX_APPWINDOW, w!("eyepiece-gate"), w!("eyepiece gate"), WS_POPUP | WS_VISIBLE, x, y, w, h, None, None, Some(inst.into()), None)
    }
}

pub fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

/// handles waiting window messages. false once the window is gone.
pub fn pump() -> bool {
    unsafe {
        let mut m = MSG::default();
        while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
            if m.message == WM_QUIT {
                return false;
            }
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
        true
    }
}

pub fn close(hwnd: HWND) {
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    pump();
}
