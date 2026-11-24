//! frames on demand, for any kind of clip: a worker keeps ready the frames the render thread says it wants (the one
//! on screen, and what comes next), and the render thread asks for the one it needs. what "loading a frame" means is
//! the lane's own business (see `Loader`).

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

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
    /// produces a frame. a lane whose loads can be long asks `abandoned` now and then, and returns Ok(None) when it
    /// says yes: the frame is no longer wanted, and something else is.
    fn load(&mut self, frame: usize, abandoned: &dyn Fn() -> bool) -> Result<Option<Arc<Self::Item>>, String>;
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
    /// the frames to keep ready, most urgent first
    want: Vec<usize>,
    ready: Vec<(usize, Arc<T>)>,
    failed: Vec<(usize, String)>,
    quit: bool,
}

/// runs a step that reads an untrusted file. if it stops on something the parser did not foresee, that is this
/// frame's fault to report, not the end of the reader or of the program.
pub fn guarded<T>(step: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)).unwrap_or_else(|_| Err("the reader stopped on this frame".into()))
}

impl<T: Send + Sync + 'static> Source<T> {
    /// starts the worker for a clip. it keeps frame 0 ready until told otherwise.
    pub fn start(loader: impl Loader<Item = T>) -> Result<Source<T>, String> {
        let shared = Arc::new(Shared { state: Mutex::new(State { want: vec![0], ready: Vec::new(), failed: Vec::new(), quit: false }), wake: Condvar::new(), arrived: Condvar::new() });
        let theirs = shared.clone();
        let worker = std::thread::Builder::new().name("frames".into()).spawn(move || work(&theirs, loader)).map_err(|e| e.to_string())?;
        Ok(Source { shared, worker: Some(worker) })
    }

    /// tells the worker which frames to keep ready, most urgent first: the one on screen, then the ones that come
    /// next (`play::Transport::ahead`). everything else is let go.
    pub fn want(&self, frames: &[usize]) {
        let mut s = self.shared.state.lock().unwrap();
        if s.want != frames {
            s.want = frames.to_vec();
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

fn work<L: Loader>(shared: &Shared<L::Item>, mut loader: L) {
    loop {
        let (missing, retired) = {
            let mut s = shared.state.lock().unwrap();
            loop {
                if s.quit {
                    return;
                }
                let want = s.want.clone();
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
        let abandoned = || {
            let s = shared.state.lock().unwrap();
            s.quit || !s.want.contains(&frame)
        };
        let result = guarded(|| loader.load(frame, &abandoned));
        let mut s = shared.state.lock().unwrap();
        match result {
            Ok(Some(item)) => s.ready.push((frame, item)),
            Ok(None) => {}
            Err(why) => s.failed.push((frame, why)),
        }
        shared.arrived.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
