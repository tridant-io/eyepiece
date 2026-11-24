//! windows: the window and the direct3d renderer, as the player (`crate::player`) knows them.

mod d3d;
mod instance;
mod paint;
mod ui_paint;
pub mod window;

pub use d3d::Gpu;
pub use paint::Paint;
pub use ui_paint::UiPainter;

use crate::media::clip::Clip;
use std::sync::mpsc;

/// opens the window and runs until it closes. `first` is the file to open at once, if any. when a player is
/// running already, it is handed `first` instead, and this returns with nothing made.
pub fn run(first: Option<String>) -> Result<(), String> {
    if instance::handed_over(first.as_deref()) {
        return Ok(());
    }
    // the header is read before the window exists, so the window has the clip's shape from the start and never
    // jumps. that takes a few milliseconds; the graphics device takes a few hundred, and meanwhile the first frame
    // is already being read
    // a file that would have to be read through first (it lists no frames) is left for the window to open
    let first = first.map(|path| {
        let clip = Clip::open_known(&path);
        (path, clip)
    });
    let client = match &first {
        Some((_, Ok(clip))) => window::client_for((clip.width, clip.height)),
        _ => (1180, 720),
    };
    let (tx, rx) = mpsc::channel();
    // the render thread's way to wake itself: what it has read or written in the background says when it is done
    let wake = tx.clone();
    let handle = window::create(client, tx).map_err(|e| e.to_string())?;
    let render = std::thread::Builder::new().name("render".into()).spawn(move || crate::player::run(handle, rx, wake, first)).map_err(|e| e.to_string())?;
    window::run();
    let _ = render.join();
    Ok(())
}
