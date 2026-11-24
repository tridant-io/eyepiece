//! the window: plain win32, with no title bar of its own. the strip our renderer draws along the top is the title
//! bar, and the picture reaches the window's top edge. the system's own frame is kept on the other three sides,
//! where windows 10 and 11 draw it invisible: so resizing, snapping, the shadow and win + arrows all still work.
//! it owns the message loop on the main thread and turns messages into events for the render thread, which never
//! waits on it. dragging or resizing the window therefore never stalls the picture.

use crate::event::{Button, Event, Key};
use std::ffi::c_void;
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWM_TIMING_INFO};
use windows::Win32::Graphics::Gdi::{GetStockObject, ScreenToClient, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_END, VK_HOME, VK_LEFT, VK_RIGHT, VK_SHIFT, VK_SPACE};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;

const STYLE: WINDOW_STYLE = WS_OVERLAPPEDWINDOW;
/// one frame per 120 wheel units, so a free-spinning wheel stays exact
const WHEEL_NOTCH: i32 = 120;
/// the pointer left the window (asked for with TrackMouseEvent)
const WM_MOUSELEAVE: u32 = 0x02A3;
/// the picture area is never smaller than this, in the design's pixels
const MIN_CLIENT: (i32, i32) = (640, 360);
/// how far in from the top edge a drag resizes the window, in the design's pixels. the other edges have the
/// system's own invisible border outside the picture
const TOP_GRIP: i32 = 5;

/// our own messages, posted by the render thread: a new title is waiting in `TITLE`; move the window with the
/// pointer; minimize; maximize or restore
const WM_TITLE: u32 = WM_APP;
const WM_BEGIN_MOVE: u32 = WM_APP + 1;
const WM_MINIMIZE: u32 = WM_APP + 2;
const WM_TOGGLE_MAXIMIZE: u32 = WM_APP + 3;
/// the title the render thread last asked for. one window a process, for now.
static TITLE: Mutex<Option<String>> = Mutex::new(None);

/// what the window procedure needs between messages
struct Shared {
    events: Sender<Event>,
    wheel: i32,
    /// windows has been asked to say when the pointer leaves
    tracking: bool,
}

/// a window handle that may cross to the render thread: win32 lets any thread size, title and present to a window.
#[derive(Clone, Copy)]
pub struct Handle(pub HWND);
unsafe impl Send for Handle {}

fn work_area() -> RECT {
    let mut area = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut area as *mut RECT as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    area
}

/// the size of the picture area for a clip: its own size if that fits in 90% of the screen's work area, otherwise
/// fitted to 90%, and never under 640 x 360.
pub fn client_for(video: (usize, usize)) -> (u32, u32) {
    let area = work_area();
    let (max_w, max_h) = ((area.right - area.left) as f64 * 0.9, (area.bottom - area.top) as f64 * 0.9);
    let (vw, vh) = (video.0.max(1) as f64, video.1.max(1) as f64);
    let scale = (max_w / vw).min(max_h / vh).min(1.0);
    (((vw * scale).round() as u32).max(MIN_CLIENT.0 as u32), ((vh * scale).round() as u32).max(MIN_CLIENT.1 as u32))
}

/// how much wider and taller than its picture area the window is: the system's frame on the left, right and bottom.
/// there is none on top.
fn frame() -> (i32, i32, i32) {
    let mut r = RECT::default();
    unsafe {
        let _ = AdjustWindowRectEx(&mut r, STYLE, false, WINDOW_EX_STYLE(0));
    }
    (-r.left, r.right, r.bottom)
}

/// the window's position and size that give `client` pixels of picture, centered on the work area.
fn outer(client: (u32, u32)) -> (i32, i32, i32, i32) {
    let (left, right, bottom) = frame();
    let area = work_area();
    let (w, h) = (client.0 as i32 + left + right, client.1 as i32 + bottom);
    (area.left + ((area.right - area.left) - w) / 2, (area.top + ((area.bottom - area.top) - h) / 2).max(area.top), w, h)
}

/// the window, black, on screen at once. `client` is the picture area in physical pixels.
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
        // a seam for the tests, which drive the pointer with posted messages: windows is then never asked to say
        // when the real pointer leaves, because it would say so at once
        let posted_pointer = std::env::var_os("EYEPIECE_POSTED_POINTER").is_some();
        let (x, y, w, h) = outer(client);
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("eyepiece"), w!("eyepiece"), STYLE, x, y, w, h, None, None, Some(inst.into()), None)?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(Box::new(Shared { events, wheel: 0, tracking: posted_pointer })) as isize);
        // square corners and no border line, so no pixel of the picture is covered (windows 11; ignored before it)
        let (square, no_color) = (1u32, 0xffff_fffeu32);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const u32 as *const c_void, 4);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &no_color as *const u32 as *const c_void, 4);
        // the frame was measured before this procedure was listening: have it measured again, our way
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_FRAMECHANGED | SWP_NOZORDER | SWP_NOACTIVATE);
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

/// physical pixels for one of the design's pixels on the display the window is on: 1 at 100%, 1.5 at 150%.
pub fn scale(window: Handle) -> f32 {
    (unsafe { GetDpiForWindow(window.0) } as f32 / 96.0).max(0.5)
}

pub fn client_size(window: Handle) -> (u32, u32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(window.0, &mut r);
    }
    ((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32)
}

