//! windows: the window and the direct3d renderer, as the player (`crate::player`) knows them.

mod d3d;
pub mod files;
mod instance;
mod paint;
mod swap;
mod ui_paint;
pub mod window;

/// what a loop holds for one turn: nothing here. on macos it lets go of what the system handed out in the turn.
pub struct Pool;

impl Pool {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Pool {
        Pool
    }
}

pub use d3d::Gpu;
pub use paint::Paint;
pub use swap::swap_in;
pub use ui_paint::UiPainter;

use crate::media::clip::Clip;
use std::sync::mpsc;

/// opens the window and runs until it closes. `first` is the file to open at once, if any. when a player is
/// running already, it is handed `first` instead, and this returns with nothing made; unless the settings say one
/// window is not wanted, and then this one opens a window of its own.
pub fn run(first: Option<String>) -> Result<(), String> {
    let settings = crate::settings::Settings::load();
    if settings.one_window && instance::handed_over(first.as_deref()) {
        return Ok(());
    }
    Gpu::early();
    // the header is read before the window exists, so the window has the clip's shape from the start and never
    // jumps. that takes a few milliseconds; the graphics device, which is being made meanwhile, takes far longer,
    // and the first frame is already being read
    // a file that would have to be read through first (it lists no frames) is left for the window to open
    let first = first.map(|path| {
        let clip = Clip::open_known(&path);
        (path, clip)
    });
    // with no clip to give it a shape, the window is as it was left, when that is kept, fitted to this display
    let client = match (&first, settings.window_size.filter(|_| settings.remember_size)) {
        (Some((_, Ok(clip))), _) => window::client_for(clip.upright()),
        (_, Some((w, h))) => window::client_for((w as usize, h as usize)),
        _ => (1180, 720),
    };
    let (tx, rx) = mpsc::channel();
    // the render thread's way to wake itself: what it has read or written in the background says when it is done
    let wake = tx.clone();
    let handle = window::create(client, tx).map_err(|e| e.to_string())?;
    let render = std::thread::Builder::new().name("render".into()).spawn(move || crate::player::run(handle, rx, wake, first, settings)).map_err(|e| e.to_string())?;
    window::run();
    let _ = render.join();
    Ok(())
}
