//! frames on demand, for any kind of clip: workers keep ready the frames the render thread says it wants (the one
//! on screen, and what comes next), and the render thread asks for the one it needs. what "loading a frame" means is
//! the lane's own business (see `Loader`).

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// frames past the one on screen that a lane is asked for, for each of its workers: a lane on one worker gets four,
/// one on eight gets 32, so every worker has a frame to load and each frame is asked for well before it is due
const AHEAD_EACH: usize = 4;
/// the most workers a lane runs
const MOST_WORKERS: usize = 8;

pub enum Frame<T> {
    Ready(Arc<T>),
    /// not there yet: ask again
    Pending,
    /// this frame cannot be shown, and why
    Failed(String),
}

/// how one lane produces a frame. it runs on a worker thread and is the only thing there that touches the file.
pub trait Loader: Send + 'static {
    type Item: Send + Sync + 'static;
    /// produces a frame. a lane whose loads can be long asks `abandoned` now and then, and returns Ok(None) when it
    /// says yes: the frame is no longer wanted, and something else is.
    fn load(&mut self, frame: usize, abandoned: &dyn Fn() -> bool) -> Result<Option<Arc<Self::Item>>, String>;
    /// a frame nobody holds any more. the lane may reuse its memory; by default it is simply dropped.
    fn retire(&mut self, _item: Self::Item) {}
    /// another loader for the same clip, for a lane whose frames stand alone: with one on each worker, several frames
    /// load at once. by default there is none, and the lane loads one frame at a time.
    fn split(&self) -> Option<Self>
    where
        Self: Sized,
    {
        None
    }
}

pub struct Source<T> {
    shared: Arc<Shared<T>>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    /// the workers sleep on this until the wanted frames change
    wake: Condvar,
    /// the render thread waits on this for a frame to arrive
    arrived: Condvar,
}

struct State<T> {
    /// the frames to keep ready, most urgent first
    want: Vec<usize>,
    /// how many of the frames asked for are wanted
    look: usize,
    /// each with the last change of `want` that wanted it
    ready: Vec<(usize, Arc<T>, u64)>,
    /// how many frames may be ready, wanted or not, in the lane's memory budget. wanted frames stay beyond it
    fit: usize,
    failed: Vec<(usize, String)>,
    /// the frames workers are loading
    taken: Vec<usize>,
    /// how many times `want` has changed
    asked: u64,
    quit: bool,
}

/// runs a step that reads an untrusted file. if it stops on something the parser did not foresee, that is this
/// frame's fault to report, not the end of the reader or of the program.
pub fn guarded<T>(step: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)).unwrap_or_else(|_| Err("the reader stopped on this frame".into()))
}

/// the memory a lane may keep frames in: a quarter of the machine's (plan section 06, ram around the playhead).
pub fn budget() -> usize {
    ram() / 4
}

#[cfg(windows)]
fn ram() -> usize {
    // the windows crate has this behind a feature the build does not turn on
    #[link(name = "kernel32")]
    extern "system" {
        fn GetPhysicallyInstalledSystemMemory(kilobytes: *mut u64) -> i32;
    }
    let mut kilobytes = 0;
    if unsafe { GetPhysicallyInstalledSystemMemory(&mut kilobytes) } == 0 {
        return 0;
    }
    (kilobytes * 1024) as usize
}

/// macos comes later: until then nothing is kept but the frames wanted.
#[cfg(not(windows))]
fn ram() -> usize {
    0
}

/// workers for a lane whose frames stand alone: every core but two, which stay free for the render thread and for
/// the rest of the program and the system, so the frame on screen is drawn on time. at most eight: each holds a
/// whole frame in flight (133 MB at 4K in float), and on the dev machine (20 threads) sixteen played 4K exr, png and
/// notchlc no better than eight, where four left png 10 frames of 48 late and notchlc 44 of 120.
fn workers() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).saturating_sub(2).clamp(1, MOST_WORKERS)
}