fn post(window: Handle, message: u32) {
    unsafe {
        let _ = PostMessageW(Some(window.0), message, WPARAM(0), LPARAM(0));
    }
}

/// asks the window to show a title, without waiting for it: the window's thread takes tens of milliseconds over
/// it, which is a missed refresh or two if the render thread waits. the taskbar and alt-tab show the title.
pub fn set_title(window: Handle, title: &str) {
    *TITLE.lock().unwrap() = Some(title.to_string());
    post(window, WM_TITLE);
}

/// hands the window to the system to move with the pointer, whose button is down: snapping to the screen's edges
/// included. the picture keeps playing meanwhile.
pub fn begin_move(window: Handle) {
    post(window, WM_BEGIN_MOVE);
}

pub fn minimize(window: Handle) {
    post(window, WM_MINIMIZE);
}

/// maximizes the window, or puts it back.
pub fn toggle_maximize(window: Handle) {
    post(window, WM_TOGGLE_MAXIMIZE);
}

pub fn close(window: Handle) {
    post(window, WM_CLOSE);
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

/// the two coordinates packed into a message's last parameter
fn xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

/// the frame, our way: the system's on the left, right and bottom, none on top.
unsafe fn frame_without_top(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let params = lp.0 as *mut NCCALCSIZE_PARAMS;
    let top = (*params).rgrc[0].top;
    DefWindowProcW(hwnd, msg, wp, lp);
    // a maximized window hangs over the screen's edges by its frame, at the top too: keep the picture on screen
    let dpi = GetDpiForWindow(hwnd);
    let overhang = if IsZoomed(hwnd).as_bool() { GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi) } else { 0 };
    (*params).rgrc[0].top = top + overhang;
    LRESULT(0)
}

/// which part of the window a point is on. the system knows the left, right and bottom borders; the top edge is
/// ours to give a grip, since the picture starts at it.
unsafe fn hit(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let found = DefWindowProcW(hwnd, msg, wp, lp);
    if found.0 != HTCLIENT as isize || IsZoomed(hwnd).as_bool() {
        return found;
    }
    let (x, y) = xy(lp);
    let mut window = RECT::default();
    let _ = GetWindowRect(hwnd, &mut window);
    let grip = TOP_GRIP * GetDpiForWindow(hwnd) as i32 / 96;
    if y - window.top >= grip {
        return found;
    }
    let corner = grip * 3;
    LRESULT(if x - window.left < corner {
        HTTOPLEFT
    } else if window.right - x < corner {
        HTTOPRIGHT
    } else {
        HTTOP
    } as isize)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // the shape of the window does not wait for the rest of the setup
    match msg {
        WM_NCCALCSIZE if wp.0 != 0 => return frame_without_top(hwnd, msg, wp, lp),
        WM_NCHITTEST => return hit(hwnd, msg, wp, lp),
        WM_GETMINMAXINFO => {
            let (left, right, bottom) = frame();
            let scale = GetDpiForWindow(hwnd).max(96) as i32;
            let info = lp.0 as *mut MINMAXINFO;
            (*info).ptMinTrackSize = POINT { x: MIN_CLIENT.0 * scale / 96 + left + right, y: MIN_CLIENT.1 * scale / 96 + bottom };
            return LRESULT(0);
        }
        _ => {}
    }
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
        WM_MOUSEMOVE => {
            if !(*shared).tracking {
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                (*shared).tracking = TrackMouseEvent(&mut track).is_ok();
            }
            let (x, y) = xy(lp);
            let _ = (*shared).events.send(Event::PointerMove { x: x as f32, y: y as f32 });
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            (*shared).tracking = false;
            let _ = (*shared).events.send(Event::PointerLeave);
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP => {
            let (button, down) = match msg {
                WM_LBUTTONDOWN => (Button::Left, true),
                WM_LBUTTONUP => (Button::Left, false),
                WM_RBUTTONDOWN => (Button::Right, true),
                WM_RBUTTONUP => (Button::Right, false),
                WM_MBUTTONDOWN => (Button::Middle, true),
                _ => (Button::Middle, false),
            };
            // a drag that leaves the window keeps reporting to it until the button comes up
            if button == Button::Left {
                if down {
                    SetCapture(hwnd);
                } else {
                    let _ = ReleaseCapture();
                }
            }
            let (x, y) = xy(lp);
            send(Event::PointerButton { button, down, x: x as f32, y: y as f32 });
            LRESULT(0)
        }
        WM_BEGIN_MOVE => {
            let _ = ReleaseCapture();
            // the system's own move, as if its title bar had been grabbed. it returns when the button comes up
            SendMessageW(hwnd, WM_SYSCOMMAND, Some(WPARAM((SC_MOVE | HTCAPTION) as usize)), Some(LPARAM(0)));
            // the move took the button's release with it: report it here, where the pointer is now
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            let _ = ScreenToClient(hwnd, &mut p);
            send(Event::PointerButton { button: Button::Left, down: false, x: p.x as f32, y: p.y as f32 });
            LRESULT(0)
        }
        WM_MINIMIZE => {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
            LRESULT(0)
        }
        WM_TOGGLE_MAXIMIZE => {
            let _ = ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE });
            LRESULT(0)
        }
        WM_DPICHANGED => {
            send(Event::Scale(((wp.0 >> 16) & 0xffff) as f32 / 96.0));
            // windows suggests where the window should sit at the new scale
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
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
