//! the window: plain win32. it owns the message loop on the main thread and turns messages into events for the
//! render thread, which never waits on it. dragging or resizing the window therefore never stalls the picture.

use crate::event::{Event, Key};
use std::ffi::c_void;
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_END, VK_HOME, VK_LEFT, VK_RIGHT, VK_SHIFT, VK_SPACE};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

const STYLE: WINDOW_STYLE = WS_OVERLAPPEDWINDOW;
/// one frame per 120 wheel units, so a free-spinning wheel stays exact
const WHEEL_NOTCH: i32 = 120;

/// our own message: a new title is waiting in `TITLE`
const WM_TITLE: u32 = WM_APP;
/// the title the render thread last asked for. one window a process, for now.
static TITLE: Mutex<Option<String>> = Mutex::new(None);

/// what the window procedure needs between messages
struct Shared {
    events: Sender<Event>,
    wheel: i32,
}

/// a window handle that may cross to the render thread: win32 lets any thread size, title and present to a window.
#[derive(Clone, Copy)]
pub struct Handle(pub HWND);
unsafe impl Send for Handle {}

/// the size of the picture area for a clip: its own size if that fits in 90% of the screen's work area, otherwise
/// fitted to 90%, and never under 640 x 360.
pub fn client_for(video: (usize, usize)) -> (u32, u32) {
    let mut area = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut area as *mut RECT as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    let (max_w, max_h) = ((area.right - area.left) as f64 * 0.9, (area.bottom - area.top) as f64 * 0.9);
    let (vw, vh) = (video.0.max(1) as f64, video.1.max(1) as f64);
    let scale = (max_w / vw).min(max_h / vh).min(1.0);
    (((vw * scale).round() as u32).max(640), ((vh * scale).round() as u32).max(360))
}

/// the outer size and position that give `client` pixels of picture, centered on the work area.
fn outer(client: (u32, u32)) -> (i32, i32, i32, i32) {
    let mut r = RECT { left: 0, top: 0, right: client.0 as i32, bottom: client.1 as i32 };
    let mut area = RECT::default();
    unsafe {
        let _ = AdjustWindowRectEx(&mut r, STYLE, false, WINDOW_EX_STYLE(0));
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut area as *mut RECT as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    (area.left + ((area.right - area.left) - w) / 2, (area.top + ((area.bottom - area.top) - h) / 2).max(area.top), w, h)
}

/// an ordinary window, black, on screen at once. `client` is the picture area in physical pixels.
pub fn create(client: (u32, u32), events: Sender<Event>) -> Result<Handle> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let inst = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: w!("eyepiece"),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        let (x, y, w, h) = outer(client);
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("eyepiece"), w!("eyepiece"), STYLE, x, y, w, h, None, None, Some(inst.into()), None)?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(Box::new(Shared { events, wheel: 0 })) as isize);
        DragAcceptFiles(hwnd, true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok(Handle(hwnd))
    }
}

/// runs the window's messages until it closes. call on the thread that made the window.
pub fn run() {
    unsafe {
        let mut m = MSG::default();
        while GetMessageW(&mut m, None, 0, 0).as_bool() {
            DispatchMessageW(&m);
        }
    }
}

/// resizes the window so its picture area is `client`, centered. for a newly opened clip.
pub fn fit(window: Handle, client: (u32, u32)) {
    let (x, y, w, h) = outer(client);
    unsafe {
        let _ = SetWindowPos(window.0, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

pub fn client_size(window: Handle) -> (u32, u32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(window.0, &mut r);
    }
    ((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32)
}

/// asks the window to show a title, without waiting for it: redrawing a title bar takes the window's thread tens of
/// milliseconds, which is a missed refresh or two if the render thread waits.
pub fn set_title(window: Handle, title: &str) {
    *TITLE.lock().unwrap() = Some(title.to_string());
    unsafe {
        let _ = PostMessageW(Some(window.0), WM_TITLE, WPARAM(0), LPARAM(0));
    }
}

/// the rate the desktop is composed at, in Hz, as the system reports it. 60 if it will not say.
pub fn refresh_rate() -> f64 {
    let mut info = DWM_TIMING_INFO { cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    let rate = unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }.ok().map(|_| info.rateRefresh);
    match rate {
        Some(r) if r.uiDenominator > 0 && r.uiNumerator > 0 => r.uiNumerator as f64 / r.uiDenominator as f64,
        _ => 60.0,
    }
}

fn key(vk: u16) -> Option<Key> {
    Some(match vk {
        v if v == VK_SPACE.0 => Key::Space,
        v if v == VK_LEFT.0 => Key::Left,
        v if v == VK_RIGHT.0 => Key::Right,
        v if v == VK_HOME.0 => Key::Home,
        v if v == VK_END.0 => Key::End,
        0x46 => Key::F,
        0x47 => Key::G,
        _ => return None,
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let shared = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Shared;
    if shared.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    // the render thread going away first is not an error here: the window is closing too
    let send = |e: Event| {
        let _ = (*shared).events.send(e);
    };
    match msg {
        WM_KEYDOWN => {
            if let Some(key) = key(wp.0 as u16) {
                send(Event::Key { key, shift: GetKeyState(VK_SHIFT.0 as i32) < 0 });
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let total = (*shared).wheel + ((wp.0 >> 16) & 0xffff) as i16 as i32;
            let notches = total / WHEEL_NOTCH;
            (*shared).wheel = total - notches * WHEEL_NOTCH;
            if notches != 0 {
                let _ = (*shared).events.send(Event::Wheel { notches });
            }
            LRESULT(0)
        }
        WM_SIZE => {
            let (w, h) = ((lp.0 & 0xffff) as u32, ((lp.0 >> 16) & 0xffff) as u32);
            // a minimized window reports 0 x 0: nothing to draw into
            if w > 0 && h > 0 {
                send(Event::Resize { w, h });
            }
            LRESULT(0)
        }
        WM_DROPFILES => {
            let drop = HDROP(wp.0 as *mut c_void);
            let mut name = [0u16; 1024];
            let len = DragQueryFileW(drop, 0, Some(&mut name)) as usize;
            DragFinish(drop);
            if len > 0 {
                send(Event::Open(String::from_utf16_lossy(&name[..len])));
            }
            LRESULT(0)
        }
        WM_TITLE => {
            if let Some(title) = TITLE.lock().unwrap().take() {
                let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
            }
            LRESULT(0)
        }
        // the picture is drawn by the render thread; erasing here would flash black between frames
        WM_ERASEBKGND => LRESULT(1),
        WM_DESTROY => {
            send(Event::Close);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(shared));
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
