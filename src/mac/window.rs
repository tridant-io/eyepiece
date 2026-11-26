//! the window: appkit, with no title bar. it is borderless with square corners, so no pixel of the picture is covered,
//! and keeps the system's shadow and its edges that resize. the strip our renderer draws along the top is the title bar.
//! it owns the event loop on the main thread and turns events into ours for the render thread, which never waits on
//! it; what the render thread asks of the window is done on the main thread, a moment later.

use crate::event::{Button, Event, Key, Mods};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSApplicationPresentationOptions, NSApplicationTerminateReply, NSBackingStoreType, NSColor, NSCursor, NSDragOperation, NSDraggingInfo, NSEvent,
    NSEventMask, NSEventModifierFlags, NSEventType, NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardTypePNG, NSPasteboardTypeString, NSScreen, NSTrackingArea, NSTrackingAreaOptions, NSView, NSWindow, NSWindowDelegate,
    NSAutoresizingMaskOptions, NSButton, NSImage, NSWindowButton, NSWindowOcclusionState, NSWindowStyleMask, NSWorkspace,
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSArray, NSCalendar, NSCalendarUnit, NSData, NSDate, NSDefaultRunLoopMode, NSNotification, NSObject, NSObjectProtocol, NSOperationQueue, NSPoint, NSRect, NSSize, NSString, NSURL};
use objc2_metal::MTLCreateSystemDefaultDevice;
use objc2_quartz_core::CAMetalLayer;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;

/// the picture area is never smaller than this, in the design's pixels (appkit's points)
const MIN_CLIENT: (f64, f64) = (640.0, 360.0);
/// how far two fingers on the trackpad move for one notch of a wheel, in points
const NOTCH: f64 = 12.0;

/// where the window's events go. one window a process.
static EVENTS: Mutex<Option<Sender<Event>>> = Mutex::new(None);
/// set when the window has closed: the event loop ends
static CLOSED: AtomicBool = AtomicBool::new(false);
/// what the render thread reads without asking the main thread: the picture area in physical pixels, physical pixels
/// for one point (as the bits of an f32), and 90% of the main display's free area in physical pixels
static CLIENT: (AtomicU32, AtomicU32) = (AtomicU32::new(1), AtomicU32::new(1));
static SCALE: AtomicU32 = AtomicU32::new(0);
static ROOM: (AtomicU32, AtomicU32) = (AtomicU32::new(1920), AtomicU32::new(1080));
/// whether any of the window can be seen, as the system last said: one that cannot has no frame to wait for
static VISIBLE: AtomicBool = AtomicBool::new(true);
/// the refresh rate of the display the window is on, as the bits of an f32; 0 until it is known
static HZ: AtomicU32 = AtomicU32::new(0);
/// the system asks for less motion, as it said when the window was made
static STILL: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// the window and its view, on the main thread. what changes between events is the view's own (`ViewState`)
    static WINDOW: RefCell<Option<(Retained<NSWindow>, Retained<PictureView>)>> = const { RefCell::new(None) };
}

/// the window's metal layer, which may cross to the render thread: a layer is drawn into from any thread.
#[derive(Clone, Copy)]
pub struct Handle(pub *const CAMetalLayer);
unsafe impl Send for Handle {}

// the render thread going away first is not an error here: the window is closing too
fn send(event: Event) {
    if let Some(events) = EVENTS.lock().unwrap().as_ref() {
        let _ = events.send(event);
    }
}

/// does `work` on the main thread, a moment later, with the window and its view. nothing of ours is borrowed
/// meanwhile: appkit calls back into the view from inside its own calls.
fn on_main(work: impl FnOnce(&NSWindow, &PictureView, MainThreadMarker) + Send + 'static) {
    let work = Cell::new(Some(work));
    let block = block2::RcBlock::new(move || {
        let (Some(work), Some(mtm)) = (work.take(), MainThreadMarker::new()) else { return };
        if let Some((window, view)) = WINDOW.with_borrow(|window| window.clone()) {
            work(&window, &view, mtm);
        }
    });
    unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
}

pub struct ViewState {
    /// what is left of the trackpad's travel after the notches sent
    wheel: Cell<f64>,
    /// the press a move of the window starts from
    pressed: RefCell<Option<Retained<NSEvent>>>,
    /// while fullscreen: where the window was, to put it back
    restore: Cell<Option<NSRect>>,
    pointer: Cell<Pointer>,
    /// the system's close, minimize and zoom buttons, in a view of their own over the strip's left end
    buttons: RefCell<Option<Retained<Buttons>>>,
}

