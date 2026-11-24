//! hap frames on demand: a worker reads frames by their place in the file (the container's sample table), decompresses
//! them in ordinary ram, and keeps the one on screen and its neighbors ready as texture blocks exactly as stored.
//! (never decompress into the gpu's own mapped memory: it is up to 100 times slower. phase 0.)

use super::ff::{self, Sample};
use super::hap::{self, Tex};
use std::fs::File;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// frames kept ready past the one on screen, in the direction of travel
const AHEAD: usize = 4;
/// threads for one frame's snappy chunks
const SNAPPY_THREADS: usize = 4;

pub enum Frame {
    /// the frame's blocks, ready to copy into a texture
    Ready(Arc<Vec<u8>>),
    /// not read yet: ask again
    Pending,
    /// this frame cannot be shown, and why
    Failed(String),
}

pub struct HapSource {
    shared: Arc<Shared>,
    pub tex: Tex,
    /// the texture's real size: the picture rounded up to whole 4x4 blocks
    pub padded: (usize, usize),
}

struct Shared {
    state: Mutex<State>,
    /// the worker sleeps on this until the wanted frames change
    wake: Condvar,
    /// the render thread waits on this for a frame to arrive
    arrived: Condvar,
}

struct State {
    frame: usize,
    playing: bool,
    ready: Vec<(usize, Arc<Vec<u8>>)>,
    failed: Vec<(usize, String)>,
    spare: Vec<Vec<u8>>,
    quit: bool,
}

/// the layer that holds the picture: hap q alpha carries a second, alpha-only layer after it.
fn picture<'a, 'b>(layers: &'b [hap::Layer<'a>]) -> Result<&'b hap::Layer<'a>, String> {
    layers.iter().find(|l| l.tex != Tex::Bc4).or(layers.first()).ok_or_else(|| "hap frame with no texture".to_string())
}

impl HapSource {
    /// reads frame 0 to learn the texture format, then starts the worker.
    pub fn open(path: &str, samples: Vec<Sample>, width: usize, height: usize) -> Result<HapSource, String> {
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
        let shared = Arc::new(Shared { state: Mutex::new(State { frame: 0, playing: false, ready: Vec::new(), failed: Vec::new(), spare: Vec::new(), quit: false }), wake: Condvar::new(), arrived: Condvar::new() });
        let worker = shared.clone();
        std::thread::Builder::new().name("hap".into()).spawn(move || work(&worker, &file, &samples, tex, expect)).map_err(|e| e.to_string())?;
        Ok(HapSource { shared, tex, padded })
    }

    /// tells the worker which frame is on screen and whether the clip is running, so it knows what to keep ready.
    pub fn want(&self, frame: usize, playing: bool) {
        let mut s = self.shared.state.lock().unwrap();
        if s.frame != frame || s.playing != playing {
            s.frame = frame;
            s.playing = playing;
            self.shared.wake.notify_one();
        }
    }

    /// the frame if it is ready, waiting up to `patience` for it to arrive.
    pub fn get(&self, frame: usize, patience: Duration) -> Frame {
        let settled = |s: &State| s.ready.iter().any(|r| r.0 == frame) || s.failed.iter().any(|f| f.0 == frame);
        let s = self.shared.state.lock().unwrap();
        let (s, _) = self.shared.arrived.wait_timeout_while(s, patience, |s| !settled(s)).unwrap();
        if let Some((_, blocks)) = s.ready.iter().find(|r| r.0 == frame) {
            return Frame::Ready(blocks.clone());
        }
        match s.failed.iter().find(|f| f.0 == frame) {
            Some((_, why)) => Frame::Failed(why.clone()),
            None => Frame::Pending,
        }
    }
}

impl Drop for HapSource {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().quit = true;
        self.shared.wake.notify_one();
    }
}

/// the frames worth having ready, most urgent first: the one on screen, then what comes next while playing, or
/// both neighbors while standing still, so a single step either way is already there.
fn wanted(frame: usize, playing: bool, frames: usize) -> Vec<usize> {
    if playing {
        return (0..=AHEAD.min(frames - 1)).map(|i| (frame + i) % frames).collect();
    }
    let mut w = vec![frame];
    if frame + 1 < frames {
        w.push(frame + 1);
    }
    if frame > 0 {
        w.push(frame - 1);
    }
    w
}

fn work(shared: &Shared, file: &File, samples: &[Sample], tex: Tex, expect: usize) {
    let mut raw = Vec::new();
    loop {
        let (frame, mut blocks) = {
            let mut s = shared.state.lock().unwrap();
            loop {
                if s.quit {
                    return;
                }
                let want = wanted(s.frame, s.playing, samples.len());
                // let go of what is no longer near, keeping the memory for the next frame
                let mut i = 0;
                while i < s.ready.len() {
                    if want.contains(&s.ready[i].0) {
                        i += 1;
                    } else if let Ok(buf) = Arc::try_unwrap(s.ready.swap_remove(i).1) {
                        s.spare.push(buf);
                    }
                }
                s.failed.retain(|f| want.contains(&f.0));
                let missing = want.into_iter().find(|f| !s.ready.iter().any(|r| r.0 == *f) && !s.failed.iter().any(|r| r.0 == *f));
                match missing {
                    Some(f) => break (f, s.spare.pop().unwrap_or_default()),
                    None => s = shared.wake.wait(s).unwrap(),
                }
            }
        };
        let result = guarded(|| load(file, &samples[frame], &mut raw, &mut blocks, tex, expect));
        let mut s = shared.state.lock().unwrap();
        match result {
            Ok(()) => s.ready.push((frame, Arc::new(blocks))),
            Err(why) => s.failed.push((frame, why)),
        }
        shared.arrived.notify_all();
    }
}

/// runs a step that reads an untrusted file. if it stops on something the parser did not foresee, that is this
/// frame's fault to report, not the end of the reader or of the program.
fn guarded<T>(step: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)).unwrap_or_else(|_| Err("the reader stopped on this frame".into()))
}

/// one frame from the file into `blocks`. a frame that does not hold what frame 0 promised is refused, not shown.
fn load(file: &File, sample: &Sample, raw: &mut Vec<u8>, blocks: &mut Vec<u8>, tex: Tex, expect: usize) -> Result<(), String> {
    raw.resize(sample.size, 0);
    ff::read_at(file, raw, sample.pos).map_err(|e| e.to_string())?;
    let layers = hap::parse(raw)?;
    let layer = picture(&layers)?;
    if layer.tex != tex || layer.len != expect {
        return Err("this frame's texture differs from the clip's first".into());
    }
    blocks.resize(expect, 0);
    hap::decompress(layer, blocks, SNAPPY_THREADS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standing_still_keeps_both_neighbors() {
        assert_eq!(wanted(5, false, 10), [5, 6, 4]);
        assert_eq!(wanted(0, false, 10), [0, 1]);
        assert_eq!(wanted(9, false, 10), [9, 8]);
        assert_eq!(wanted(0, false, 1), [0]);
    }

    #[test]
    fn playing_looks_ahead_and_goes_round() {
        assert_eq!(wanted(8, true, 10), [8, 9, 0, 1, 2]);
        assert_eq!(wanted(1, true, 3), [1, 2, 0]);
    }
}
