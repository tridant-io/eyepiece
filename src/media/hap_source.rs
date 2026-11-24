//! the hap lane: frames are read by their place in the file (the container's sample table) and decompressed in
//! ordinary ram into texture blocks exactly as stored.
//! (never decompress into the gpu's own mapped memory: it is up to 100 times slower. phase 0.)

use super::ff::{self, Sample};
use super::hap::{self, Tex};
use super::source::{guarded, Loader, Source};
use std::fs::File;
use std::sync::Arc;

/// threads for one frame's snappy chunks
const SNAPPY_THREADS: usize = 4;
/// finished frames whose memory is kept for the next ones
const SPARES: usize = 4;

pub struct HapLane {
    pub source: Source<Vec<u8>>,
    pub tex: Tex,
    /// the texture's real size: the picture rounded up to whole 4x4 blocks
    pub padded: (usize, usize),
}

struct HapLoader {
    file: File,
    samples: Vec<Sample>,
    tex: Tex,
    /// the bytes of texture every frame must hold
    expect: usize,
    raw: Vec<u8>,
    spare: Vec<Vec<u8>>,
}

/// the layer that holds the picture: hap q alpha carries a second, alpha-only layer after it.
fn picture<'a, 'b>(layers: &'b [hap::Layer<'a>]) -> Result<&'b hap::Layer<'a>, String> {
    layers.iter().find(|l| l.tex != Tex::Bc4).or(layers.first()).ok_or_else(|| "hap frame with no texture".to_string())
}

impl HapLane {
    /// reads frame 0 to learn the texture format, then starts the worker.
    pub fn open(path: &str, samples: Vec<Sample>, width: usize, height: usize) -> Result<HapLane, String> {
        let file = ff::open_shared(path)?;
        let first = samples.first().ok_or("no frames")?;
        let mut raw = vec![0u8; first.size];
        ff::read_at(&file, &mut raw, first.pos).map_err(|e| e.to_string())?;
        let (tex, len) = guarded(|| {
            let layers = hap::parse(&raw)?;
            let l = picture(&layers)?;
            Ok((l.tex, l.len))
        })?;
        let padded = hap::padded(width, height);
        let expect = padded.0 / 4 * (padded.1 / 4) * tex.block_bytes();
        if len != expect {
            return Err(format!("hap frame holds {len} bytes of texture, but {width} x {height} needs {expect}"));
        }
        let source = Source::start(HapLoader { file, samples, tex, expect, raw, spare: Vec::new() })?;
        Ok(HapLane { source, tex, padded })
    }
}

impl Loader for HapLoader {
    type Item = Vec<u8>;

    /// one frame from the file. a frame that does not hold what frame 0 promised is refused, not shown.
    fn load(&mut self, frame: usize, _abandoned: &dyn Fn() -> bool) -> Result<Option<Arc<Vec<u8>>>, String> {
        let sample = self.samples[frame];
        self.raw.resize(sample.size, 0);
        ff::read_at(&self.file, &mut self.raw, sample.pos).map_err(|e| e.to_string())?;
        let layers = hap::parse(&self.raw)?;
        let layer = picture(&layers)?;
        if layer.tex != self.tex || layer.len != self.expect {
            return Err("this frame's texture differs from the clip's first".into());
        }
        let mut blocks = self.spare.pop().unwrap_or_default();
        blocks.resize(self.expect, 0);
        hap::decompress(layer, &mut blocks, SNAPPY_THREADS)?;
        Ok(Some(Arc::new(blocks)))
    }

    fn retire(&mut self, blocks: Vec<u8>) {
        if self.spare.len() < SPARES {
            self.spare.push(blocks);
        }
    }
}
