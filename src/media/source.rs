//! frames on demand, for any kind of clip: a worker keeps the frame on screen and its neighbors ready, the render
//! thread asks for the one it needs. what "loading a frame" means is the lane's own business (see `Loader`).

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// frames kept ready past the one on screen, in the direction of travel
const AHEAD: usize = 4;

pub enum Frame<T> {
    Ready(Arc<T>),
    /// not there yet: ask again
    Pending,
    /// this frame cannot be shown, and why
    Failed(String),
}

/// how one lane produces a frame. it runs on the worker thread and is the only thing that touches the file.
pub trait Loader: Send + 'static {
    type Item: Send + Sync + 'static;
    fn load(&mut self, frame: usize) -> Result<Arc<Self::Item>, String>;
    /// a frame nobody holds any more. the lane may reuse its memory; by default it is simply dropped.
    fn retire(&mut self, _item: Self::Item) {}
}

pub struct Source<T> {
    shared: Arc<Shared<T>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    /// the worker sleeps on this until the wanted frames change
    wake: Condvar,
    /// the render thread waits on this for a frame to arrive
    arrived: Condvar,
}

struct State<T> {
    frame: usize,
    playing: bool,
    ready: Vec<(usize, Arc<T>)>,
    failed: Vec<(usize, String)>,
    quit: bool,
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

/// runs a step that reads an untrusted file. if it stops on something the parser did not foresee, that is this
/// frame's fault to report, not the end of the reader or of the program.
pub fn guarded<T>(step: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)).unwrap_or_else(|_| Err("the reader stopped on this frame".into()))
}

impl<T: Send + Sync + 'static> Source<T> {
    /// starts the worker for a clip of `frames` frames.
    pub fn start(frames: usize, loader: impl Loader<Item = T>) -> Result<Source<T>, String> {
        let shared = Arc::new(Shared { state: Mutex::new(State { frame: 0, playing: false, ready: Vec::new(), failed: Vec::new(), quit: false }), wake: Condvar::new(), arrived: Condvar::new() });
        let theirs = shared.clone();
        let worker = std::thread::Builder::new().name("frames".into()).spawn(move || work(&theirs, frames.max(1), loader)).map_err(|e| e.to_string())?;
        Ok(Source { shared, worker: Some(worker) })
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
    pub fn get(&self, frame: usize, patience: Duration) -> Frame<T> {
        let settled = |s: &State<T>| s.ready.iter().any(|r| r.0 == frame) || s.failed.iter().any(|f| f.0 == frame);
        let s = self.shared.state.lock().unwrap();
        let (s, _) = self.shared.arrived.wait_timeout_while(s, patience, |s| !settled(s)).unwrap();
        if let Some((_, item)) = s.ready.iter().find(|r| r.0 == frame) {
            return Frame::Ready(item.clone());
        }
        match s.failed.iter().find(|f| f.0 == frame) {
            Some((_, why)) => Frame::Failed(why.clone()),
            None => Frame::Pending,
        }
    }
}

impl<T> Drop for Source<T> {
    /// stops the worker and waits for it: the lane and everything it holds (a decoder using the renderer's device) is
    /// gone when this returns.
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().quit = true;
        self.shared.wake.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn work<L: Loader>(shared: &Shared<L::Item>, frames: usize, mut loader: L) {
    loop {
        let (missing, retired) = {
            let mut s = shared.state.lock().unwrap();
            loop {
                if s.quit {
                    return;
                }
                let want = wanted(s.frame, s.playing, frames);
                // let go of what is no longer near. what nobody else still holds goes back to the lane
                let (mut retired, mut i) = (Vec::new(), 0);
                while i < s.ready.len() {
                    if want.contains(&s.ready[i].0) {
                        i += 1;
                    } else if let Ok(item) = Arc::try_unwrap(s.ready.swap_remove(i).1) {
                        retired.push(item);
                    }
                }
                s.failed.retain(|f| want.contains(&f.0));
                let missing = want.into_iter().find(|f| !s.ready.iter().any(|r| r.0 == *f) && !s.failed.iter().any(|r| r.0 == *f));
                if missing.is_some() || !retired.is_empty() {
                    break (missing, retired);
                }
                s = shared.wake.wait(s).unwrap();
            }
        };
        for item in retired {
            loader.retire(item);
        }
        let Some(frame) = missing else { continue };
        let result = guarded(|| loader.load(frame));
        let mut s = shared.state.lock().unwrap();
        match result {
            Ok(item) => s.ready.push((frame, item)),
            Err(why) => s.failed.push((frame, why)),
        }
        shared.arrived.notify_all();
    }
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

    /// a lane whose frames are just their own numbers, and which breaks on frame 3.
    struct Numbers;
    impl Loader for Numbers {
        type Item = usize;
        fn load(&mut self, frame: usize) -> Result<Arc<usize>, String> {
            match frame {
                3 => panic!("a parser bug"),
                4 => Err("damaged".into()),
                n => Ok(Arc::new(n * 10)),
            }
        }
    }

    #[test]
    fn frames_arrive_and_faults_are_reported_not_fatal() {
        let source = Source::start(10, Numbers).unwrap();
        let get = |frame| {
            source.want(frame, false);
            source.get(frame, Duration::from_secs(5))
        };
        assert!(matches!(get(2), Frame::Ready(n) if *n == 20));
        assert!(matches!(get(3), Frame::Failed(why) if why == "the reader stopped on this frame"));
        assert!(matches!(get(4), Frame::Failed(why) if why == "damaged"));
        // the worker is still alive after both
        assert!(matches!(get(9), Frame::Ready(n) if *n == 90));
    }
}
