//! a second reader of the clip on screen, for what the window's own reader cannot do without moving the picture:
//! the frame under the pointer on the timebar, and how much changes across a loop's seam, from the clip's last frame
//! to its first.

use super::make::{self, Reader};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

/// the reader lets its clip go when nothing has been asked of it for this long: it holds a decoder of its own
const IDLE: Duration = Duration::from_secs(5);

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
    /// a small picture of a frame, for the timebar. while a clip is `playing`, a clip whose frames need the ones
    /// before them decoded first is left alone: a long group of them would take the decoder from the picture
    Frame { path: PathBuf, frame: usize, playing: bool },
    /// the clip changed on disk or left the screen: the reader lets go of it
    Forget,
}

pub enum Answer {
    /// None when the clip's frames cannot be read, or it has only one
    Seam(PathBuf, Option<Seam>),
    /// the frame's picture: red, green and blue, a byte each
    Frame { path: PathBuf, frame: usize, width: u32, height: u32, pixels: Vec<u8> },
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

/// the spare reader's work, for a thread of its own. the clip asked about stays open from one asking to the next,
/// until another is asked about, it is forgotten, or nothing is asked for a while. of the frames asked for while it
/// was busy only the newest is read; every seam asked about is answered. `told` is called after each answer.
/// returns when the window's end is gone.
pub fn serve<R: Reader>(asks: Receiver<Ask>, answers: Sender<Answer>, told: impl Fn(), mut open: impl FnMut(&Path) -> Option<R>) {
    let mut held: Option<(PathBuf, R)> = None;
    loop {
        let first = match asks.recv_timeout(IDLE) {
            Ok(ask) => ask,
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                held = None;
                match asks.recv() {
                    Ok(ask) => ask,
                    Err(_) => return,
                }
            }
        };
        let mut waiting = vec![first];
        waiting.extend(asks.try_iter());
        let newest_frame = waiting.iter().rposition(|ask| matches!(ask, Ask::Frame { .. }));
        for (i, ask) in waiting.into_iter().enumerate() {
            let path = match &ask {
                Ask::Forget => {
                    held = None;
                    continue;
                }
                Ask::Frame { .. } if Some(i) != newest_frame => continue,
                Ask::Seam(path) | Ask::Frame { path, .. } => path.clone(),
            };
            if held.as_ref().is_none_or(|(of, _)| of != &path) {
                // the one before goes first: two decoders are not held at once
                held = None;
                held = open(&path).map(|clip| (path.clone(), clip));
            }
            let clip = held.as_mut().map(|(_, clip)| clip);
            let answer = match ask {
                Ask::Seam(_) => Some(Answer::Seam(path, clip.and_then(seam))),
                Ask::Frame { frame, playing, .. } => clip.filter(|clip| !playing || clip.stands_alone()).and_then(|clip| {
                    let frame = frame.min(clip.frames().saturating_sub(1));
                    make::small(clip, frame).map(|((width, height), pixels)| Answer::Frame { path, frame, width, height, pixels })
                }),
                Ask::Forget => None,
            };
            if let Some(answer) = answer {
                if answers.send(answer).is_err() {
                    return;
                }
                told();
            }
        }
    }
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
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    /// a clip whose frames are flat gray, as bright as their number; with `same`, all alike. `long` frames need
    /// the ones before them. it counts the times it is opened.
    struct Flat {
        frames: usize,
        same: bool,
        long: bool,
    }

    impl Reader for Flat {
        fn frames(&self) -> usize {
            self.frames
        }
        fn size(&self) -> (u32, u32) {
            (4, 2)
        }
        fn stands_alone(&self) -> bool {
            !self.long
        }
        fn read(&mut self, frame: usize, w: u32, h: u32) -> Option<Vec<u16>> {
            Some(vec![if self.same { 7 } else { frame as u16 * 6553 }; (w * h * 4) as usize])
        }
    }

    /// a spare reader over made-up clips, and how often it opened one.
    fn spare() -> (Spare, Arc<AtomicUsize>, std::thread::JoinHandle<()>) {
        let (spare, asks, answers) = Spare::new();
        let opened = Arc::new(AtomicUsize::new(0));
        let theirs = opened.clone();
        let worker = std::thread::spawn(move || {
            serve(asks, answers, || {}, |path| {
                theirs.fetch_add(1, Ordering::Relaxed);
                let clip = |frames, same, long| Some(Flat { frames, same, long });
                match path.to_str() {
                    Some("ten") => clip(10, false, false),
                    Some("doubled") => clip(10, true, false),
                    Some("still") => clip(1, false, false),
                    Some("long") => clip(10, false, true),
                    _ => None,
                }
            })
        });
        (spare, opened, worker)
    }

    fn answer(spare: &Spare, ask: Ask) -> Answer {
        spare.ask(ask);
        spare.answers.recv_timeout(Duration::from_secs(20)).expect("an answer")
    }

    #[test]
    fn the_last_frame_is_compared_with_the_first() {
        let (spare, _, worker) = spare();
        let seam = |name: &str| match answer(&spare, Ask::Seam(name.into())) {
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
        let (spare, opened, worker) = spare();
        let frame = |name: &str, frame, playing| answer(&spare, Ask::Frame { path: name.into(), frame, playing });
        for wanted in [3, 7] {
            match frame("ten", wanted, false) {
                Answer::Frame { frame, width, height, pixels, .. } => assert_eq!((frame, width, height, pixels[0]), (wanted, 4, 2, (wanted * 6553 / 257) as u8)),
                Answer::Seam(..) => panic!("a seam for a frame"),
            }
        }
        // one clip, opened once; a frame past the end is the last
        assert_eq!(opened.load(Ordering::Relaxed), 1);
        assert!(matches!(frame("ten", 99, true), Answer::Frame { frame: 9, .. }));
        // while a clip plays, frames that need the ones before them are not read: the next answer is the seam's
        spare.ask(Ask::Frame { path: "long".into(), frame: 5, playing: true });
        assert!(matches!(answer(&spare, Ask::Seam("long".into())), Answer::Seam(..)));
        assert!(matches!(frame("long", 5, false), Answer::Frame { frame: 5, .. }));
        // forgotten, the clip is opened anew, for a frame and for its seam: a file that changed on disk is
        // forgotten before it is asked about again, so no answer is of the file that was
        let before = opened.load(Ordering::Relaxed);
        spare.ask(Ask::Forget);
        assert!(matches!(frame("long", 5, false), Answer::Frame { .. }));
        spare.ask(Ask::Forget);
        assert!(matches!(answer(&spare, Ask::Seam("long".into())), Answer::Seam(..)));
        assert_eq!(opened.load(Ordering::Relaxed), before + 2);
        drop(spare);
        worker.join().unwrap();
    }
}
