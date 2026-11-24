//! the lane for stills: a numbered run of image files, or one file alone. every frame is its own file, read whole and
//! decoded on a worker, several at once, and frames already decoded stay in memory while the budget holds them.

use super::source::{self, Loader, Source};
use super::still::{self, Image, Samples};
use std::sync::Arc;

pub struct SeqLane {
    pub source: Source<Image>,
    /// how every frame's numbers are stored: a frame stored otherwise fails
    pub samples: Samples,
    /// as the frame the clip opened on has them, for what the renderer sets up before any frame arrives
    pub bits: u32,
}

struct SeqLoader<F> {
    /// the file of each frame
    path_of: Arc<F>,
    /// frames with no file, in order
    missing: Vec<usize>,
    /// the size and samples every frame must have, so the renderer can keep one texture: the first frame's
    shape: (usize, usize, Samples),
    /// a finished frame's memory, for the next frame
    spare: Option<Vec<u8>>,
}

impl SeqLane {
    /// starts the workers, frame i being the file `path_of(i)`. `missing` lists, in order, the frames with no file.
    /// `opened` is the frame the clip opens on, already decoded.
    pub fn open(missing: Vec<usize>, opened: (usize, Arc<Image>), path_of: impl Fn(usize) -> String + Send + Sync + 'static) -> Result<SeqLane, String> {
        let image = &opened.1;
        let (shape, bits, frame_bytes) = ((image.width, image.height, image.samples), image.bits, image.data.len());
        let loader = SeqLoader { path_of: Arc::new(path_of), missing, shape, spare: None };
        let source = Source::start_shared(loader, source::budget(), frame_bytes, opened)?;
        Ok(SeqLane { source, samples: shape.2, bits })
    }
}

impl<F: Fn(usize) -> String + Send + Sync + 'static> Loader for SeqLoader<F> {
    type Item = Image;

    fn load(&mut self, frame: usize, _abandoned: &dyn Fn() -> bool) -> Result<Option<Arc<Image>>, String> {
        if self.missing.binary_search(&frame).is_ok() {
            return Err("this frame's file is missing".into());
        }
        let image = still::decode_into(&(self.path_of)(frame), self.spare.take().unwrap_or_default())?;
        if (image.width, image.height, image.samples) != self.shape {
            return Err("this frame's size or depth differs from the clip's".into());
        }
        Ok(Some(Arc::new(image)))
    }

    fn retire(&mut self, image: Image) {
        self.spare = Some(image.data);
    }

    fn split(&self) -> Option<Self> {
        Some(SeqLoader { path_of: self.path_of.clone(), missing: self.missing.clone(), shape: self.shape, spare: None })
    }
}