impl<T: Send + Sync + 'static> Source<T> {
    /// starts one worker for a clip, keeping nothing but the frames wanted. it keeps frame 0 ready until told
    /// otherwise.
    pub fn start(loader: impl Loader<Item = T>) -> Result<Source<T>, String> {
        Source::run(vec![loader], 0, None)
    }

    /// starts workers for a clip whose frames stand alone: as many as `workers()`, while `loader` splits. frames
    /// already loaded stay ready while `budget` bytes hold them, at `frame_bytes` a frame, the least recently wanted
    /// let go first. `opened` is the frame the clip opens on, already loaded, which is kept ready until told otherwise.
    pub fn start_shared<L: Loader<Item = T>>(loader: L, budget: usize, frame_bytes: usize, opened: (usize, Arc<T>)) -> Result<Source<T>, String> {
        let mut loaders = vec![loader];
        while loaders.len() < workers() {
            match loaders[0].split() {
                Some(other) => loaders.push(other),
                None => break,
            }
        }
        Source::run(loaders, budget / frame_bytes.max(1), Some(opened))
    }

    fn run<L: Loader<Item = T>>(loaders: Vec<L>, fit: usize, opened: Option<(usize, Arc<T>)>) -> Result<Source<T>, String> {
        // never more wanted than the budget holds, but always what one worker is asked for
        let look = (1 + AHEAD_EACH * loaders.len()).min(fit.max(1 + AHEAD_EACH));
        let (want, ready) = match opened {
            Some((frame, item)) => (vec![frame], vec![(frame, item, 0)]),
            None => (vec![0], Vec::new()),
        };
        let state = State { want, look, ready, fit, failed: Vec::new(), taken: Vec::new(), asked: 0, quit: false };
        let mut source = Source { shared: Arc::new(Shared { state: Mutex::new(state), wake: Condvar::new(), arrived: Condvar::new() }), workers: Vec::new() };
        for loader in loaders {
            let theirs = source.shared.clone();
            // a worker that does not start stops the ones that did, as the source drops
            source.workers.push(std::thread::Builder::new().name("frames".into()).spawn(move || work(&theirs, loader)).map_err(|e| e.to_string())?);
        }
        Ok(source)
    }

    /// tells the workers which frames to keep ready, most urgent first: the one on screen, then the ones that come
    /// next (`play::Transport::ahead`), as many as the workers can use. the rest may be let go.
    pub fn want(&self, frames: &[usize]) {
        let mut s = self.shared.state.lock().unwrap();
        let frames = &frames[..frames.len().min(s.look)];
        if s.want != frames {
            s.asked += 1;
            let asked = s.asked;
            for r in s.ready.iter_mut().filter(|r| frames.contains(&r.0)) {
                r.2 = asked;
            }
            s.want = frames.to_vec();
            self.shared.wake.notify_all();
        }
    }

    /// the frame if it is ready, waiting up to `patience` for it to arrive.
    pub fn get(&self, frame: usize, patience: Duration) -> Frame<T> {
        let settled = |s: &State<T>| s.ready.iter().any(|r| r.0 == frame) || s.failed.iter().any(|f| f.0 == frame);
        let s = self.shared.state.lock().unwrap();
        let (s, _) = self.shared.arrived.wait_timeout_while(s, patience, |s| !settled(s)).unwrap();
        if let Some((_, item, _)) = s.ready.iter().find(|r| r.0 == frame) {
            return Frame::Ready(item.clone());
        }
        match s.failed.iter().find(|f| f.0 == frame) {
            Some((_, why)) => Frame::Failed(why.clone()),
            None => Frame::Pending,
        }
    }
}

