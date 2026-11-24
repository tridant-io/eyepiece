//! a second reader of the clip on screen, for what the window's own reader cannot do without moving the picture:
//! the timebar's pictures, and how much changes across a loop's seam, from the clip's last frame to its first.
//! the timebar's pictures are the design's hybrid (plan section 06): the timebar strip, frames of the clip made
//! behind everything else, the nearest of which the timebar shows at once, then the frame under the pointer; or, for a clip
//! whose frames need the ones before them, the key frame nearest it, never a frame that costs its whole group.

use super::make::{self, Reader};
use crate::browse::{self, Volume};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// the reader lets its clip go when nothing has been asked of it or made for this long: it holds a decoder of its own
const IDLE: Duration = Duration::from_secs(5);
/// how often a strip that waits for play to stop looks again
const LOOK: Duration = Duration::from_millis(100);
/// a strip has a frame every so many seconds, and no more than so many frames
const STRIP_EVERY: f64 = 2.0;
const STRIP_MOST: usize = 100;

/// how the last frame of a clip and its first compare
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seam {
    /// every pixel is the same: a loop shows that frame twice running, which stutters
    pub identical: bool,
    /// the mean change of a pixel's red, green and blue across the seam, as a part of full scale
    pub change: f64,
}

pub enum Ask {
    /// how the clip's last frame and its first compare
    Seam(PathBuf),
    /// a small picture of a frame, for the timebar: of a clip whose frames need the ones before them, the key frame
    /// nearest it. while a clip is `playing`, such a clip is left alone: its decoder would take time from the picture
    Frame { path: PathBuf, frame: usize, playing: bool },
    /// the clip that is up: its timebar strip is made, behind everything else. none is made on a network volume (adr 0004)
    Strip(PathBuf),
    /// the clip changed on disk or left the screen: the reader lets go of it, and of its strip
    Forget,
}

pub enum Answer {
    /// None when the clip's frames cannot be read, or it has only one
    Seam(PathBuf, Option<Seam>),
    /// a picture for the timebar, of `frame`: red, green and blue, a byte each. one of the clip's `strip`, or the
    /// picture asked for last
    Frame { path: PathBuf, frame: usize, width: u32, height: u32, pixels: Vec<u8>, strip: bool },
}

/// the window's end of the spare reader
pub struct Spare {
    asks: Sender<Ask>,
    answers: Receiver<Answer>,
}

impl Spare {
    /// the window's end, and the reader's two for `serve`.
    pub fn new() -> (Spare, Receiver<Ask>, Sender<Answer>) {
        let ((asks, theirs), (answers, ours)) = (mpsc::channel(), mpsc::channel());
        (Spare { asks, answers: ours }, theirs, answers)
    }

    pub fn ask(&self, ask: Ask) {
        let _ = self.asks.send(ask);
    }

    pub fn take(&self) -> Option<Answer> {
        self.answers.try_recv().ok()
    }
}

/// compares two pictures of one size, four 16-bit numbers a pixel. the alpha is left out: it is not shown.
pub fn compare(last: &[u16], first: &[u16]) -> Seam {
    let (mut sum, mut count) = (0u64, 0u64);
    for (a, b) in last.chunks_exact(4).zip(first.chunks_exact(4)) {
        sum += (0..3).map(|c| a[c].abs_diff(b[c]) as u64).sum::<u64>();
        count += 3;
    }
    Seam { identical: sum == 0 && count > 0, change: sum as f64 / (count.max(1) as f64 * 65535.0) }
}

/// the frames of a clip's strip, in the order they are made: one every `STRIP_EVERY` seconds from the first, at most
/// `STRIP_MOST` spread over the clip, and for a clip with `keys` the key frame nearest each. coarse first, as a card's
/// previews are made, so the timebar has a picture near wherever it is hovered while the strip fills in.
fn strip_frames(frames: usize, fps: f64, keys: Option<&[usize]>) -> Vec<usize> {
    let step = (STRIP_EVERY * fps).max(frames as f64 / STRIP_MOST as f64).max(1.0);
    let count = ((frames as f64 / step).ceil() as usize).clamp(1, STRIP_MOST);
    let mut at: Vec<usize> = (0..count).map(|i| ((i as f64 * step).round() as usize).min(frames.saturating_sub(1))).collect();
    if let Some(keys) = keys {
        at = at.into_iter().filter_map(|frame| nearest(keys, frame)).collect();
    }
    at.dedup();
    if at.is_empty() {
        return at;
    }
    make::order(at.len()).into_iter().map(|i| at[i]).collect()
}

