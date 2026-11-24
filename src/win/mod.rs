//! windows: the window, the direct3d renderer and the render thread.

mod d3d;
mod instance;
mod render;
mod ui_paint;
mod window;

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
    let first = first.map(|path| {
        let clip = Clip::open(&path);
        (path, clip)
    });
    let client = match &first {
        Some((_, Ok(clip))) => window::client_for((clip.width, clip.height)),
        _ => (1180, 720),
    };
    let (tx, rx) = mpsc::channel();
    let handle = window::create(client, tx).map_err(|e| e.to_string())?;
    let render = std::thread::Builder::new().name("render".into()).spawn(move || render::run(handle, rx, first)).map_err(|e| e.to_string())?;
    window::run();
    let _ = render.join();
    Ok(())
}