/// where the system's buttons sit in the strip, in points from the window's top left: the first one's corner, the
/// step from one to the next, and the room the strip keeps for the three
const BUTTONS_AT: (f64, f64) = (12.0, 8.0);
const BUTTONS_STEP: f64 = 20.0;
const BUTTONS_ROOM: f32 = 78.0;

define_class!(
    // SAFETY: NSView asks nothing of its subclasses, and this one is made and used on the main thread only
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "EyepieceButtons"]
    #[ivars = Cell<bool>]
    /// holds the system's window buttons, which the window, having no title bar, has no place for. true while the
    /// pointer is among them.
    struct Buttons;

    impl Buttons {
        // the buttons ask the view they sit in whether the pointer is among them, and show their marks while it is
        #[unsafe(method(_mouseInGroup:))]
        fn mouse_in_group(&self, _button: &NSButton) -> bool {
            self.ivars().get()
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.among(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.among(false);
        }

        #[unsafe(method(closePressed:))]
        fn close_pressed(&self, _button: &NSButton) {
            close(Handle(std::ptr::null()));
        }

        #[unsafe(method(minimizePressed:))]
        fn minimize_pressed(&self, _button: &NSButton) {
            if let Some(window) = self.window() {
                window.miniaturize(None);
            }
        }

        // in fullscreen it leaves fullscreen, which is what putting the window back means there: what P does
        #[unsafe(method(zoomPressed:))]
        fn zoom_pressed(&self, _button: &NSButton) {
            let full = WINDOW.with_borrow(|window| window.as_ref().is_some_and(|(_, view)| view.ivars().restore.get().is_some()));
            if full {
                send(Event::Key { key: Key::P, mods: Mods::default() });
            } else if let Some(window) = self.window() {
                window.zoom(None);
            }
        }
    }
);

impl Buttons {
    fn among(&self, over: bool) {
        self.ivars().set(over);
        for button in self.subviews().iter() {
            button.setNeedsDisplay(true);
        }
    }

    /// the three buttons in a view just their size, hidden until the strip comes up.
    fn new(mtm: MainThreadMarker) -> Retained<Buttons> {
        let size = NSSize::new(2.0 * BUTTONS_STEP + 14.0, 16.0);
        let holder = Buttons::alloc(mtm).set_ivars(Cell::new(false));
        let holder: Retained<Buttons> = unsafe { msg_send![super(holder), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), size)] };
        let titled = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable;
        let made = [(NSWindowButton::CloseButton, sel!(closePressed:)), (NSWindowButton::MiniaturizeButton, sel!(minimizePressed:)), (NSWindowButton::ZoomButton, sel!(zoomPressed:))];
        for (i, (kind, pressed)) in made.into_iter().enumerate() {
            let Some(button) = NSWindow::standardWindowButton_forStyleMask(kind, titled, mtm) else { continue };
            button.setFrameOrigin(NSPoint::new(i as f64 * BUTTONS_STEP, 0.0));
            let target: &AnyObject = &holder;
            unsafe {
                button.setTarget(Some(target));
                button.setAction(Some(pressed));
            }
            holder.addSubview(&button);
        }
        let tracking = NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways | NSTrackingAreaOptions::InVisibleRect;
        let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(mtm.alloc(), holder.bounds(), tracking, Some(&holder), None) };
        holder.addTrackingArea(&area);
        holder.setHidden(true);
        holder
    }

    /// puts the buttons over the strip's left end: in the window's frame view, above the picture, held to its top
    /// left corner. again after the window's style changed, in case it made itself a new frame view.
    fn place(&self, window: &NSWindow) {
        let Some(frame) = window.contentView().and_then(|picture| unsafe { picture.superview() }) else { return };
        if unsafe { self.superview() }.as_deref() != Some(&*frame) {
            frame.addSubview(self);
        }
        // the frame view counts from its bottom
        self.setFrameOrigin(NSPoint::new(BUTTONS_AT.0, frame.bounds().size.height - BUTTONS_AT.1 - self.frame().size.height));
        self.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMaxXMargin | NSAutoresizingMaskOptions::ViewMinYMargin);
    }
}

define_class!(
    // SAFETY: the superclass has no subclassing requirements we break, and the class is only used on the main thread
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "EyepieceWindow"]
    struct Frame;

    // a window without a title bar takes no keys unless it says it does
    impl Frame {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            true
        }
    }
);

