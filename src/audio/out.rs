//! the system's default output, through cpal. its callback takes the sound of the play of the moment and counts what
//! it hands the device: that count, and when the device says it will be heard, is the clock when a clip has sound.

use super::ring::Ring;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// how long making the stream may take: it is made when a clip opens or plays, on the player's own thread
const OPEN_WITHIN: Duration = Duration::from_millis(500);

/// the sound of one play, from the decode thread to the device.
pub struct Feed {
    pub ring: Ring,
    /// the sound has no more to give: its end, or a fault in it
    pub ended: AtomicBool,
    /// frames handed to the device that count as played
    counted: AtomicU64,
    stamp: Mutex<Stamp>,
    /// the most frames ever reported heard, so the clock never steps back
    heard: AtomicU64,
}

#[derive(Clone, Copy, Default)]
struct Stamp {
    /// frames counted before the last callback, and in it
    before: u64,
    given: u64,
    /// when the first of the last callback's frames reaches the speaker
    plays_at: Option<Instant>,
}

impl Feed {
    pub fn new(rate: u32, channels: u16) -> Feed {
        // half a second decoded ahead: a busy machine can hold the decode thread back for a while
        Feed { ring: Ring::new(rate as usize / 2 * channels as usize), ended: AtomicBool::new(false), counted: AtomicU64::new(0), stamp: Mutex::new(Stamp::default()), heard: AtomicU64::new(0) }
    }

    pub fn end(&self) {
        self.ended.store(true, Ordering::Release);
    }

    /// fills `out` for the device and counts it, for a callback whose first frame is heard at `plays_at`.
    fn take(&self, out: &mut [f32], channels: usize, plays_at: Instant) {
        // looked at before the ring: once the end is seen, all the sound is in the ring, so a short read means it is over
        let ended = self.ended.load(Ordering::Acquire);
        let n = self.ring.pop(out);
        out[n..].fill(0.0);
        // the sound counts, and once it is over the silence after it does too, so the clock runs on to the picture's
        // end. silence while the decoder catches up does not: the picture waits with the sound
        let given = (if ended { out.len() } else { n } / channels) as u64;
        let before = self.counted.fetch_add(given, Ordering::Relaxed);
        // a starved callback leaves the stamp alone: what the device holds ahead of it is silence, which the stamp
        // would take for sound still to be heard
        if given == 0 {
            return;
        }
        // the other side holds this only for a moment; if it is held now, the next callback stamps instead
        if let Ok(mut stamp) = self.stamp.try_lock() {
            *stamp = Stamp { before, given, plays_at: Some(plays_at) };
        }
    }

    /// the frames heard by `now`, at the device's `rate`. never fewer than the last answer.
    pub fn heard(&self, now: Instant, rate: u32) -> u64 {
        let stamp = *self.stamp.lock().unwrap_or_else(|e| e.into_inner());
        let heard = heard(&stamp, now, rate);
        self.heard.fetch_max(heard, Ordering::Relaxed).max(heard)
    }
}

/// frames heard by `now`: those counted before the last callback, less any still waiting in the device, or with as
/// many of the last callback's own as have reached the speaker. between callbacks the count moves with the time, so
/// the clock does not jump a callback's worth at a time.
fn heard(stamp: &Stamp, now: Instant, rate: u32) -> u64 {
    let Some(at) = stamp.plays_at else { return 0 };
    let frames = |d: Duration| (d.as_secs_f64() * rate as f64) as u64;
    match now.checked_duration_since(at) {
        Some(since) => stamp.before + frames(since).min(stamp.given),
        None => stamp.before.saturating_sub(frames(at - now)),
    }
}

/// what the player and the device's callback share.
pub struct Shared {
    /// the play to sound. the callback looks only when `changed` is set, and never waits for the lock
    next: Mutex<Option<Arc<Feed>>>,
    changed: AtomicBool,
    /// the volume, or 0 when muted, as an f32's bits
    gain: AtomicU32,
}

impl Shared {
    pub fn new() -> Shared {
        Shared { next: Mutex::new(None), changed: AtomicBool::new(false), gain: AtomicU32::new(1.0f32.to_bits()) }
    }

    pub fn set_feed(&self, feed: Option<Arc<Feed>>) {
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        *next = feed;
        self.changed.store(true, Ordering::Release);
    }

    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }
}

/// the device's callback, apart from the device: takes up a new play when there is one, and fills `out` from it.
fn fill(shared: &Shared, feed: &mut Option<Arc<Feed>>, out: &mut [f32], channels: usize, plays_at: Instant) {
    if shared.changed.load(Ordering::Acquire) {
        // if the lock is held, the new play is taken up on the next call
        if let Ok(next) = shared.next.try_lock() {
            *feed = next.clone();
            shared.changed.store(false, Ordering::Relaxed);
        }
    }
    match feed {
        Some(feed) => feed.take(out, channels, plays_at),
        None => out.fill(0.0),
    }
    let gain = f32::from_bits(shared.gain.load(Ordering::Relaxed));
    out.iter_mut().for_each(|s| *s *= gain);
}

pub struct Output {
    _stream: cpal::Stream,
    pub rate: u32,
    pub channels: u16,
    lost: Arc<AtomicBool>,
}