/// the one of `frames` (in order) nearest `frame`; the earlier of two as near.
fn nearest(frames: &[usize], frame: usize) -> Option<usize> {
    let after = frames.partition_point(|&f| f < frame);
    match (after.checked_sub(1).map(|i| frames[i]), frames.get(after).copied()) {
        (Some(before), Some(after)) => Some(if frame - before <= after - frame { before } else { after }),
        (before, after) => before.or(after),
    }
}

/// the timebar strip of the clip that is up: the frames still to make, in order, once the clip has been opened for it; and the
/// frames made, which the window has
struct Strip {
    path: PathBuf,
    todo: Option<Vec<usize>>,
    made: Vec<usize>,
}

impl Strip {
    fn unmade(&self) -> bool {
        self.todo.as_ref().is_none_or(|todo| !todo.is_empty())
    }
}

/// the spare reader's work, for a thread of its own. the clip asked about stays open from one asking to the next,
/// until another is asked about, it is forgotten, or nothing is asked or made for a while. of the frames asked for
/// while it was busy only the newest is read; every seam asked about is answered. the strip is made a frame at a
/// time while nothing is asked and `busy` (a clip plays) is not set, and a picture the window already has (one of
/// the strip, or the one answered last) is not read again. `told` is called after each answer. returns when the
/// window's end is gone.
pub fn serve<R: Reader>(asks: Receiver<Ask>, answers: Sender<Answer>, busy: &AtomicBool, told: impl Fn(), mut open: impl FnMut(&Path) -> Option<R>) {
    let mut held: Option<(PathBuf, R)> = None;
    let mut strip: Option<Strip> = None;
    // the picture answered last, and when anything was last asked or made
    let mut answered: Option<(PathBuf, usize)> = None;
    let mut since = Instant::now();
    let send = |answer: Answer| {
        let sent = answers.send(answer).is_ok();
        if sent {
            told();
        }
        sent
    };
    loop {
        let making = strip.as_ref().is_some_and(Strip::unmade);
        let first = if making || held.is_some() {
            asks.recv_timeout(match (making, busy.load(Ordering::Relaxed)) {
                (true, false) => Duration::ZERO,
                (true, true) => LOOK,
                (false, _) => IDLE,
            })
        } else {
            asks.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        let first = match first {
            Ok(ask) => ask,
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) if making && !busy.load(Ordering::Relaxed) => {
                // one frame of the strip
                since = Instant::now();
                let Some(s) = strip.as_mut() else { continue };
                let Some(clip) = hold(&mut held, &s.path, &mut open) else {
                    strip = None;
                    continue;
                };
                let todo = s.todo.get_or_insert_with(|| strip_frames(clip.frames(), clip.rate(), clip.keys().as_deref()));
                if todo.is_empty() {
                    continue;
                }
                let frame = todo.remove(0);
                if let Some((at, (width, height), pixels)) = picture(clip, frame).filter(|(at, ..)| !s.made.contains(at)) {
                    s.made.push(at);
                    if !send(Answer::Frame { path: s.path.clone(), frame: at, width, height, pixels, strip: true }) {
                        return;
                    }
                }
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {
                if since.elapsed() >= IDLE {
                    held = None;
                }
                continue;
            }
        };
        since = Instant::now();
        let mut waiting = vec![first];
        waiting.extend(asks.try_iter());
        let newest_frame = waiting.iter().rposition(|ask| matches!(ask, Ask::Frame { .. }));
        for (i, ask) in waiting.into_iter().enumerate() {
            match ask {
                Ask::Forget => (held, strip, answered) = (None, None, None),
                Ask::Strip(path) => strip = (browse::volume_of(&path) != Volume::Network).then(|| Strip { path, todo: None, made: Vec::new() }),
                Ask::Seam(path) => {
                    let found = hold(&mut held, &path, &mut open).and_then(seam);
                    if !send(Answer::Seam(path, found)) {
                        return;
                    }
                }
                Ask::Frame { path, frame, playing } if Some(i) == newest_frame => {
                    let Some(clip) = hold(&mut held, &path, &mut open) else { continue };
                    let keys = clip.keys();
                    if playing && keys.is_some() {
                        continue;
                    }
                    let frame = frame.min(clip.frames().saturating_sub(1));
                    let Some(want) = keys.map_or(Some(frame), |keys| nearest(&keys, frame)) else { continue };
                    let had = strip.as_ref().is_some_and(|s| s.path == path && s.made.contains(&want)) || answered.as_ref().is_some_and(|(of, at)| *of == path && *at == want);
                    if had {
                        continue;
                    }
                    let Some((at, (width, height), pixels)) = picture(clip, want) else { continue };
                    answered = Some((path.clone(), at));
                    if !send(Answer::Frame { path, frame: at, width, height, pixels, strip: false }) {
                        return;
                    }
                }
                Ask::Frame { .. } => {}
            }
        }
    }
}

/// the clip at `path`, opened unless it is the one held. the one held before goes first: two decoders are not held
/// at once.
fn hold<'a, R: Reader>(held: &'a mut Option<(PathBuf, R)>, path: &Path, open: &mut impl FnMut(&Path) -> Option<R>) -> Option<&'a mut R> {
    if held.as_ref().is_none_or(|(of, _)| of != path) {
        *held = None;
        *held = open(path).map(|clip| (path.to_path_buf(), clip));
    }
    held.as_mut().map(|(_, clip)| clip)
}

