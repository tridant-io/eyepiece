//! macos: the window and the metal renderer, as the player (`crate::player`) knows them.

pub mod files;
mod instance;
mod metal;
mod paint;
mod swap;
mod ui_paint;
pub mod window;

pub use metal::Gpu;
pub use paint::Paint;
pub use swap::swap_in;
pub use ui_paint::UiPainter;

use crate::media::clip::Clip;
use std::ffi::c_void;
use std::sync::mpsc;

/// an autorelease pool for one turn of a loop: what appkit and metal hand out in the turn is let go at its end.
/// a thread with none of its own keeps all of it until the thread ends, which a loop's thread does not.
pub struct Pool(*mut c_void);

extern "C" {
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

impl Pool {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Pool {
        Pool(unsafe { objc_autoreleasePoolPush() })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}

/// opens the window and runs until it closes. `first` is the file to open at once, if any.
pub fn run(first: Option<String>) -> Result<(), String> {
    // a player already open takes the file, and this one is done before it has made anything; unless the settings
    // say one window is not wanted, and then this one opens a window of its own and takes no one's
    let settings = crate::settings::Settings::load();
    let later = match settings.one_window.then(|| instance::claim(first.as_deref())) {
        Some(instance::Claim::Theirs) => return Ok(()),
        Some(instance::Claim::Ours(later)) => later,
        None => None,
    };
    let mtm = objc2::MainThreadMarker::new().ok_or("the window is made on the main thread")?;
    // the application first: the displays are asked about through it
    objc2_app_kit::NSApplication::sharedApplication(mtm);
    window::measure_display(mtm);
    // before the first file: a sequence in it opens at the settings' rate
    crate::media::clip::set_sequence_rate(settings.sequence_rate);
    // the header is read before the window exists, so the window has the clip's shape from the start and never
    // jumps. a file that would have to be read through first (it lists no frames) is left for the window to open
    let first = first.map(|path| {
        let clip = Clip::open_known(&path);
        (path, clip)
    });
    // with no clip to give it a shape, the window is as it was left, when that is kept, fitted to this display
    let client = match (&first, settings.window_size.filter(|_| settings.remember_size)) {
        (Some((_, Ok(clip))), _) => window::client_for(clip.upright()),
        (_, Some((w, h))) => window::client_for((w as usize, h as usize)),
        _ => window::client_for_none(),
    };
    let (tx, rx) = mpsc::channel();
    // the render thread's way to wake itself: what it has read or written in the background says when it is done
    let wake = tx.clone();
    let handle = window::create(client, tx)?;
    if let Some(later) = later {
        instance::serve(later, window::handed);
    }
    let render = std::thread::Builder::new().name("render".into()).spawn(move || crate::player::run(handle, rx, wake, first, settings)).map_err(|e| e.to_string())?;
    window::run();
    let _ = render.join();
    Ok(())
}
