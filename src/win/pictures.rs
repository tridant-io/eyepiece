//! the maker of the browser's pictures, on windows: a thread with a graphics device of its own and no window, and
//! one clip open at a time, read through the same lanes and shaders as the window's clips (`render::View`).

use super::d3d::Gpu;
use super::render::View;
use crate::media::clip::Clip;
use crate::preview::make::{self, Pictures, Reader};
use crate::preview::Cache;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// how long a frame is waited for before the picture is left out: a long group of 4K frames decodes within it
const WAIT: Duration = Duration::from_secs(5);

/// starts the maker. it makes nothing while `playing` is set. None when its thread will not start.
pub fn start(playing: Arc<AtomicBool>) -> Option<Pictures> {
    let (pictures, wants, got) = Pictures::new();
    let work = move || {
        // with no device there are no pictures: the cards stay plain
        let Ok(gpu) = Gpu::offscreen() else { return };
        make::serve(wants, got, Cache::open(), &playing, |path| Open::new(&gpu, path));
    };
    std::thread::Builder::new().name("pictures".into()).spawn(work).ok().map(|_| pictures)
}

struct Open<'a> {
    gpu: &'a Gpu,
    view: View,
}

impl<'a> Open<'a> {
    fn new(gpu: &'a Gpu, path: &Path) -> Option<Open<'a>> {
        let clip = Clip::open(path.to_str()?).ok()?;
        // the rate of the display is nothing to a clip that never plays
        Some(Open { gpu, view: View::new(gpu, clip, 60.0).ok()? })
    }
}

impl Reader for Open<'_> {
    fn frames(&self) -> usize {
        self.view.clip.timeline.frames()
    }

    fn size(&self) -> (u32, u32) {
        (self.view.clip.width as u32, self.view.clip.height as u32)
    }

    fn read(&mut self, frame: usize, w: u32, h: u32) -> Option<Vec<u16>> {
        self.view.transport.go(frame as i64);
        // this frame and no other: nothing is read ahead for a picture
        let until = Instant::now() + WAIT;
        while self.view.waiting() && Instant::now() < until {
            self.view.load_wanting(self.gpu, Duration::from_millis(50), &[frame]);
        }
        if self.view.waiting() || self.view.fault.is_some() {
            return None;
        }
        self.view.read(self.gpu, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a picture of a real clip, through a device with no window: the frame asked for, at the size asked for.
    #[test]
    fn a_frame_is_read_with_no_window() {
        let gpu = Gpu::offscreen().expect("a graphics device");
        for name in ["hapq_64.mov", "h264_64.mp4", "prores_64.mov", "still_8bit.png"] {
            let mut open = Open::new(&gpu, Path::new(&format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR")))).unwrap_or_else(|| panic!("{name} opens"));
            let last = open.frames() - 1;
            let picture = open.read(last, 32, 32).unwrap_or_else(|| panic!("{name}: its last frame is read"));
            assert_eq!(picture.len(), 32 * 32 * 4, "{name}");
            // a picture, not a blank: some pixel differs from the first
            assert!(picture.chunks_exact(4).any(|p| p[..3] != picture[..3]), "{name} is one flat color");
        }
    }
}