impl Output {
    /// the system's default output, always running and playing whatever `shared` holds, silence when nothing.
    /// None when there is no device or it will not open.
    pub fn open(shared: Arc<Shared>) -> Option<Output> {
        let device = cpal::default_host().default_output_device()?;
        let config = device.default_output_config().ok()?;
        let (rate, channels) = (config.sample_rate(), config.channels());
        if rate == 0 || channels == 0 {
            return None;
        }
        let lost = Arc::new(AtomicBool::new(false));
        let theirs = lost.clone();
        let mut feed = None;
        let stream = device
            .build_output_stream(
                cpal::StreamConfig { channels, sample_rate: rate, buffer_size: cpal::BufferSize::Default },
                move |out: &mut [f32], info: &cpal::OutputCallbackInfo| {
                    // what is written now is heard after what the device already holds
                    let t = info.timestamp();
                    let now = Instant::now();
                    let plays_at = now.checked_add(t.playback.duration_since(t.callback)).unwrap_or(now);
                    fill(&shared, &mut feed, out, channels as usize, plays_at);
                },
                move |e: cpal::Error| {
                    // a glitch, or a stream the system moved to a new default device, plays on. anything else is gone
                    if !matches!(e.kind(), cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied | cpal::ErrorKind::DeviceChanged) {
                        theirs.store(true, Ordering::Relaxed);
                    }
                },
                Some(OPEN_WITHIN),
            )
            .ok()?;
        stream.play().ok()?;
        Some(Output { _stream: stream, rate, channels, lost })
    }

    /// the device failed or went away: nothing more is heard through this output.
    pub fn lost(&self) -> bool {
        self.lost.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn heard_moves_with_the_speaker_between_callbacks() {
        let t = Instant::now() + 50 * MS;
        let stamp = Stamp { before: 960, given: 480, plays_at: Some(t) };
        // 10 ms before the last callback's first frame is heard, 480 frames of the one before are still to come
        assert_eq!(heard(&stamp, t - 10 * MS, 48_000), 480);
        assert_eq!(heard(&stamp, t, 48_000), 960);
        assert_eq!(heard(&stamp, t + 5 * MS, 48_000), 1200);
        // never past what the device was given
        assert_eq!(heard(&stamp, t + 1000 * MS, 48_000), 1440);
        assert_eq!(heard(&Stamp::default(), t, 48_000), 0);
    }

    #[test]
    fn the_callback_plays_the_newest_feed_and_counts_only_sound_until_it_ends() {
        let shared = Shared::new();
        let (mut feed, mut out, now) = (None, [9.0; 6], Instant::now());
        // nothing to play: silence
        fill(&shared, &mut feed, &mut out, 2, now);
        assert_eq!(out, [0.0; 6]);

        let a = Arc::new(Feed::new(48_000, 2));
        a.ring.push(&[1.0, -1.0, 2.0, -2.0]);
        shared.set_feed(Some(a.clone()));
        shared.set_gain(0.5);
        fill(&shared, &mut feed, &mut out, 2, now);
        assert_eq!(out, [0.5, -0.5, 1.0, -1.0, 0.0, 0.0]);
        // the decoder is behind: the silence after the sound is not counted, so the clock waits for it
        assert_eq!(a.counted.load(Ordering::Relaxed), 2);
        // once the sound is over, silence counts: the clock runs on to the picture's end
        a.end();
        fill(&shared, &mut feed, &mut out, 2, now);
        assert_eq!(a.counted.load(Ordering::Relaxed), 5);

        let b = Arc::new(Feed::new(48_000, 2));
        b.ring.push(&[3.0; 6]);
        shared.set_feed(Some(b.clone()));
        shared.set_gain(1.0);
        fill(&shared, &mut feed, &mut out, 2, now);
        assert_eq!(out, [3.0; 6]);
        assert_eq!((a.counted.load(Ordering::Relaxed), b.counted.load(Ordering::Relaxed)), (5, 3));

        shared.set_feed(None);
        fill(&shared, &mut feed, &mut out, 2, now);
        assert_eq!(out, [0.0; 6]);
    }

    #[test]
    fn sound_is_all_heard_when_the_callbacks_after_it_are_starved() {
        let (feed, mut out, t) = (Feed::new(48_000, 2), [0.0; 6], Instant::now());
        feed.ring.push(&[1.0; 4]);
        feed.take(&mut out, 2, t);
        // the decoder is behind: this one gets nothing, and what the device holds ahead of it is the silence after the
        // two frames, not sound still to come
        feed.take(&mut out, 2, t + 10 * MS);
        assert_eq!(feed.heard(t + 5 * MS, 48_000), 2);
    }

    #[test]
    fn the_clock_never_steps_back() {
        let feed = Feed::new(48_000, 2);
        let t = Instant::now();
        *feed.stamp.lock().unwrap() = Stamp { before: 0, given: 480, plays_at: Some(t) };
        assert_eq!(feed.heard(t + 20 * MS, 48_000), 480);
        // a later stamp that puts the speaker behind where it was said to be
        *feed.stamp.lock().unwrap() = Stamp { before: 480, given: 480, plays_at: Some(t + 30 * MS) };
        assert_eq!(feed.heard(t + 21 * MS, 48_000), 480);
    }

    /// needs an output device. plays 200 ms of silence: nothing is heard.
    #[test]
    #[ignore]
    fn the_default_output_plays_and_counts_silence() {
        let shared = Arc::new(Shared::new());
        let opened = Instant::now();
        let output = Output::open(shared.clone()).expect("an output device");
        println!("opened the default output in {:?}: {} Hz, {} channels", opened.elapsed(), output.rate, output.channels);
        let frames = output.rate as usize / 5;
        let feed = Arc::new(Feed::new(output.rate, output.channels));
        assert_eq!(feed.ring.push(&vec![0.0; frames * output.channels as usize]), frames * output.channels as usize);
        shared.set_feed(Some(feed.clone()));
        std::thread::sleep(Duration::from_millis(600));
        assert!(!output.lost());
        // all of it went to the device, and the starved callbacks after it counted nothing
        assert_eq!(feed.counted.load(Ordering::Relaxed), frames as u64);
        assert_eq!(feed.heard(Instant::now(), output.rate), frames as u64);
    }
}