define_class!(
    // SAFETY: as above; it keeps nothing
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "EyepieceDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSApplicationDelegate for Delegate {
        // quit from the dock, a logout or a restart: the window closes its own way, so what is kept between runs is
        // saved, and the program then ends by itself a moment later
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            close(Handle(std::ptr::null()));
            NSApplicationTerminateReply::TerminateCancel
        }
    }

    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowDidChangeOcclusionState:))]
        fn occlusion_changed(&self, _notification: &NSNotification) {
            watched();
        }

        #[unsafe(method(windowDidChangeScreen:))]
        fn screen_changed(&self, _notification: &NSNotification) {
            watched();
        }
    }
);

/// takes whether the window can be seen and its display's refresh rate, for the render thread to read.
fn watch(window: &NSWindow) {
    VISIBLE.store(window.occlusionState().contains(NSWindowOcclusionState::Visible), Ordering::Relaxed);
    if let Some(screen) = window.screen() {
        HZ.store((screen.maximumFramesPerSecond() as f32).to_bits(), Ordering::Relaxed);
    }
}

fn watched() {
    if let Some((window, _)) = WINDOW.with_borrow(|window| window.clone()) {
        watch(&window);
    }
}

/// true while any of the window can be seen.
pub fn visible() -> bool {
    VISIBLE.load(Ordering::Relaxed)
}

