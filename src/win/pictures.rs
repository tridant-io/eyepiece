//! the maker of the browser's pictures, on windows: a thread with a graphics device of its own and no window, and
//! one clip open at a time, read through the same lanes and shaders as the window's clips (`render::View`).

use super::d3d::Gpu;
use super::render::View;
use crate::media::clip::{Clip, Lane};
use crate::event::Event;
use crate::preview::make::{self, Pictures, Reader};
use crate::preview::spare::{self, Spare};
use crate::preview::Cache;
use std::cell::OnceCell;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// how long a frame is waited for before the picture is left out: a long group of 4K frames decodes within it
const WAIT: Duration = Duration::from_secs(5);

/// starts the maker. it makes nothing while `playing` is set. None when its thread will not start.
pub fn start(playing: Arc<AtomicBool>) -> Option<Pictures> {
    let (pictures, wants, got) = Pictures::new();
    let work = move || {
        // the device is made when the first picture is wanted, not at launch: a player that never shows its browser
        // never pays for it. with no device there are no pictures, and the cards stay plain
        let gpu: OnceCell<Option<Gpu>> = OnceCell::new();
        make::serve(wants, got, Cache::open(), &playing, |path| Open::new(gpu.get_or_init(|| Gpu::offscreen().ok()).as_ref()?, path));
    };
    std::thread::Builder::new().name("pictures".into()).spawn(work).ok().map(|_| pictures)
}

/// starts the spare reader of the clip on screen, which wakes the window when it has an answer. its device too is
/// made when first needed. None when its thread will not start.
pub fn start_spare(wake: Sender<Event>) -> Option<Spare> {
    let (spare, asks, answers) = Spare::new();
    let work = move || {
        let gpu: OnceCell<Option<Gpu>> = OnceCell::new();
        let told = || {
            let _ = wake.send(Event::Wake);
        };
        spare::serve(asks, answers, told, |path| Open::new(gpu.get_or_init(|| Gpu::offscreen().ok()).as_ref()?, path));
    };
    std::thread::Builder::new().name("spare reader".into()).spawn(work).ok().map(|_| spare)
}

struct Open<'a> {
    gpu: &'a Gpu,
    view: View,
}

impl<'a> Open<'a> {
    fn new(gpu: &'a Gpu, path: &Path) -> Option<Open<'a>> {
        // a file that would have to be read through to count its frames gets no picture for that
        let clip = Clip::open_known(path.to_str()?).ok()?;
        // the rate of the display is nothing to a clip that never plays
        let view = View::new(gpu, clip, 60.0).ok()?;
        // one frame at a time, and none kept: 24 frames of a 4K float sequence would be 3 GB
        match &view.clip.lane {
            Lane::Hap(hap) => hap.source.keep_only_wanted(),
            Lane::Video(video) => video.source.keep_only_wanted(),
            Lane::Stills(stills) => stills.source.keep_only_wanted(),
            Lane::Waiting(_) => {}
        }
        Some(Open { gpu, view })
    }
}

impl Reader for Open<'_> {
    fn frames(&self) -> usize {
        self.view.clip.timeline.frames()
    }

    fn size(&self) -> (u32, u32) {
        (self.view.clip.width as u32, self.view.clip.height as u32)
    }

    fn stands_alone(&self) -> bool {
        match &self.view.clip.lane {
            Lane::Video(video) => video.alone,
            Lane::Hap(_) | Lane::Stills(_) => true,
            Lane::Waiting(_) => false,
        }
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

    /// a real clip's seam: its last frame against its first differs, and a frame against itself does not.
    #[test]
    fn a_clips_last_frame_is_compared_with_its_first() {
        let gpu = Gpu::offscreen().expect("a graphics device");
        let mut open = Open::new(&gpu, Path::new(&format!("{}/tests/media/hapq_64.mov", env!("CARGO_MANIFEST_DIR")))).expect("it opens");
        let (last, first) = (open.read(open.frames() - 1, 64, 64).expect("the last frame"), open.read(0, 64, 64).expect("the first"));
        let seam = spare::compare(&last, &first);
        assert!(!seam.identical && seam.change > 0.0, "{seam:?}");
        assert!(spare::compare(&first, &open.read(0, 64, 64).expect("the first again")).identical);
    }

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
