//! macos: the window and the metal renderer, as the player (`crate::player`) knows them.

mod metal;
mod paint;
mod ui_paint;
pub mod window;

pub use metal::Gpu;
pub use paint::Paint;
pub use ui_paint::UiPainter;

use crate::media::clip::Clip;
use std::sync::mpsc;

/// opens the window and runs until it closes. `first` is the file to open at once, if any.
pub fn run(first: Option<String>) -> Result<(), String> {
    let mtm = objc2::MainThreadMarker::new().ok_or("the window is made on the main thread")?;
    window::measure_display(mtm);
    // the header is read before the window exists, so the window has the clip's shape from the start and never
    // jumps. a file that would have to be read through first (it lists no frames) is left for the window to open
    let first = first.map(|path| {
        let clip = Clip::open_known(&path);
        (path, clip)
    });
    let client = match &first {
        Some((_, Ok(clip))) => window::client_for((clip.width, clip.height)),
        _ => window::client_for_none(),
    };
    let (tx, rx) = mpsc::channel();
    // the render thread's way to wake itself: what it has read or written in the background says when it is done
    let wake = tx.clone();
    let handle = window::create(client, tx)?;
    let render = std::thread::Builder::new().name("render".into()).spawn(move || crate::player::run(handle, rx, wake, first)).map_err(|e| e.to_string())?;
    window::run();
    let _ = render.join();
    Ok(())
}