define_class!(
    // SAFETY: as above
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "EyepiecePicture"]
    #[ivars = ViewState]
    pub struct PictureView;

    impl PictureView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        // the first click on a window that is not in front is a click, not only a wake
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        // from the top left, as the picture is addressed
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.ivars().pressed.replace(Some(event.retain()));
            self.button(event, Button::Left, true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            self.ivars().pressed.replace(None);
            self.button(event, Button::Left, false);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_down(&self, event: &NSEvent) {
            self.button(event, Button::Right, true);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_up(&self, event: &NSEvent) {
            self.button(event, Button::Right, false);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_down(&self, event: &NSEvent) {
            self.button(event, Button::Middle, true);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_up(&self, event: &NSEvent) {
            self.button(event, Button::Middle, false);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.moved(event);
        }

        // a drag that leaves the window keeps reporting to it until the button comes up
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.moved(event);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_dragged(&self, event: &NSEvent) {
            self.moved(event);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            send(Event::PointerLeave);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            // the coasting after the fingers lift is not the user's: a frame is stepped to, not thrown at
            if !event.momentumPhase().is_empty() {
                return;
            }
            // with shift held the system turns a mouse's wheel sideways
            let held = mods(event);
            let moved = if held.shift && event.scrollingDeltaY() == 0.0 { event.scrollingDeltaX() } else { event.scrollingDeltaY() };
            // a mouse's wheel counts in lines, one a notch; a trackpad in points, which are gathered into notches
            let total = self.ivars().wheel.get() + if event.hasPreciseScrollingDeltas() { moved / NOTCH } else { moved };
            let notches = total.trunc();
            self.ivars().wheel.set(total - notches);
            if notches != 0.0 {
                send(Event::Wheel { notches: notches as i32, mods: held });
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let held = mods(event);
            let key = key(event);
            // command + Q, as everywhere on a mac
            if key == Some(Key::Q) && event.modifierFlags().contains(NSEventModifierFlags::Command) {
                return close(Handle(std::ptr::null()));
            }
            if let Some(key) = key {
                send(Event::Key { key, mods: held });
            }
            // what the key types, for the few fields that take text: not with command or control held, and not the
            // private characters appkit gives the arrows and their like
            if !held.ctrl {
                for c in event.characters().map(|typed| typed.to_string()).unwrap_or_default().chars() {
                    if !c.is_control() && !('\u{f700}'..='\u{f8ff}').contains(&c) {
                        send(Event::Char(c));
                    }
                }
            }
        }

        // shift, option, control or command went down or came up: shift keeps a box being drawn square, at once
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            send(Event::Held(mods(event)));
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.measured();
        }

        // the window moved to a display with another scale
        #[unsafe(method(viewDidChangeBackingProperties))]
        fn backing_changed(&self) {
            let _: () = unsafe { msg_send![super(self), viewDidChangeBackingProperties] };
            let was = f32::from_bits(SCALE.load(Ordering::Relaxed));
            self.measured();
            let now = f32::from_bits(SCALE.load(Ordering::Relaxed));
            if now != was {
                send(Event::Scale(now));
            }
        }

        // which pointer shows over the picture: appkit asks whenever it might have changed
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            let cursor = match self.ivars().pointer.get() {
                Pointer::Cross => NSCursor::crosshairCursor(),
                Pointer::Resize => NSCursor::resizeLeftRightCursor(),
                _ => NSCursor::arrowCursor(),
            };
            self.addCursorRect_cursor(self.bounds(), &cursor);
        }

        // a file dragged over the window may be dropped on it
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, _sender: &objc2::runtime::ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            NSDragOperation::Copy
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag(&self, sender: &objc2::runtime::ProtocolObject<dyn NSDraggingInfo>) -> bool {
            let dropped = unsafe { sender.draggingPasteboard().stringForType(NSPasteboardTypeFileURL) };
            let path = dropped.and_then(|url| NSURL::URLWithString(&url)).and_then(|url| url.path());
            match path {
                Some(path) => {
                    send(Event::Open(path.to_string()));
                    true
                }
                None => false,
            }
        }
    }
);

impl PictureView {
    /// where an event's pointer is in the picture area, in physical pixels from its top left.
    fn place(&self, event: &NSEvent) -> (f32, f32) {
        let at = self.convertPoint_fromView(event.locationInWindow(), None);
        let scale = f32::from_bits(SCALE.load(Ordering::Relaxed)) as f64;
        ((at.x * scale) as f32, (at.y * scale) as f32)
    }

    fn button(&self, event: &NSEvent, button: Button, down: bool) {
        let (x, y) = self.place(event);
        send(Event::PointerButton { button, down, x, y, mods: mods(event) });
    }

    fn moved(&self, event: &NSEvent) {
        let (x, y) = self.place(event);
        send(Event::PointerMove { x, y });
    }

    /// takes the picture area's size and scale as they are now, and says so when the size changed.
    fn measured(&self) {
        let scale = self.window().map_or(1.0, |window| window.backingScaleFactor());
        let size = self.bounds().size;
        let (w, h) = (((size.width * scale).round() as u32).max(1), ((size.height * scale).round() as u32).max(1));
        SCALE.store((scale as f32).to_bits(), Ordering::Relaxed);
        // a layer the view is given, not one it makes, is not told the display's scale
        if let Some(layer) = self.layer() {
            layer.setContentsScale(scale);
        }
        if (CLIENT.0.swap(w, Ordering::Relaxed), CLIENT.1.swap(h, Ordering::Relaxed)) != (w, h) {
            send(Event::Resize { w, h });
        }
    }
}

/// which of shift, option and control or command are held: command does here what control does on windows.
fn mods(event: &NSEvent) -> Mods {
    let held = event.modifierFlags();
    Mods { shift: held.contains(NSEventModifierFlags::Shift), alt: held.contains(NSEventModifierFlags::Option), ctrl: held.contains(NSEventModifierFlags::Command) || held.contains(NSEventModifierFlags::Control) }
}

/// a key by its place on the keyboard, as an american layout letters it: for a layout that types no latin letters.
fn by_place(code: u16) -> Key {
    match code {
        0 => Key::A,
        8 => Key::C,
        2 => Key::D,
        14 => Key::E,
        3 => Key::F,
        5 => Key::G,
        4 => Key::H,
        34 => Key::I,
        38 => Key::J,
        40 => Key::K,
        37 => Key::L,
        46 => Key::M,
        45 => Key::N,
        31 => Key::O,
        35 => Key::P,
        12 => Key::Q,
        15 => Key::R,
        1 => Key::S,
        17 => Key::T,
        9 => Key::V,
        13 => Key::W,
        16 => Key::Y,
        6 => Key::Z,
        29 => Key::Digit0,
        18 => Key::Digit1,
        19 => Key::Digit2,
        27 => Key::Minus,
        24 => Key::Equals,
        33 => Key::BracketLeft,
        30 => Key::BracketRight,
        43 => Key::Comma,
        44 => Key::Slash,
        _ => Key::Other,
    }
}

/// a key press as one of our keys: the keys with no letter by their place on the keyboard, the rest by the letter the
/// layout gives them, whatever is held with them.
fn key(event: &NSEvent) -> Option<Key> {
    let code = event.keyCode();
    Some(match code {
        49 => Key::Space,
        36 | 76 => Key::Enter,
        53 => Key::Escape,
        51 => Key::Backspace,
        117 => Key::Delete,
        48 => Key::Tab,
        123 => Key::Left,
        124 => Key::Right,
        126 => Key::Up,
        125 => Key::Down,
        115 => Key::Home,
        119 => Key::End,
        116 => Key::PageUp,
        121 => Key::PageDown,
        96 => Key::F5,
        _ => match event.charactersIgnoringModifiers().and_then(|typed| typed.to_string().chars().next()).map(|c| c.to_ascii_lowercase()) {
            Some('0' | ')') => Key::Digit0,
            Some('1' | '!') => Key::Digit1,
            Some('2' | '@') => Key::Digit2,
            Some('-' | '_') => Key::Minus,
            Some('=' | '+') => Key::Equals,
            Some('[' | '{') => Key::BracketLeft,
            Some(']' | '}') => Key::BracketRight,
            Some(',' | '<') => Key::Comma,
            Some('/' | '?') => Key::Slash,
            Some('a') => Key::A,
            Some('c') => Key::C,
            Some('d') => Key::D,
            Some('e') => Key::E,
            Some('f') => Key::F,
            Some('g') => Key::G,
            Some('h') => Key::H,
            Some('i') => Key::I,
            Some('j') => Key::J,
            Some('k') => Key::K,
            Some('l') => Key::L,
            Some('m') => Key::M,
            Some('n') => Key::N,
            Some('o') => Key::O,
            Some('p') => Key::P,
            Some('q') => Key::Q,
            Some('r') => Key::R,
            Some('s') => Key::S,
            Some('t') => Key::T,
            Some('v') => Key::V,
            Some('w') => Key::W,
            Some('y') => Key::Y,
            Some('z') => Key::Z,
            Some(c) if !c.is_ascii() => by_place(code),
            Some(_) => Key::Other,
            None => return None,
        },
    })
}

/// the size of the picture area for a clip, in physical pixels: its own size if that fits in 90% of the main
/// display's free area, otherwise fitted to that, and never under 640 x 360 of the design's pixels.
pub fn client_for(video: (usize, usize)) -> (u32, u32) {
    let (max_w, max_h) = (ROOM.0.load(Ordering::Relaxed) as f64, ROOM.1.load(Ordering::Relaxed) as f64);
    let (vw, vh) = (video.0.max(1) as f64, video.1.max(1) as f64);
    let fitted = (max_w / vw).min(max_h / vh).min(1.0);
    (((vw * fitted).round()).max(MIN_CLIENT.0 * points()) as u32, ((vh * fitted).round()).max(MIN_CLIENT.1 * points()) as u32)
}

/// physical pixels for one point on the main display; 1 while no display has said.
fn points() -> f64 {
    Some(f32::from_bits(SCALE.load(Ordering::Relaxed)) as f64).filter(|scale| *scale > 0.0).unwrap_or(1.0)
}

/// the picture area of a window opened with no clip: 1180 x 720 of the design's pixels, as on windows.
pub fn client_for_none() -> (u32, u32) {
    client_for(((1180.0 * points()) as usize, (720.0 * points()) as usize))
}

/// where a window with `client` physical pixels of picture goes: centered in the display's free area.
fn placed(screen: &NSScreen, client: (u32, u32)) -> NSRect {
    let (scale, room) = (screen.backingScaleFactor(), screen.visibleFrame());
    let size = NSSize::new(client.0 as f64 / scale, client.1 as f64 / scale);
    NSRect::new(NSPoint::new(room.origin.x + (room.size.width - size.width) / 2.0, room.origin.y + (room.size.height - size.height) / 2.0), size)
}

/// takes the main display's scale and free area, which `client_for` reads. call on the main thread, before it.
pub fn measure_display(mtm: MainThreadMarker) {
    let Some(screen) = NSScreen::mainScreen(mtm) else { return };
    let (scale, room) = (screen.backingScaleFactor(), screen.visibleFrame());
    SCALE.store((scale as f32).to_bits(), Ordering::Relaxed);
    ROOM.0.store((room.size.width * scale * 0.9) as u32, Ordering::Relaxed);
    ROOM.1.store((room.size.height * scale * 0.9) as u32, Ordering::Relaxed);
}

/// the window, black, on screen at once. `client` is the picture area in physical pixels. call on the main thread.
pub fn create(client: (u32, u32), events: Sender<Event>) -> Result<Handle, String> {
    let mtm = MainThreadMarker::new().ok_or("the window is made on the main thread")?;
    *EVENTS.lock().unwrap() = Some(events);
    STILL.store(NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion(), Ordering::Relaxed);
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    // lives as long as the program: the application and the window hold it without keeping it
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // the dock's picture of the program: a bare program, with no bundle round it, has none of its own
    if let Some(icon) = NSImage::initWithData(objc2::AllocAnyThread::alloc(), &NSData::with_bytes(include_bytes!("../../assets/icon/eyepiece.png"))) {
        unsafe { app.setApplicationIconImage(Some(&icon)) };
    }
    app.finishLaunching();
    let screen = NSScreen::mainScreen(mtm).ok_or("no display")?;
    let rect = placed(&screen, client);
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::Resizable | NSWindowStyleMask::Miniaturizable;
    let window: Retained<Frame> = unsafe { msg_send![Frame::alloc(mtm), initWithContentRect: rect, styleMask: style, backing: NSBackingStoreType::Buffered, defer: false] };
    let window: Retained<NSWindow> = Retained::into_super(window);
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str("eyepiece"));
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    std::mem::forget(delegate);
    // black until the first frame, not the system's window gray
    window.setBackgroundColor(Some(&NSColor::blackColor()));
    window.setHasShadow(true);
    window.setContentMinSize(NSSize::new(MIN_CLIENT.0, MIN_CLIENT.1));
    window.setAcceptsMouseMovedEvents(true);
    let view = PictureView::alloc(mtm).set_ivars(ViewState { wheel: Cell::new(0.0), pressed: RefCell::new(None), restore: Cell::new(None), pointer: Cell::new(Pointer::Arrow), buttons: RefCell::new(None) });
    let view: Retained<PictureView> = unsafe { msg_send![super(view), initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), rect.size)] };
    // the layer is set up here, before the view has it: the render thread only draws into it
    let layer = CAMetalLayer::new();
    layer.setDevice(MTLCreateSystemDefaultDevice().as_deref());
    layer.setPixelFormat(super::metal::TARGETS[0]);
    // a frame may wait for the display behind one other, not two: a step shows a refresh sooner
    layer.setMaximumDrawableCount(2);
    // the picture's alpha is the file's, not a way to see through the window
    layer.setOpaque(true);
    layer.setContentsScale(screen.backingScaleFactor());
    layer.setDrawableSize(CGSize { width: client.0 as f64, height: client.1 as f64 });
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    // the pointer's moves and its leaving, wherever the view's edges are
    let tracking = NSTrackingAreaOptions::MouseMoved | NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways | NSTrackingAreaOptions::InVisibleRect;
    let area = unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(mtm.alloc(), view.bounds(), tracking, Some(&view), None) };
    view.addTrackingArea(&area);
    view.registerForDraggedTypes(&NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]));
    window.setContentView(Some(&view));
    window.makeFirstResponder(Some(&view));
    let buttons = Buttons::new(mtm);
    buttons.place(&window);
    view.ivars().buttons.replace(Some(buttons));
    view.measured();
    window.makeKeyAndOrderFront(None);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    watch(&window);
    // the layer lives as long as the view, which is kept
    let handle = Handle(Retained::as_ptr(&layer));
    WINDOW.set(Some((window, view)));
    Ok(handle)
}

