//! the sound at a speed: as decoded at 1×, through signalsmith stretch from 0.5× to 2× so it keeps its pitch, and
//! none beyond (the design, fig. 06).

use super::decode::Decoder;
use signalsmith_stretch::Stretch;
use std::ops::RangeInclusive;

/// the speeds that have sound
pub const WITH_SOUND: RangeInclusive<f64> = 0.5..=2.0;

pub struct Speed {
    speed: f64,
    channels: usize,
    /// None at 1×, where the stretcher is left out
    stretch: Option<Stretch>,
    /// decoded and not yet stretched
    input: Vec<f32>,
    /// frames into and out of the stretcher: out is kept at in / speed, so a frame out is exactly `speed` frames in
    frames_in: u64,
    frames_out: u64,
    /// output frames still to drop: the stretcher's first ones are from before the start
    drop: usize,
    done: bool,
}

impl Speed {
    /// the sound from `seconds` into the clip at `speed` (one of WITH_SOUND), as interleaved float at `rate` and
    /// `channels`.
    pub fn start(decoder: &mut Decoder, seconds: f64, speed: f64, rate: u32, channels: u16) -> Result<Speed, String> {
        decoder.seek(seconds, rate, channels)?;
        let ch = channels as usize;
        let mut s = Speed { speed, channels: ch, stretch: None, input: Vec::new(), frames_in: 0, frames_out: 0, drop: 0, done: false };
        if speed != 1.0 {
            let mut stretch = Stretch::preset_default(channels as u32, rate);
            // the stretcher's time starts its input latency before the first frames it is given, so those go in as
            // pre-roll and its time starts at `seconds`. its output latency is then the first output, which is dropped
            let pre = stretch.input_latency() * ch;
            while s.input.len() < pre && decoder.next(&mut s.input)? {}
            s.input.resize(s.input.len().max(pre), 0.0);
            stretch.seek(&s.input[..pre], speed);
            s.input.drain(..pre);
            s.drop = stretch.output_latency();
            s.stretch = Some(stretch);
        }
        Ok(s)
    }

    /// appends the next of the sound to `out`. false at its end.
    pub fn next(&mut self, decoder: &mut Decoder, out: &mut Vec<f32>) -> Result<bool, String> {
        let Some(stretch) = &mut self.stretch else { return decoder.next(out) };
        if self.done {
            return Ok(false);
        }
        let ch = self.channels;
        let more = decoder.next(&mut self.input)?;
        if !more {
            // silence carries the stretcher's time to the end of the sound
            self.input.resize(self.input.len() + stretch.input_latency() * ch, 0.0);
        }
        self.frames_in += (self.input.len() / ch) as u64;
        let total = (self.frames_in as f64 / self.speed).round() as u64;
        let at = out.len();
        out.resize(at + (total - self.frames_out) as usize * ch, 0.0);
        self.frames_out = total;
        stretch.process(&self.input, &mut out[at..]);
        self.input.clear();
        if !more {
            // and what it still holds is the last of it
            let tail = out.len();
            out.resize(tail + stretch.output_latency() * ch, 0.0);
            stretch.flush(&mut out[tail..]);
            self.done = true;
        }
        let dropped = self.drop.min((out.len() - at) / ch);
        out.drain(at..at + dropped * ch);
        self.drop -= dropped;
        Ok(true)
    }
}
