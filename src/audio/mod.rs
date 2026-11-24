//! a clip's sound: decoded on its own thread, played on the system's default output, and the clock while a clip with
//! sound plays (the design: "clips with sound: audio is the clock"). the player loops by playing again from the
//! start; nothing here loops.

mod decode;
mod out;
mod ring;
mod speed;

use crate::media::source::guarded;
use decode::Decoder;
use out::{Feed, Output, Shared};
use speed::Speed;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// how long the decode thread rests when the device has all the sound it can hold
const REST: Duration = Duration::from_millis(5);

pub struct Info {
    /// ffmpeg's name for the codec
    pub codec: String,
    pub channels: u16,
    pub sample_rate: u32,
}

pub struct Audio {
    info: Info,
    /// to the decode thread: the next play, or None to stop
    commands: Option<Sender<Option<Play>>>,
    worker: Option<std::thread::JoinHandle<()>>,
    shared: Arc<Shared>,
    /// None when there is no device, or it would not open
    output: Option<Output>,
    /// what is playing: its feed, the second it started from, and its speed. None while paused or silent
    playing: Option<(Arc<Feed>, f64, f64)>,
    volume: f32,
    muted: bool,
}

/// one play, for the decode thread.
struct Play {
    feed: Arc<Feed>,
    seconds: f64,
    speed: f64,
    rate: u32,
    channels: u16,
}

impl Audio {
    /// None when the file has no audio stream.
    pub fn open(path: &str) -> Result<Option<Audio>, String> {
        let Some((decoder, info)) = Decoder::open(path)? else { return Ok(None) };
        let (commands, theirs) = mpsc::channel();
        let worker = std::thread::Builder::new().name("sound".into()).spawn(move || work(decoder, theirs)).map_err(|e| e.to_string())?;
        let shared = Arc::new(Shared::new());
        // no device is not a fault: the clip plays without sound, on the display's clock
        let output = Output::open(shared.clone());
        Ok(Some(Audio { info, commands: Some(commands), worker: Some(worker), shared, output, playing: None, volume: 1.0, muted: false }))
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    /// starts, or restarts, playing from `seconds` into the clip at `speed` (1.0 is normal).
    /// pitch is kept from 0.5 to 2; outside that the sound is silent, as the design says.
    pub fn play(&mut self, seconds: f64, speed: f64) {
        if !speed::WITH_SOUND.contains(&speed) {
            return self.pause();
        }
        // a device that went away, or was never there, may be back
        if self.output.as_ref().is_none_or(|o| o.lost()) {
            self.output = Output::open(self.shared.clone());
        }
        let Some(output) = &self.output else { return self.pause() };
        let feed = Arc::new(Feed::new(output.rate, output.channels));
        self.send(Some(Play { feed: feed.clone(), seconds, speed, rate: output.rate, channels: output.channels }));
        self.shared.set_feed(Some(feed.clone()));
        self.playing = Some((feed, seconds, speed));
    }

    pub fn pause(&mut self) {
        self.send(None);
        self.shared.set_feed(None);
        self.playing = None;
    }

    /// 0 to 1
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        self.shared.set_gain(if self.muted { 0.0 } else { self.volume });
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.shared.set_gain(if self.muted { 0.0 } else { self.volume });
    }

    /// where the sound is now, in seconds of the clip, from the device's own count of samples played.
    /// this is the clock when a clip has sound. None while paused or silent.
    pub fn position(&self) -> Option<f64> {
        let (feed, start, speed) = self.playing.as_ref()?;
        let output = self.output.as_ref().filter(|o| !o.lost())?;
        Some(clip_seconds(*start, feed.heard(Instant::now(), output.rate), output.rate, *speed))
    }

    fn send(&self, play: Option<Play>) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(play);
        }
    }
}