/// runs the window's events until it closes. call on the thread that made the window.
pub fn run() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    while !CLOSED.load(Ordering::Relaxed) {
        // what appkit hands out while one event is dealt with is let go when it is done
        objc2::rc::autoreleasepool(|_| {
            if let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&NSDate::distantFuture()), unsafe { NSDefaultRunLoopMode }, true) {
                app.sendEvent(&event);
            }
        });
    }
}

/// grows the window so its picture area is at least `client`, centered: for a newly opened clip. the window never
/// shrinks, and a fullscreen or zoomed one stays as it is.
pub fn fit(_window: Handle, client: (u32, u32)) {
    on_main(move |window, view, _| {
        let now = (CLIENT.0.load(Ordering::Relaxed), CLIENT.1.load(Ordering::Relaxed));
        let client = (client.0.max(now.0), client.1.max(now.1));
        let Some(screen) = window.screen() else { return };
        if client != now && view.ivars().restore.get().is_none() && !window.isZoomed() {
            window.setFrame_display(placed(&screen, client), true);
        }
    });
}

/// physical pixels for one of the design's pixels on the display the window is on: 2 on a retina display.
pub fn scale(_window: Handle) -> f32 {
    f32::from_bits(SCALE.load(Ordering::Relaxed)).max(0.5)
}

