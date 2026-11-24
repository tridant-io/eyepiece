//! answers worked out off the drawing thread: folder listings and a file's metadata. one thread a request, so a
//! slow share holds up only its own. each slot (a folder, a file) keeps only its newest request: an answer to an
//! older one that comes in later is dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

pub struct Jobs<V> {
    count: u64,
    /// the newest request still waiting in each slot
    newest: HashMap<PathBuf, u64>,
    send: Sender<(PathBuf, u64, V)>,
    done: Receiver<(PathBuf, u64, V)>,
}

impl<V: Send + 'static> Jobs<V> {
    pub fn new() -> Jobs<V> {
        let (send, done) = mpsc::channel();
        Jobs { count: 0, newest: HashMap::new(), send, done }
    }

    /// works out `work` for `slot` on a thread of its own.
    pub fn ask(&mut self, slot: PathBuf, work: fn(&Path) -> V) {
        let id = self.number(slot.clone());
        let send = self.send.clone();
        std::thread::spawn(move || {
            let answer = work(&slot);
            let _ = send.send((slot, id, answer));
        });
    }

    fn number(&mut self, slot: PathBuf) -> u64 {
        self.count += 1;
        self.newest.insert(slot, self.count);
        self.count
    }

    /// the answer to the newest request in this slot, and none since it was taken: an older one is stale
    fn fresh(&mut self, slot: &Path, id: u64) -> bool {
        let fresh = self.newest.get(slot) == Some(&id);
        if fresh {
            self.newest.remove(slot);
        }
        fresh
    }

    pub fn waiting(&self, slot: &Path) -> bool {
        self.newest.contains_key(slot)
    }

    pub fn busy(&self) -> bool {
        !self.newest.is_empty()
    }

    /// the slots no longer wanted: their answers will be dropped
    pub fn keep_only(&mut self, wanted: impl Fn(&Path) -> bool) {
        self.newest.retain(|slot, _| wanted(slot));
    }

    /// the fresh answers that have come in since the last look.
    pub fn answers(&mut self) -> Vec<(PathBuf, V)> {
        let came: Vec<_> = self.done.try_iter().collect();
        came.into_iter().filter(|(slot, id, _)| self.fresh(slot, *id)).map(|(slot, _, answer)| (slot, answer)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_request_in_a_slot_wins() {
        let mut jobs = Jobs::<()>::new();
        let (a, b) = (PathBuf::from("a"), PathBuf::from("b"));
        let old = jobs.number(a.clone());
        let new = jobs.number(a.clone());
        let other = jobs.number(b.clone());
        // the older request to the same folder answers late: dropped, and the slot still waits for the newer one
        assert!(!jobs.fresh(&a, old));
        assert!(jobs.waiting(&a));
        assert!(jobs.fresh(&a, new));
        assert!(!jobs.waiting(&a) && jobs.busy());
        // taken once only
        assert!(!jobs.fresh(&a, new));
        // a slot let go of drops its answer when it comes
        jobs.keep_only(|slot| slot != b);
        assert!(!jobs.fresh(&b, other));
        assert!(!jobs.busy());
    }

    #[test]
    fn answers_come_back_from_their_threads() {
        let mut jobs = Jobs::new();
        jobs.ask(PathBuf::from("x"), |p| p.to_string_lossy().len());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut got = Vec::new();
        while got.is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            got = jobs.answers();
        }
        assert_eq!(got, [(PathBuf::from("x"), 1)]);
        assert!(!jobs.busy());
    }
}
