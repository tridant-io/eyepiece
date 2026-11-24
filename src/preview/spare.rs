//! a second reader of the clip on screen, for what the window's own reader cannot do without moving the picture.
//! so far one thing: how much changes across a loop's seam, from the clip's last frame to its first.

use super::make::Reader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

/// how the last frame of a clip and its first compare
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seam {
    /// every pixel is the same: a loop shows that frame twice running, which stutters
    pub identical: bool,
    /// the mean change of a pixel's red, green and blue across the seam, as a part of full scale
    pub change: f64,
}

/// the window's end of the spare reader: clips to ask about, and what comes back for each (None when its frames
/// cannot be read, or it has only one)
pub struct Spare {
    asks: Sender<PathBuf>,
    answers: Receiver<(PathBuf, Option<Seam>)>,
}

impl Spare {
    /// the window's end, and the reader's two for `serve`.
    pub fn new() -> (Spare, Receiver<PathBuf>, Sender<(PathBuf, Option<Seam>)>) {
        let ((asks, theirs), (answers, ours)) = (mpsc::channel(), mpsc::channel());
        (Spare { asks, answers: ours }, theirs, answers)
    }

    pub fn ask(&self, path: &Path) {
        let _ = self.asks.send(path.to_path_buf());
    }

    pub fn take(&self) -> Option<(PathBuf, Option<Seam>)> {
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

/// the spare reader's work, for a thread of its own: each clip asked about is opened, and its last frame and its
/// first are read at full size and compared. when several are waiting only the newest is answered. `told` is
/// called after each answer. returns when the window's end is gone.
pub fn serve<R: Reader>(asks: Receiver<PathBuf>, answers: Sender<(PathBuf, Option<Seam>)>, told: impl Fn(), mut open: impl FnMut(&Path) -> Option<R>) {
    while let Ok(mut path) = asks.recv() {
        while let Ok(newer) = asks.try_recv() {
            path = newer;
        }
        let seam = open(&path).and_then(|mut clip| {
            let ((w, h), frames) = (clip.size(), clip.frames());
            let last = clip.read(frames.checked_sub(1).filter(|&last| last > 0)?, w, h)?;
            Some(compare(&last, &clip.read(0, w, h)?))
        });
        if answers.send((path, seam)).is_err() {
            return;
        }
        told();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// a clip whose frames are flat gray, as bright as their number; with `same`, all alike.
    struct Flat(usize, bool);

    impl Reader for Flat {
        fn frames(&self) -> usize {
            self.0
        }
        fn size(&self) -> (u32, u32) {
            (4, 2)
        }
        fn read(&mut self, frame: usize, w: u32, h: u32) -> Option<Vec<u16>> {
            Some(vec![if self.1 { 7 } else { frame as u16 * 6553 }; (w * h * 4) as usize])
        }
    }

    #[test]
    fn the_last_frame_is_compared_with_the_first() {
        let (spare, asks, answers) = Spare::new();
        let worker = std::thread::spawn(move || {
            serve(asks, answers, || {}, |path| match path.to_str() {
                Some("ten") => Some(Flat(10, false)),
                Some("doubled") => Some(Flat(10, true)),
                Some("still") => Some(Flat(1, false)),
                _ => None,
            })
        });
        let mut answer = |name: &str| {
            spare.ask(Path::new(name));
            spare.answers.recv_timeout(std::time::Duration::from_secs(20)).expect("an answer").1
        };
        // frame 9 against frame 0: nine steps of a tenth of full scale each, near enough
        let seam = answer("ten").expect("a seam");
        assert!(!seam.identical && (seam.change - 0.9).abs() < 0.001, "{seam:?}");
        assert_eq!(answer("doubled").map(|s| s.identical), Some(true));
        // a still has no seam, and a file that does not open has no answer
        assert_eq!((answer("still"), answer("missing")), (None, None));
        drop(spare);
        worker.join().unwrap();
    }
}