pub fn client_size(_window: Handle) -> (u32, u32) {
    (CLIENT.0.load(Ordering::Relaxed), CLIENT.1.load(Ordering::Relaxed))
}

/// asks the window to show a title, without waiting for it. mission control and the window menu show the title.
pub fn set_title(_window: Handle, title: &str) {
    let title = title.to_string();
    on_main(move |window, _, _| window.setTitle(&NSString::from_str(&title)));
}

/// has the window follow the pointer, whose button is down, until the button comes up. the picture keeps playing
/// meanwhile.
pub fn begin_move(_window: Handle) {
    on_main(|window, view, _| {
        // only while the button is still down: a quick flick may have ended before this arrives
        let (Some(pressed), true) = (view.ivars().pressed.take(), NSEvent::pressedMouseButtons() & 1 != 0) else { return };
        window.performWindowDragWithEvent(&pressed);
        // the system has the drag from here, and the button's release with it: the controls are told it is up
        view.button(&pressed, Button::Left, false);
    });
}

pub fn minimize(_window: Handle) {
    on_main(|window, _, _| window.miniaturize(None));
}

/// fills the display's free area with the window, or puts it back.
pub fn toggle_maximize(_window: Handle) {
    on_main(|window, _, _| window.zoom(None));
}

pub fn close(_window: Handle) {
    on_main(|window, _, mtm| {
        send(Event::Close);
        window.close();
        CLOSED.store(true, Ordering::Relaxed);
        // the event loop is waiting for an event: this one tells it to look again
        let wake = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(NSEventType::ApplicationDefined, NSPoint::new(0.0, 0.0), NSEventModifierFlags::empty(), 0.0, 0, None, 0, 0, 0);
        if let Some(wake) = wake {
            NSApplication::sharedApplication(mtm).postEvent_atStart(&wake, true);
        }
    });
}