/// a frame as a small picture, with the frame it turned out to be: a key frame asked for at its place in the file is
/// shown a few frames later in an open group, which the reader learns as it reads it.
fn picture<R: Reader>(clip: &mut R, frame: usize) -> Option<(usize, (u32, u32), Vec<u8>)> {
    let place = clip.keys().and_then(|keys| keys.iter().position(|&k| k == frame));
    let (size, pixels) = make::small(clip, frame)?;
    let at = place.and_then(|i| clip.keys()?.get(i).copied()).unwrap_or(frame);
    Some((at, size, pixels))
}

/// a clip's last frame against its first, at full size.
fn seam<R: Reader>(clip: &mut R) -> Option<Seam> {
    let ((w, h), frames) = (clip.size(), clip.frames());
    let last = clip.read(frames.checked_sub(1).filter(|&last| last > 0)?, w, h)?;
    Some(compare(&last, &clip.read(0, w, h)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    #[test]
    fn a_seam_is_measured_by_what_is_shown() {
        let black = [0u16, 0, 0, 65535];
        let white = [65535u16; 4];
        // two pixels: one stays black, one goes from black to white
        let seam = compare(&[black, black].concat(), &[black, white].concat());
        assert_eq!((seam.identical, seam.change), (false, 0.5));
        // the same picture: a doubled frame. a change in the alpha alone is not one
        assert_eq!(compare(&[black, white].concat(), &[black, white].concat()), Seam { identical: true, change: 0.0 });
        assert!(compare(&[0, 0, 0, 65535], &[0, 0, 0, 0]).identical);
        // nothing to compare is not a doubled frame
        assert!(!compare(&[], &[]).identical);
    }

    #[test]
    fn a_strip_is_a_frame_every_two_seconds_at_most_a_hundred_and_made_coarse_first() {
        // 10 s at 60: the middle, the ends, the quarters
        assert_eq!(strip_frames(600, 60.0, None), [240, 0, 480, 120, 360]);
        // a key frame every 250: the one nearest each, once
        assert_eq!(strip_frames(600, 60.0, Some(&[0, 250, 500])), [250, 0, 500]);
        // 27 minutes: a hundred, evenly spread
        let mut long = strip_frames(100_000, 60.0, None);
        long.sort_unstable();
        assert_eq!((long.len(), long[1], long[99]), (100, 1000, 99_000));
        // a clip shorter than two seconds, and one that says no rate
        assert_eq!((strip_frames(24, 30.0, None), strip_frames(5, 0.0, None)), (vec![0], vec![2, 0, 4, 1, 3]));
        assert_eq!((nearest(&[0, 4, 8], 2), nearest(&[0, 4, 8], 3), nearest(&[0, 4, 8], 99), nearest(&[], 3)), (Some(0), Some(4), Some(8), None));
    }

    /// what a reader of made-up clips has done, and whether a clip plays
    #[derive(Default)]
    struct Counts {
        opened: AtomicUsize,
        reads: AtomicUsize,
        busy: AtomicBool,
    }

    /// a clip at a frame a second whose frames are flat gray, as bright as their number; with `same`, all alike.
    /// `long` frames need the ones before them, with a key frame every 4. it counts its reads.
    struct Flat {
        frames: usize,
        same: bool,
        long: bool,
        counts: Arc<Counts>,
    }

    impl Reader for Flat {
        fn frames(&self) -> usize {
            self.frames
        }
        fn size(&self) -> (u32, u32) {
            (4, 2)
        }
        fn rate(&self) -> f64 {
            1.0
        }
        fn keys(&self) -> Option<Vec<usize>> {
            self.long.then(|| (0..self.frames).step_by(4).collect())
        }
        fn read(&mut self, frame: usize, w: u32, h: u32) -> Option<Vec<u16>> {
            self.counts.reads.fetch_add(1, Ordering::Relaxed);
            Some(vec![if self.same { 7 } else { frame as u16 * 6553 }; (w * h * 4) as usize])
        }
    }

    /// a spare reader over made-up clips, by name, and what it has done. a clip named "shared" is on a share.
    fn spare() -> (Spare, Arc<Counts>, std::thread::JoinHandle<()>) {
        let (spare, asks, answers) = Spare::new();
        let counts = Arc::new(Counts::default());
        let theirs = counts.clone();
        let worker = std::thread::spawn(move || {
            serve(asks, answers, &theirs.busy, || {}, |path| {
                theirs.opened.fetch_add(1, Ordering::Relaxed);
                let clip = |frames, same, long| Some(Flat { frames, same, long, counts: theirs.clone() });
                match path.file_name()?.to_str()? {
                    "ten" | "shared" => clip(10, false, false),
                    "doubled" => clip(10, true, false),
                    "still" => clip(1, false, false),
                    "long" => clip(10, false, true),
                    _ => None,
                }
            })
        });
        (spare, counts, worker)
    }

    /// where a made-up clip is: in the temp folder, on a local disk (a bare name would count as on a network
    /// volume). nothing there is opened
    fn at(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    fn answer(spare: &Spare, ask: Ask) -> Answer {
        spare.ask(ask);
        next(spare)
    }

    fn next(spare: &Spare) -> Answer {
        spare.answers.recv_timeout(Duration::from_secs(20)).expect("an answer")
    }

    fn count(n: &AtomicUsize) -> usize {
        n.load(Ordering::Relaxed)
    }

    #[test]
    fn the_last_frame_is_compared_with_the_first() {
        let (spare, _, worker) = spare();
        let seam = |name: &str| match answer(&spare, Ask::Seam(at(name))) {
            Answer::Seam(_, seam) => seam,
            Answer::Frame { .. } => panic!("a frame for a seam"),
        };
        // frame 9 against frame 0: nine steps of a tenth of full scale each, near enough
        let ten = seam("ten").expect("a seam");
        assert!(!ten.identical && (ten.change - 0.9).abs() < 0.001, "{ten:?}");
        assert_eq!(seam("doubled").map(|s| s.identical), Some(true));
        // a still has no seam, and a file that does not open has no answer
        assert_eq!((seam("still"), seam("missing")), (None, None));
        drop(spare);
        worker.join().unwrap();
    }

    #[test]
    fn a_frame_for_the_timebar_is_read_small_from_a_clip_that_stays_open() {
        let (spare, counts, worker) = spare();
        let frame = |name: &str, frame, playing| answer(&spare, Ask::Frame { path: at(name), frame, playing });
        for wanted in [3, 7] {
            match frame("ten", wanted, false) {
                Answer::Frame { frame, width, height, pixels, strip, .. } => assert_eq!((frame, width, height, pixels[0], strip), (wanted, 4, 2, (wanted * 6553 / 257) as u8, false)),
                Answer::Seam(..) => panic!("a seam for a frame"),
            }
        }
        // one clip, opened once; a frame past the end is the last
        assert_eq!(count(&counts.opened), 1);
        assert!(matches!(frame("ten", 99, true), Answer::Frame { frame: 9, .. }));
        // while a clip plays, frames that need the ones before them are not read: the next answer is the seam's
        spare.ask(Ask::Frame { path: at("long"), frame: 5, playing: true });
        assert!(matches!(answer(&spare, Ask::Seam(at("long"))), Answer::Seam(..)));
        // standing still, such a clip gives the key frame nearest the frame asked for
        assert!(matches!(frame("long", 5, false), Answer::Frame { frame: 4, .. }));
        // forgotten, the clip is opened anew, for a frame and for its seam: a file that changed on disk is
        // forgotten before it is asked about again, so no answer is of the file that was
        let before = count(&counts.opened);
        spare.ask(Ask::Forget);
        assert!(matches!(frame("long", 5, false), Answer::Frame { .. }));
        spare.ask(Ask::Forget);
        assert!(matches!(answer(&spare, Ask::Seam(at("long"))), Answer::Seam(..)));
        assert_eq!(count(&counts.opened), before + 2);
        drop(spare);
        worker.join().unwrap();
    }

    /// the strips of a clip at a frame a second (a frame every two) and of one with a key frame every 4: nothing while
    /// a clip plays, then made coarse first; and a picture the window has is not read again.
    #[test]
    fn the_strip_waits_while_a_clip_plays_and_what_it_holds_is_not_read_again() {
        let (spare, counts, worker) = spare();
        counts.busy.store(true, Ordering::Relaxed);
        spare.ask(Ask::Strip(at("ten")));
        std::thread::sleep(Duration::from_millis(300));
        assert!(spare.take().is_none() && count(&counts.opened) == 0, "a strip was made while a clip played");
        counts.busy.store(false, Ordering::Relaxed);
        let made: Vec<(usize, bool)> = (0..5)
            .map(|_| match next(&spare) {
                Answer::Frame { frame, strip, .. } => (frame, strip),
                Answer::Seam(..) => panic!("a seam for a strip"),
            })
            .collect();
        assert_eq!(made, [(4, true), (0, true), (8, true), (2, true), (6, true)]);
        assert_eq!((count(&counts.opened), count(&counts.reads)), (1, 5));
        // a frame of the strip is had already; one that is not is read, once, however often it is asked for
        spare.ask(Ask::Frame { path: at("ten"), frame: 6, playing: false });
        std::thread::sleep(Duration::from_millis(200));
        assert!(matches!(answer(&spare, Ask::Frame { path: at("ten"), frame: 5, playing: false }), Answer::Frame { frame: 5, strip: false, .. }));
        spare.ask(Ask::Frame { path: at("ten"), frame: 5, playing: false });
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(count(&counts.reads), 6);
        // a clip with long groups: the key frame nearest each two seconds, and the nearest key frame had already
        spare.ask(Ask::Forget);
        spare.ask(Ask::Strip(at("long")));
        let made: Vec<usize> = (0..3)
            .map(|_| match next(&spare) {
                Answer::Frame { frame, .. } => frame,
                Answer::Seam(..) => panic!("a seam for a strip"),
            })
            .collect();
        assert_eq!(made, [4, 0, 8]);
        spare.ask(Ask::Frame { path: at("long"), frame: 5, playing: false });
        std::thread::sleep(Duration::from_millis(200));
        assert!(spare.take().is_none());
        drop(spare);
        worker.join().unwrap();
    }

    /// on a network volume nothing more is read than before the strip: asked for a strip there, the reader opens
    /// nothing; a picture asked for by hovering is read as it always was, one open and one read. the path stands for
    /// a scratch folder on this pc's own share and is never touched.
    #[test]
    fn nothing_more_is_read_on_a_network_volume() {
        let (spare, counts, worker) = spare();
        let shared = PathBuf::from(r"\\localhost\C$\eyepiece-scratch\shared");
        spare.ask(Ask::Strip(shared.clone()));
        std::thread::sleep(Duration::from_millis(300));
        assert!(spare.take().is_none());
        assert_eq!((count(&counts.opened), count(&counts.reads)), (0, 0));
        assert!(matches!(answer(&spare, Ask::Frame { path: shared, frame: 3, playing: false }), Answer::Frame { frame: 3, strip: false, .. }));
        assert_eq!((count(&counts.opened), count(&counts.reads)), (1, 1));
        drop(spare);
        worker.join().unwrap();
    }
}