impl<T> Drop for Source<T> {
    /// stops the workers and waits for them: the lane and everything it holds (a decoder using the renderer's device)
    /// is gone when this returns.
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().quit = true;
        self.shared.wake.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn work<L: Loader>(shared: &Shared<L::Item>, mut loader: L) {
    loop {
        let (missing, retired) = {
            let mut s = shared.state.lock().unwrap();
            loop {
                if s.quit {
                    return;
                }
                let want = s.want.clone();
                // let go of what is no longer wanted while more is ready than the budget holds, the least recently
                // wanted first. what nobody else still holds goes back to this worker's lane
                let mut retired = Vec::new();
                while s.ready.len() > s.fit {
                    let Some(oldest) = (0..s.ready.len()).filter(|&i| !want.contains(&s.ready[i].0)).min_by_key(|&i| s.ready[i].2) else { break };
                    if let Ok(item) = Arc::try_unwrap(s.ready.swap_remove(oldest).1) {
                        retired.push(item);
                    }
                }
                s.failed.retain(|f| want.contains(&f.0));
                // the most urgent frame not ready, not failed, and not being loaded by another worker
                let missing = want.into_iter().find(|f| !s.ready.iter().any(|r| r.0 == *f) && !s.failed.iter().any(|r| r.0 == *f) && !s.taken.contains(f));
                if let Some(frame) = missing {
                    s.taken.push(frame);
                }
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
        let abandoned = || {
            let s = shared.state.lock().unwrap();
            s.quit || !s.want.contains(&frame)
        };
        let result = guarded(|| loader.load(frame, &abandoned));
        let mut s = shared.state.lock().unwrap();
        s.taken.retain(|&t| t != frame);
        let asked = s.asked;
        match result {
            Ok(Some(item)) => s.ready.push((frame, item, asked)),
            Ok(None) => {}
            Err(why) => s.failed.push((frame, why)),
        }
        shared.arrived.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// a lane whose frames are just their own numbers, and which breaks on frame 3.
    struct Numbers;
    impl Loader for Numbers {
        type Item = usize;
        fn load(&mut self, frame: usize, _abandoned: &dyn Fn() -> bool) -> Result<Option<Arc<usize>>, String> {
            match frame {
                3 => panic!("a parser bug"),
                4 => Err("damaged".into()),
                n => Ok(Some(Arc::new(n * 10))),
            }
        }
    }

    #[test]
    fn frames_arrive_and_faults_are_reported_not_fatal() {
        let source = Source::start(Numbers).unwrap();
        let get = |frame| {
            source.want(&[frame]);
            source.get(frame, Duration::from_secs(5))
        };
        assert!(matches!(get(2), Frame::Ready(n) if *n == 20));
        assert!(matches!(get(3), Frame::Failed(why) if why == "the reader stopped on this frame"));
        assert!(matches!(get(4), Frame::Failed(why) if why == "damaged"));
        // the worker is still alive after both
        assert!(matches!(get(9), Frame::Ready(n) if *n == 90));
    }

    #[derive(Default)]
    struct Counts {
        loads: AtomicUsize,
        retired: AtomicUsize,
        loading: AtomicUsize,
        most_at_once: AtomicUsize,
    }

    /// a lane that splits, whose loads take a while, which counts its loads and the frames handed back to it, and
    /// which breaks on frame 9.
    struct Slow(Arc<Counts>);
    impl Loader for Slow {
        type Item = usize;
        fn load(&mut self, frame: usize, _abandoned: &dyn Fn() -> bool) -> Result<Option<Arc<usize>>, String> {
            let c = &self.0;
            c.loads.fetch_add(1, Ordering::SeqCst);
            c.most_at_once.fetch_max(c.loading.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            c.loading.fetch_sub(1, Ordering::SeqCst);
            match frame {
                9 => panic!("a parser bug"),
                n => Ok(Some(Arc::new(n))),
            }
        }
        fn retire(&mut self, _item: usize) {
            self.0.retired.fetch_add(1, Ordering::SeqCst);
        }
        fn split(&self) -> Option<Slow> {
            Some(Slow(self.0.clone()))
        }
    }

    fn ready(source: &Source<usize>, frame: usize) -> bool {
        matches!(source.get(frame, Duration::from_secs(10)), Frame::Ready(n) if *n == frame)
    }

    fn count(n: &AtomicUsize) -> usize {
        n.load(Ordering::SeqCst)
    }

    #[test]
    fn workers_share_the_frames_and_keep_what_the_budget_holds() {
        let counts = Arc::new(Counts::default());
        // room for six frames of one byte each, and the opening frame already there
        let source = Source::start_shared(Slow(counts.clone()), 6, 1, (0, Arc::new(0))).unwrap();
        assert!(ready(&source, 0));
        source.want(&[1, 2, 3, 4]);
        assert!((1..=4).all(|f| ready(&source, f)));
        // each frame once and the opening one never, side by side when there is more than one worker
        assert_eq!(count(&counts.loads), 4);
        assert_eq!(count(&counts.most_at_once) > 1, workers() > 1);
        // frames no longer wanted stay ready while the budget holds them, in the order they were last wanted
        for f in [2, 1, 3, 4] {
            source.want(&[f]);
            assert!(ready(&source, f));
        }
        assert_eq!(count(&counts.loads), 4);
        // past the budget the least recently wanted go back to the lane: 0, then 2
        source.want(&[5, 6, 7]);
        assert!((5..=7).all(|f| ready(&source, f)));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(count(&counts.retired), 2);
        let mut kept: Vec<usize> = source.shared.state.lock().unwrap().ready.iter().map(|r| r.0).collect();
        kept.sort();
        assert_eq!(kept, [1, 3, 4, 5, 6, 7]);
        // a panic is that frame's fault, and the others still arrive
        source.want(&[9, 8]);
        assert!(matches!(source.get(9, Duration::from_secs(10)), Frame::Failed(why) if why == "the reader stopped on this frame"));
        assert!(ready(&source, 8));
        // dropping it joins every worker, and with them goes every loader
        drop(source);
        assert_eq!(Arc::strong_count(&counts), 1);
    }

    #[test]
    fn with_no_budget_only_the_frames_wanted_are_kept() {
        let counts = Arc::new(Counts::default());
        let source = Source::start_shared(Slow(counts.clone()), 0, 1, (0, Arc::new(0))).unwrap();
        // and as many asked for as one worker is
        assert_eq!(source.shared.state.lock().unwrap().look, 1 + AHEAD_EACH);
        source.want(&[1, 2]);
        assert!(ready(&source, 1) && ready(&source, 2));
        source.want(&[2]);
        // a worker lets 1 go when it next looks, which is at once
        std::thread::sleep(Duration::from_millis(100));
        let kept: Vec<usize> = source.shared.state.lock().unwrap().ready.iter().map(|r| r.0).collect();
        assert_eq!((kept, count(&counts.retired)), (vec![2], 2));
        source.want(&[1]);
        assert!(ready(&source, 1));
        assert_eq!(count(&counts.loads), 3);
    }
}