/// fills a display with the picture area, or puts the window back as it was. `next_display` fills the display
/// after the one the window is on, whether or not it is fullscreen already. the menu bar and the dock step aside.
pub fn set_fullscreen(_window: Handle, on: bool, next_display: bool) {
    on_main(move |window, view, mtm| {
        let (app, restore) = (NSApplication::sharedApplication(mtm), &view.ivars().restore);
        if !on {
            if let Some(was) = restore.take() {
                app.setPresentationOptions(NSApplicationPresentationOptions::Default);
                window.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::Resizable | NSWindowStyleMask::Miniaturizable);
                window.setHasShadow(true);
                window.setFrame_display(was, true);
                if let Some(buttons) = view.ivars().buttons.borrow().as_ref() {
                    buttons.place(window);
                }
                window.makeFirstResponder(Some(view));
            }
            return;
        }
        let screens = NSScreen::screens(mtm);
        let Some(mut screen) = window.screen() else { return };
        if next_display {
            let at = screens.iter().position(|s| s == screen).unwrap_or(0);
            if let Some(next) = screens.iter().nth((at + 1) % screens.len().max(1)) {
                screen = next;
            }
        }
        if restore.get().is_none() {
            restore.set(Some(window.frame()));
        }
        app.setPresentationOptions(NSApplicationPresentationOptions::HideDock | NSApplicationPresentationOptions::HideMenuBar);
        // no edges that resize and no shadow while it fills the display
        window.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::Miniaturizable);
        window.setHasShadow(false);
        window.setFrame_display(screen.frame(), true);
        if let Some(buttons) = view.ivars().buttons.borrow().as_ref() {
            buttons.place(window);
        }
        window.makeFirstResponder(Some(view));
    });
}

/// the pointer over the picture area: none (fullscreen with the controls away), the arrow, or the cross that markup
/// mode draws with.
#[derive(Clone, Copy, PartialEq)]
pub enum Pointer {
    Hidden,
    Arrow,
    Cross,
    /// two ways, left and right: on an edge that resizes a part of the browser
    Resize,
}

/// room at the strip's left for the system's window buttons, in the design's pixels.
pub fn buttons_room() -> Option<f32> {
    Some(BUTTONS_ROOM)
}

/// shows the system's window buttons, or hides them: they come and go with the strip.
pub fn show_buttons(_window: Handle, shown: bool) {
    on_main(move |_, view, _| {
        if let Some(buttons) = view.ivars().buttons.borrow().as_ref() {
            buttons.setHidden(!shown);
        }
    });
}

