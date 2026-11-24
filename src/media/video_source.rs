//! the lane for clips ffmpeg decodes (h.264, hevc, prores, notchlc): hardware where the system's decoder takes the
//! stream, software otherwise. a frame between key frames cannot be reached directly: the loader goes back to the key
//! frame before it and decodes forward, keeping what it passes.

use super::dec::{Decoder, Hardware, Layout, Picture};
use super::ff::{Input, Sample};
use super::source::{self, Loader, Source};
use ffmpeg_sys_next as ffi;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// memory given to decoded pictures kept after they were handed out. in a clip with key frames far apart, a frame
/// that is not kept costs a decode of everything since the key frame before it, so the frames just played or just
/// passed are kept: a step back inside them is one refresh. (with four kept, every fourth step back on a 4K h.264
/// clip with key frames 250 apart cost 1.5 s. measured.)
const KEPT_BYTES: usize = 512 << 20;
/// never fewer than this, and never more: a hardware decoder's surfaces are one array, and drivers cap its length
const KEPT_RANGE: (usize, usize) = (8, 32);
/// a clip whose frames all stand alone needs none kept for decoding; a few save repeating work
const KEPT_ALONE: usize = 4;

/// how many decoded pictures to keep for a clip of this size. sized for the largest layout a gpu decodes (two bytes a
/// sample, 4:2:0), since the layout is not known until the first frame is decoded.
fn kept(width: usize, height: usize, alone: bool) -> usize {
    if alone {
        return KEPT_ALONE;
    }
    (KEPT_BYTES / (width * height * 3).max(1)).clamp(KEPT_RANGE.0, KEPT_RANGE.1)
}
/// a frame this close ahead of the decoder is reached by decoding on, even past a key frame: restarting the decoder at
/// a key frame costs far more than a few frames of decoding (about 150 ms for 4K hevc in hardware, measured)
const NEAR: usize = 16;

pub struct VideoLane {
    pub source: Source<Picture>,
    /// how the pictures are stored, from the first one decoded
    pub layout: Layout,
    /// every frame is a key frame: any one is decoded without the ones before it
    pub alone: bool,
    /// the clip's key frames, shared with its loader, which puts them right (`keys_only`)
    keys: Arc<Mutex<Keys>>,
}

/// a clip's key frames by frame: none when every frame is one. at first each is where the file's table places it,
/// which in an open group (x265's default) is a few frames before the frame it is shown as
struct Keys {
    frames: Vec<usize>,
    /// a key frame asked for at its place is decoded alone: taken as it is, and put at the frame it turned out to be
    only: bool,
}

/// how a decoded picture's time tells which frame it is.
#[derive(Clone)]
pub enum Times {
    /// every frame lasts this long, in the stream's units: frames are counted from the first picture decoded
    Even(i64),
    /// when each frame is shown, in order: frames that vary, or times the container rounds (mkv)
    Shown(Vec<i64>),
}

struct VideoLoader {
    input: Input,
    decoder: Decoder,
    /// the stream's samples in decode order, as the container lists them
    samples: Vec<Sample>,
    /// the time of the first frame shown, in the stream's units, and how a time gives a frame
    first_time: i64,
    times: Times,
    /// the frame the decoder puts out next, when that is known
    next: Option<usize>,
    recent: VecDeque<(usize, Arc<Picture>)>,
    /// how many pictures `recent` holds at most
    kept: usize,
    /// samples that are decoded and not shown (an edit list hides them): frame n is at about place n + hidden
    hidden: usize,
    /// the file and the stream, when frames are decoded several at once and each worker opens its own decoder
    reopen: Option<(String, usize)>,
    keys: Arc<Mutex<Keys>>,
}

/// true when ffmpeg's decoder for the stream shares one frame out across cores.
fn splits_frames(input: &Input, stream: usize) -> bool {
    unsafe {
        let codec = ffi::avcodec_find_decoder((*(**(*input.ctx).streams.add(stream)).codecpar).codec_id);
        !codec.is_null() && (*codec).capabilities & ffi::AV_CODEC_CAP_SLICE_THREADS as i32 != 0
    }
}

