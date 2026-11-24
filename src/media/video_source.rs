//! the lane for clips ffmpeg decodes (h.264, hevc, prores): hardware where the system's decoder takes the stream,
//! software otherwise. a frame between key frames cannot be reached directly: the loader goes back to the key frame
//! before it and decodes forward, keeping what it passes.

use super::dec::{Decoder, Hardware, Layout, Picture};
use super::ff::{Input, Sample};
use super::source::{Loader, Source};
use std::collections::VecDeque;
use std::sync::Arc;

/// decoded pictures the loader keeps after handing them out, so a frame passed on the way to another is not decoded twice
const RECENT: usize = 4;
/// a frame this close ahead of the decoder is reached by decoding on, even past a key frame: restarting the decoder at
/// a key frame costs far more than a few frames of decoding (about 150 ms for 4K hevc in hardware, measured)
const NEAR: usize = 16;

pub struct VideoLane {
    pub source: Source<Picture>,
    /// how the pictures are stored, from the first one decoded
    pub layout: Layout,
}

struct VideoLoader {
    input: Input,
    decoder: Decoder,
    /// the stream's samples in decode order, as the container lists them
    samples: Vec<Sample>,
    /// the time of the first frame shown, and the time between frames, in the stream's units
    first_time: i64,
    step: i64,
    /// the frame the decoder puts out next, when that is known
    next: Option<usize>,
    recent: VecDeque<(usize, Arc<Picture>)>,
}

impl VideoLane {
    /// opens the file for decoding and decodes the first frame, which says how pictures are stored. `step` is the
    /// time between frames in the stream's units (the file's constant frame duration).
    pub fn open(path: &str, stream: usize, samples: Vec<Sample>, step: i64, hardware: Option<Hardware>) -> Result<VideoLane, String> {
        let input = Input::open(path)?;
        // when every sample is a key frame the frames stand alone: in software, decode one at a time, so a step
        // never waits behind other frames
        let alone = samples.iter().all(|s| s.key);
        let mut decoder = Decoder::open(&input, stream, hardware.as_ref(), alone)?;
        let mut first = decoder.next(&input)?.ok_or("no picture in this file")?;
        if hardware.is_some() && !first.hardware() {
            // the gpu did not take this stream (prores, 10-bit h.264, 4:2:2 on most cards): start again in software,
            // which wants its own threads
            decoder = Decoder::open(&input, stream, None, alone)?;
            decoder.seek(&input, samples.first().map_or(0, |s| s.time))?;
            first = decoder.next(&input)?.ok_or("no picture in this file")?;
        }
        let layout = first.layout()?;
        let frames = samples.len();
        let mut loader = VideoLoader { input, decoder, samples, first_time: first.time(), step: step.max(1), next: Some(1), recent: VecDeque::new() };
        loader.remember(0, Arc::new(first));
        Ok(VideoLane { source: Source::start(frames, loader)?, layout })
    }
}

impl VideoLoader {
    fn remember(&mut self, frame: usize, picture: Arc<Picture>) {
        if self.recent.len() == RECENT {
            self.recent.pop_front();
        }
        self.recent.push_back((frame, picture));
    }

    /// which frame a decoded picture is, from its time.
    fn index_of(&self, time: i64) -> usize {
        ((time - self.first_time) as f64 / self.step as f64).round().max(0.0) as usize
    }

    /// the place, in decode order, of the last key frame at or before place `at`.
    fn key_at_or_before(&self, at: usize) -> usize {
        (0..=at.min(self.samples.len() - 1)).rev().find(|&i| self.samples[i].key).unwrap_or(0)
    }
}

impl Loader for VideoLoader {
    type Item = Picture;

    fn load(&mut self, frame: usize) -> Result<Arc<Picture>, String> {
        if let Some((_, picture)) = self.recent.iter().find(|r| r.0 == frame) {
            return Ok(picture.clone());
        }
        // a key frame's place in decode order is never later than its place in display order, so the last key frame
        // at or before this frame's number is where decoding can start
        let mut key = self.key_at_or_before(frame);
        // decode on from where the decoder is when the frame is ahead and either near, or with no key frame in
        // between to start from instead. otherwise start at the key frame
        let walk = self.next.is_some_and(|next| next <= frame && (frame - next <= NEAR || key <= next));
        if !walk {
            self.decoder.seek(&self.input, self.samples[key].time)?;
            self.next = None;
        }
        let mut passed_earlier = walk;
        loop {
            let Some(picture) = self.decoder.next(&self.input)? else {
                return Err("the file ended before this frame".into());
            };
            let at = self.index_of(picture.time());
            let picture = Arc::new(picture);
            self.next = Some(at + 1);
            self.remember(at, picture.clone());
            if at == frame {
                return Ok(picture);
            }
            if at < frame {
                passed_earlier = true;
                continue;
            }
            if passed_earlier || key == 0 {
                return Err("this frame is missing from the file".into());
            }
            // the key frame we started at is shown after the frame wanted: the frame belongs to the group before
            key = self.key_at_or_before(key - 1);
            self.decoder.seek(&self.input, self.samples[key].time)?;
            self.next = None;
        }
    }
}