/// which pointer shows while it is over the picture area.
pub fn show_pointer(_window: Handle, pointer: Pointer) {
    on_main(move |window, view, _| {
        view.ivars().pointer.set(pointer);
        // hidden until it next moves, which brings the controls back and the pointer with them: hiding it outright
        // would hide it on every display, with no move over this window to bring it back
        if pointer == Pointer::Hidden {
            NSCursor::setHiddenUntilMouseMoves(true);
        }
        window.invalidateCursorRectsForView(view);
    });
}

/// puts text on the clipboard.
pub fn copy_text(text: &str) -> bool {
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    board.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
}

/// the text on the clipboard, when it holds any: for a field being typed into.
pub fn pasted_text() -> Option<String> {
    NSPasteboard::generalPasteboard().stringForType(unsafe { NSPasteboardTypeString }).map(|text| text.to_string())
}

/// puts a picture on the clipboard, as a png of 8 bits a channel. `pixels` is red, green, blue, alpha, 16 bits each,
/// row by row from the top.
pub fn copy_picture(width: u32, height: u32, pixels: &[u16]) -> bool {
    let narrow: Vec<u8> = pixels.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]].map(|v| ((v as u32 * 255 + 32767) / 65535) as u8)).collect();
    let mut file = Vec::new();
    let mut encoder = png::Encoder::new(&mut file, width, height);
    encoder.set_color(png::ColorType::Rgb);
    if encoder.write_header().and_then(|mut writer| writer.write_image_data(&narrow)).is_err() {
        return false;
    }
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    board.setData_forType(Some(&NSData::with_bytes(&file)), unsafe { NSPasteboardTypePNG })
}

/// a later launch handed over its file, or with none asked only for the window (instance.rs): the file opens in a
/// tab, and the window comes to the front, out of the dock if it was there.
pub fn handed(path: String) {
    if !path.is_empty() {
        send(Event::Open(path));
    }
    on_main(|window, _, mtm| {
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
        window.makeKeyAndOrderFront(None);
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    });
}

/// shows a file in finder, selected in its folder's window; a line in the window says so when finder could not.
pub fn reveal(file: &std::path::Path, wake: Sender<Event>) {
    let file = file.to_string_lossy().into_owned();
    on_main(move |_, _, _| {
        if !NSWorkspace::sharedWorkspace().selectFile_inFileViewerRootedAtPath(Some(&NSString::from_str(&file)), &NSString::new()) {
            let _ = wake.send(Event::Toast("finder could not show the file".into()));
        }
    });
}

/// opens a page in the system's browser, or a file in the program the system gives it; a line in the window says so
/// when it could not. the player itself sends nothing.
pub fn open_url(url: &str, wake: Sender<Event>) {
    let url = url.to_string();
    on_main(move |_, _, _| {
        // a path is a file's url, anything else is taken as a url as written
        let link = if url.starts_with('/') { Some(NSURL::fileURLWithPath(&NSString::from_str(&url))) } else { NSURL::URLWithString(&NSString::from_str(&url)) };
        if !link.is_some_and(|link| NSWorkspace::sharedWorkspace().openURL(&link)) {
            let _ = wake.send(Event::Toast("the system could not open the link".into()));
        }
    });
}

/// whether the system asks for less motion: the controls then stand still.
pub fn reduce_motion() -> bool {
    STILL.load(Ordering::Relaxed)
}

/// the time of day on the user's clock, as hours and minutes: 14:32.
pub fn clock() -> String {
    let now = NSCalendar::currentCalendar().components_fromDate(NSCalendarUnit::Hour | NSCalendarUnit::Minute, &NSDate::now());
    format!("{:02}:{:02}", now.hour(), now.minute())
}

/// the refresh rate of the display the window is on, in Hz; before that is known the main display's, as the system
/// reports it. 60 if it will not say.
pub fn refresh_rate() -> f64 {
    let known = f32::from_bits(HZ.load(Ordering::Relaxed));
    if known > 0.0 {
        return known as f64;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
        fn CGDisplayModeGetRefreshRate(mode: *mut c_void) -> f64;
        fn CGDisplayModeRelease(mode: *mut c_void);
    }
    unsafe {
        let mode = CGDisplayCopyDisplayMode(CGMainDisplayID());
        if mode.is_null() {
            return 60.0;
        }
        let rate = CGDisplayModeGetRefreshRate(mode);
        CGDisplayModeRelease(mode);
        if rate > 0.0 { rate } else { 60.0 }
    }
}
