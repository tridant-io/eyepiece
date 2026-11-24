//! a fixed ring of samples from one writer (the decode thread) to one reader (the device's callback), with no lock:
//! the callback must never wait.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

pub struct Ring {
    /// each sample's bits. relaxed atomics are plain loads and stores; the counters carry the ordering
    slots: Box<[AtomicU32]>,
    /// samples written and read since the start. only the writer moves `written`, only the reader `read`
    written: AtomicUsize,
    read: AtomicUsize,
}

impl Ring {
    pub fn new(capacity: usize) -> Ring {
        Ring { slots: (0..capacity.max(1)).map(|_| AtomicU32::new(0)).collect(), written: AtomicUsize::new(0), read: AtomicUsize::new(0) }
    }

    /// writes as much of `from` as there is room for, and says how much that was. the writer's side.
    pub fn push(&self, from: &[f32]) -> usize {
        let w = self.written.load(Ordering::Relaxed);
        let room = self.slots.len() - (w - self.read.load(Ordering::Acquire));
        let n = from.len().min(room);
        for (i, s) in from[..n].iter().enumerate() {
            self.slots[(w + i) % self.slots.len()].store(s.to_bits(), Ordering::Relaxed);
        }
        self.written.store(w + n, Ordering::Release);
        n
    }

    /// reads into `to` as much as there is, and says how much that was. the reader's side.
    pub fn pop(&self, to: &mut [f32]) -> usize {
        let r = self.read.load(Ordering::Relaxed);
        let n = to.len().min(self.written.load(Ordering::Acquire) - r);
        for (i, s) in to[..n].iter_mut().enumerate() {
            *s = f32::from_bits(self.slots[(r + i) % self.slots.len()].load(Ordering::Relaxed));
        }
        self.read.store(r + n, Ordering::Release);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_empties_and_goes_round() {
        let ring = Ring::new(4);
        let mut out = [0.0; 4];
        assert_eq!(ring.pop(&mut out), 0);
        assert_eq!(ring.push(&[1.0, 2.0, 3.0]), 3);
        // full: only one more fits
        assert_eq!(ring.push(&[4.0, 5.0]), 1);
        assert_eq!(ring.pop(&mut out[..2]), 2);
        assert_eq!(out[..2], [1.0, 2.0]);
        // these wrap past the end of the slots
        assert_eq!(ring.push(&[5.0, 6.0, 7.0]), 2);
        assert_eq!(ring.pop(&mut out), 4);
        assert_eq!(out, [3.0, 4.0, 5.0, 6.0]);
        assert_eq!(ring.pop(&mut out), 0);
    }

    #[test]
    fn a_writer_and_a_reader_on_their_own_threads_lose_nothing() {
        let ring = std::sync::Arc::new(Ring::new(64));
        let theirs = ring.clone();
        let writer = std::thread::spawn(move || {
            let all: Vec<f32> = (0..100_000).map(|i| i as f32).collect();
            let mut sent = 0;
            while sent < all.len() {
                sent += theirs.push(&all[sent..(sent + 50).min(all.len())]);
            }
        });
        let (mut got, mut buf) = (Vec::new(), [0.0; 37]);
        while got.len() < 100_000 {
            let n = ring.pop(&mut buf);
            got.extend_from_slice(&buf[..n]);
        }
        writer.join().unwrap();
        assert!(got.iter().enumerate().all(|(i, &s)| s == i as f32));
    }
}