impl Drop for Audio {
    /// stops the decode thread and waits for it, so the file is closed when this returns.
    fn drop(&mut self) {
        self.commands.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// seconds into the clip after `heard` frames at the device's `rate`, played from `start` at `speed`.
fn clip_seconds(start: f64, heard: u64, rate: u32, speed: f64) -> f64 {
    start + heard as f64 / rate as f64 * speed
}

/// the decode thread: one play at a time, decoded ahead of the device as far as its ring allows.
fn work(mut decoder: Decoder, commands: Receiver<Option<Play>>) {
    let mut job: Option<Job> = None;
    loop {
        // with nothing to do, wait for a command; with the ring full, rest a little; otherwise only look
        let command = match &job {
            None => commands.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(j) => commands.recv_timeout(if j.waiting() { REST } else { Duration::ZERO }),
        };
        match command {
            Ok(play) => job = play.and_then(|p| Job::start(&mut decoder, p)),
            Err(RecvTimeoutError::Timeout) => {
                if job.as_mut().is_some_and(|j| !j.step(&mut decoder)) {
                    job = None;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// one play, on the decode thread.
struct Job {
    feed: Arc<Feed>,
    speed: Speed,
    /// decoded, and not yet all in the ring
    chunk: Vec<f32>,
    sent: usize,
}

impl Job {
    /// None when the sound cannot start: the feed is then over, and the clock runs on in silence.
    fn start(decoder: &mut Decoder, play: Play) -> Option<Job> {
        match guarded(|| Speed::start(decoder, play.seconds, play.speed, play.rate, play.channels)) {
            Ok(speed) => Some(Job { feed: play.feed, speed, chunk: Vec::new(), sent: 0 }),
            Err(_) => {
                play.feed.end();
                None
            }
        }
    }

    /// the ring had no room for all of the last chunk.
    fn waiting(&self) -> bool {
        self.sent < self.chunk.len()
    }

    /// decodes the next chunk once the last is all in the ring, and puts in what fits. false when the sound is over.
    fn step(&mut self, decoder: &mut Decoder) -> bool {
        if !self.waiting() {
            self.chunk.clear();
            self.sent = 0;
            // the end of the sound and a fault in it end it the same way: the clock runs on in silence
            if !guarded(|| self.speed.next(decoder, &mut self.chunk)).unwrap_or(false) {
                self.feed.end();
                return false;
            }
        }
        self.sent += self.feed.ring.push(&self.chunk[self.sent..]);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(name: &str) -> String {
        format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    /// all of a clip's sound from `seconds` at `speed`, as interleaved float at `rate` and `channels`.
    fn sound(name: &str, seconds: f64, speed: f64, rate: u32, channels: u16) -> (Vec<f32>, Info) {
        let (mut decoder, info) = Decoder::open(&media(name)).unwrap().expect("a sound stream");
        let mut speed = Speed::start(&mut decoder, seconds, speed, rate, channels).unwrap();
        let mut out = Vec::new();
        while speed.next(&mut decoder, &mut out).unwrap() {}
        (out, info)
    }

    /// one channel of the middle half of the sound, away from any fade at either end.
    fn middle(samples: &[f32], channel: usize, channels: usize) -> Vec<f32> {
        let one: Vec<f32> = samples.iter().skip(channel).step_by(channels).copied().collect();
        one[one.len() / 4..one.len() * 3 / 4].to_vec()
    }

    /// a tone's frequency, from how often it crosses zero. a crossing counts only from beyond a quarter of the peak on
    /// one side to beyond it on the other, so noise around zero adds none.
    fn frequency(one: &[f32], rate: u32) -> f64 {
        let edge = one.iter().fold(0.0f32, |m, s| m.max(s.abs())) / 4.0;
        let (mut side, mut crossings) = (0.0f32, 0);
        for s in one.iter().filter(|s| s.abs() > edge) {
            if s.signum() != side {
                crossings += (side != 0.0) as usize;
                side = s.signum();
            }
        }
        crossings as f64 / 2.0 / (one.len() as f64 / rate as f64)
    }

    fn loudness(one: &[f32]) -> f32 {
        (one.iter().map(|s| s * s).sum::<f32>() / one.len() as f32).sqrt()
    }

    fn near(value: f64, want: f64, within: f64) -> bool {
        (value - want).abs() <= within
    }

    #[test]
    fn stereo_aac_decodes_to_its_length_with_each_side_its_own_tone() {
        let (out, info) = sound("tone48_stereo.mov", 0.0, 1.0, 48_000, 2);
        assert_eq!((info.codec.as_str(), info.channels, info.sample_rate), ("aac", 2, 48_000));
        // 1 s. the file's edit list trims aac's priming and padding, so the count is exact
        assert_eq!(out.len() / 2, 48_000);
        let (left, right) = (middle(&out, 0, 2), middle(&out, 1, 2));
        assert!(loudness(&left) > 0.3, "{}", loudness(&left));
        assert!(near(frequency(&left, 48_000), 440.0, 4.0), "{}", frequency(&left, 48_000));
        assert!(near(frequency(&right, 48_000), 660.0, 6.0), "{}", frequency(&right, 48_000));
    }

    #[test]
    fn mono_pcm_is_resampled_to_the_device_rate_on_both_sides() {
        let (out, info) = sound("tone44_mono.mov", 0.0, 1.0, 48_000, 2);
        assert_eq!((info.codec.as_str(), info.channels, info.sample_rate), ("pcm_u8", 1, 44_100));
        // 44 100 frames in, 1 s at 48 kHz out, give or take the resampler's filter
        assert!((out.len() as i64 / 2 - 48_000).abs() <= 32, "{}", out.len() / 2);
        let (left, right) = (middle(&out, 0, 2), middle(&out, 1, 2));
        assert_eq!(left, right);
        assert!(loudness(&left) > 0.03, "{}", loudness(&left));
        assert!(near(frequency(&left, 48_000), 440.0, 4.0), "{}", frequency(&left, 48_000));
    }

    #[test]
    fn a_seek_lands_on_the_sample() {
        let (all, _) = sound("tone48_stereo.mov", 0.0, 1.0, 48_000, 2);
        let (half, _) = sound("tone48_stereo.mov", 0.5, 1.0, 48_000, 2);
        assert_eq!(half.len(), all.len() - 24_000 * 2);
        // against the sound decoded from the start: on the sample it is far closer than a frame either way. not
        // equal, since aac does not decode bit for bit the same after a seek
        let off = |from: usize| half.iter().zip(&all[from * 2..]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(off(24_000) < 0.01 && off(23_999) > 0.02 && off(24_001) > 0.02, "{} {} {}", off(23_999), off(24_000), off(24_001));
    }

    #[test]
    fn pitch_is_kept_from_half_to_double_speed() {
        for speed in [0.5, 2.0] {
            let (out, _) = sound("tone48_stereo.mov", 0.0, speed, 48_000, 2);
            // the length follows the speed exactly
            assert_eq!(out.len() / 2, (48_000.0 / speed) as usize, "at {speed}");
            // and the tones stay where they were: played faster or slower without the stretcher, they would move
            // by the same factor
            let (left, right) = (middle(&out, 0, 2), middle(&out, 1, 2));
            assert!(loudness(&left) > 0.2, "at {speed}: {}", loudness(&left));
            assert!(near(frequency(&left, 48_000), 440.0, 9.0), "at {speed}: {}", frequency(&left, 48_000));
            assert!(near(frequency(&right, 48_000), 660.0, 13.0), "at {speed}: {}", frequency(&right, 48_000));
        }
    }

    #[test]
    fn the_clock_runs_at_the_speed() {
        assert_eq!(clip_seconds(1.0, 48_000, 48_000, 1.0), 2.0);
        assert_eq!(clip_seconds(1.0, 48_000, 48_000, 0.5), 1.5);
        assert_eq!(clip_seconds(1.0, 48_000, 48_000, 2.0), 3.0);
        assert_eq!(clip_seconds(0.0, 22_050, 44_100, 2.0), 1.0);
    }

    #[test]
    fn a_clip_without_sound_has_none() {
        assert!(Audio::open(&media("h264_64.mp4")).unwrap().is_none());
    }
}