impl VideoLane {
    /// opens the file for decoding and decodes the first frame, which says how pictures are stored. `times` says which
    /// frame a decoded picture is.
    pub fn open(path: &str, stream: usize, samples: Vec<Sample>, times: Times, size: (usize, usize), hardware: Option<Hardware>) -> Result<VideoLane, String> {
        let input = Input::open(path)?;
        // when every sample is a key frame the frames stand alone: in software, decode one at a time, so a step
        // never waits behind other frames
        let alone = samples.iter().all(|s| s.key);
        let kept = kept(size.0, size.1, alone);
        let mut decoder = Decoder::open(&input, stream, hardware.as_ref(), alone, kept)?;
        let mut first = decoder.next(&input)?.ok_or("no picture in this file")?;
        if hardware.is_some() && !first.hardware() {
            // the gpu did not take this stream (prores, 10-bit h.264, 4:2:2 on most cards): start again in software,
            // which wants its own threads
            decoder = Decoder::open(&input, stream, None, alone, kept)?;
            decoder.seek(&input, samples.first().map_or(0, |s| s.time))?;
            first = decoder.next(&input)?.ok_or("no picture in this file")?;
        }
        let layout = first.layout()?;
        let hidden = samples.iter().filter(|s| s.hidden).count();
        // frame n is at about place n + hidden, as `load` takes it
        let keys = if alone { Vec::new() } else { (hidden..samples.len()).filter(|&at| samples[at].key).map(|at| at - hidden).collect() };
        let keys = Arc::new(Mutex::new(Keys { frames: keys, only: false }));
        // frames that stand alone, in software, from a decoder that keeps each frame to one core (notchlc): several at
        // once, each worker with a decoder of its own. prores shares a frame out across cores itself
        let shared = alone && !layout.hardware && !splits_frames(&input, stream);
        let reopen = shared.then(|| (path.to_string(), stream));
        let mut loader = VideoLoader { input, decoder, samples, first_time: first.time(), times, next: Some(1), recent: VecDeque::new(), kept: if shared { 0 } else { kept }, hidden, reopen, keys: keys.clone() };
        let first = Arc::new(first);
        let source = if shared {
            // counted as four planes the size of luma: the most a software picture holds (4:4:4 with alpha, as
            // notchlc's). only a software picture has planes to ask: a hardware one is a surface on the gpu
            let frame_bytes = first.plane(0, layout.chroma_shift).0.len() * 4;
            Source::start_shared(loader, source::budget(), frame_bytes, (0, first))?
        } else {
            loader.remember(0, first);
            Source::start(loader)?
        };
        Ok(VideoLane { source, layout, alone, keys })
    }

    /// the frames decoding can start at, as far as they are known (`Keys`). empty when every frame stands alone.
    pub fn keys(&self) -> Vec<usize> {
        self.keys.lock().unwrap().frames.clone()
    }

    /// for a reader that asks for nothing but key frames (the timebar's): from now on a key frame asked for at its
    /// place is taken as it is, never the group before it decoded for the frame at that place, and `keys` learns
    /// the frame it is.
    pub fn keys_only(&self) {
        self.keys.lock().unwrap().only = true;
    }
}

impl VideoLoader {
    fn remember(&mut self, frame: usize, picture: Arc<Picture>) {
        if self.kept == 0 {
            return;
        }
        if self.recent.len() == self.kept {
            self.recent.pop_front();
        }
        self.recent.push_back((frame, picture));
    }

    /// which frame a decoded picture is, from its time.
    fn index_of(&self, time: i64) -> usize {
        match &self.times {
            Times::Even(step) => ((time - self.first_time) as f64 / (*step).max(1) as f64).round().max(0.0) as usize,
            // the frame shown nearest that time
            Times::Shown(shown) => {
                let after = shown.partition_point(|&t| t < time).min(shown.len() - 1);
                if after > 0 && time - shown[after - 1] < shown[after] - time { after - 1 } else { after }
            }
        }
    }

