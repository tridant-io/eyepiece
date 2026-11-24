//! the lane for stills: a numbered run of image files, or one file alone. every frame is its own file, read whole and
//! decoded on the worker.

use super::source::{Loader, Source};
use super::still::{self, Image, Samples};
use std::sync::Arc;

pub struct SeqLane {
    pub source: Source<Image>,
}

struct SeqLoader<F> {
    /// the file of each frame
    path_of: F,
    /// frames with no file, in order
    missing: Vec<usize>,
    /// the size and samples every frame must have, so the renderer can keep one texture: the first frame's
    shape: (usize, usize, Samples),
    /// the frame decoded when the clip was opened, handed out once
    opened: Option<(usize, Arc<Image>)>,
}

impl SeqLane {
    /// starts the worker on `frames` frames, frame i being the file `path_of(i)`. `missing` lists, in order, the frames
    /// with no file. `opened` is the frame the clip opens on, already decoded.
    pub fn open(frames: usize, missing: Vec<usize>, opened: (usize, Arc<Image>), path_of: impl Fn(usize) -> String + Send + 'static) -> Result<SeqLane, String> {
        let (at, shape) = (opened.0, (opened.1.width, opened.1.height, opened.1.samples));
        let source = Source::start(frames, SeqLoader { path_of, missing, shape, opened: Some(opened) })?;
        // the worker starts at frame 0: point it at the frame the clip opens on before it decodes anything else
        source.want(at, false);
        Ok(SeqLane { source })
    }
}

impl<F: Fn(usize) -> String + Send + 'static> Loader for SeqLoader<F> {
    type Item = Image;

    fn load(&mut self, frame: usize, _abandoned: &dyn Fn() -> bool) -> Result<Option<Arc<Image>>, String> {
        if self.missing.binary_search(&frame).is_ok() {
            return Err("this frame's file is missing".into());
        }
        if self.opened.as_ref().is_some_and(|o| o.0 == frame) {
            return Ok(self.opened.take().map(|o| o.1));
        }
        let image = still::decode(&(self.path_of)(frame))?;
        if (image.width, image.height, image.samples) != self.shape {
            return Err("this frame's size or depth differs from the clip's".into());
        }
        Ok(Some(Arc::new(image)))
    }
}