    /// the place, in decode order, of the last key frame at or before place `at`.
    fn key_at_or_before(&self, at: usize) -> usize {
        (0..=at.min(self.samples.len() - 1)).rev().find(|&i| self.samples[i].key).unwrap_or(0)
    }

    /// a time to seek to for the key frame at place `key` alone: inside its group, past when it is shown. at its own
    /// time ffmpeg's mov reader goes back to the key frame before one that is shown after its place (an open group),
    /// for the pictures shown before it, and the whole group before would be decoded.
    fn inside(&self, key: usize) -> i64 {
        self.samples[(key + 1..self.samples.len()).take_while(|&at| !self.samples[at].key).take(NEAR).last().unwrap_or(key)].time
    }
}

impl Loader for VideoLoader {
    type Item = Picture;

    fn load(&mut self, frame: usize, abandoned: &dyn Fn() -> bool) -> Result<Option<Arc<Picture>>, String> {
        if let Some((_, picture)) = self.recent.iter().find(|r| r.0 == frame) {
            return Ok(Some(picture.clone()));
        }
        // a key frame's place in decode order is never later than its place in display order, so the last key frame
        // at or before this frame's place is where decoding can start
        let mut key = self.key_at_or_before(frame + self.hidden);
        // a key frame asked for at its place by a reader of key frames alone (`VideoLane::keys_only`)
        let key_alone = {
            let keys = self.keys.lock().unwrap();
            keys.only && keys.frames.contains(&frame)
        };
        // decode on from where the decoder is when the frame is ahead and either near, or with no key frame in
        // between to start from instead. otherwise start at the key frame. with frames decoded several at once, the
        // ones in between are other workers'
        let near = if self.reopen.is_some() { 0 } else { NEAR };
        let walk = self.next.is_some_and(|next| next <= frame && (frame - next <= near || key <= next + self.hidden));
        if !walk {
            self.decoder.seek(&self.input, if key_alone { self.inside(key) } else { self.samples[key].time })?;
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
                return Ok(Some(picture));
            }
            if at < frame {
                passed_earlier = true;
                // a long way to go, and the frame may no longer be wanted: look before decoding on
                if abandoned() {
                    return Ok(None);
                }
                continue;
            }
            if passed_earlier || key == 0 {
                return Err("this frame is missing from the file".into());
            }
            // the key frame is shown after its place (an open group): a reader of key frames alone takes it as it is,
            // and learns which frame it is
            if key_alone {
                if let Some(place) = self.keys.lock().unwrap().frames.iter_mut().find(|k| **k == frame) {
                    *place = at;
                }
                return Ok(Some(picture));
            }
            // the key frame we started at is shown after the frame wanted: the frame belongs to the group before
            key = self.key_at_or_before(key - 1);
            self.decoder.seek(&self.input, self.samples[key].time)?;
            self.next = None;
        }
    }

    fn split(&self) -> Option<Self> {
        let (path, stream) = self.reopen.clone()?;
        let input = Input::open(&path).ok()?;
        let decoder = Decoder::open(&input, stream, None, true, 0).ok()?;
        let samples = self.samples.clone();
        Some(VideoLoader { input, decoder, samples, first_time: self.first_time, times: self.times.clone(), next: None, recent: VecDeque::new(), kept: 0, hidden: self.hidden, reopen: Some((path, stream)), keys: self.keys.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_frames_follow_the_memory_budget() {
        // 4K: 25 MB a picture at most, so about twenty fit in the budget
        assert_eq!(kept(3840, 2160, false), 21);
        // small pictures stop at the cap, huge ones at the floor
        assert_eq!(kept(1920, 1080, false), 32);
        assert_eq!(kept(7680, 4320, false), 8);
        // frames that stand alone need none kept for decoding
        assert_eq!(kept(3840, 2160, true), 4);
    }
}
