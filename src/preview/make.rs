//! making a clip's poster and hover previews: which frames, how small, in what order, and the thread's work of
//! making them one clip at a time, behind everything else. the frames themselves come from a `Reader`, which the
//! platform's renderer provides, so the pictures go through the same decoders and shaders as the window's.

use super::{Cache, Previews};
use crate::browse::{self, Volume};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, SystemTime};

/// how many pictures a clip gets, and the box they fit in
const COUNT: usize = 24;
const BOX: (u32, u32) = (320, 180);
/// a frame is drawn at up to this many times the picture's size each way and averaged down
const OVER: u32 = 4;

/// a clip opened for its pictures
pub trait Reader {
    /// how many frames it has, and its size in pixels
    fn frames(&self) -> usize;
    fn size(&self) -> (u32, u32);
    /// the frame drawn `w` x `h`: four 16-bit numbers a pixel (red, green, blue, alpha), row by row. None when it
    /// cannot be had
    fn read(&mut self, frame: usize, w: u32, h: u32) -> Option<Vec<u16>>;
}

/// pictures wanted of one file: its poster, or all of its previews. the size and time are the folder listing's
#[derive(Clone, PartialEq)]
pub struct Want {
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
    pub all: bool,
}

/// pictures of a file, as they are found in the cache or made
pub struct Got {
    /// what was asked
    pub want: Want,
    pub clip_frames: usize,
    pub width: u32,
    pub height: u32,
    /// the frame each is of, and its pixels: red, green and blue, a byte each
    pub pictures: Vec<(usize, Vec<u8>)>,
    /// nothing more is coming for what was asked: it is here, or cannot be made
    pub done: bool,
}

/// the browser's end of the maker: what it wants pictures of now, and the pictures as they come
pub struct Pictures {
    wants: Sender<Vec<Want>>,
    got: Receiver<Got>,
}

impl Pictures {
    /// the browser's end, and the maker's two for `serve`.
    pub fn new() -> (Pictures, Receiver<Vec<Want>>, Sender<Got>) {
        let ((wants, theirs), (got, ours)) = (mpsc::channel(), mpsc::channel());
        (Pictures { wants, got: ours }, theirs, got)
    }

    /// everything wanted now, most wanted first. it takes the place of whatever was wanted before.
    pub fn want(&self, list: Vec<Want>) {
        let _ = self.wants.send(list);
    }

    pub fn take(&self) -> Option<Got> {
        self.got.try_recv().ok()
    }
}

/// the frames a clip's previews are of: `COUNT` of them from its first frame to its last (the prototype's
/// `round(i / 23 * (n - 1))`), fewer for a clip with fewer frames.
pub fn frames(clip_frames: usize) -> Vec<usize> {
    let last = clip_frames.saturating_sub(1);
    let mut frames: Vec<usize> = (0..COUNT).map(|i| (i * last + (COUNT - 1) / 2) / (COUNT - 1)).collect();
    frames.dedup();
    frames
}

/// the order `count` previews are made in, as places in `frames`: the poster first, which is the middle one, then
/// the first, the halves, the quarters and so on (the design's 0, 12, 6, 18 …), so a card scrubs coarsely at once.
fn order(count: usize) -> Vec<usize> {
    let mut order = vec![count / 2];
    let mut step = count.max(1);
    loop {
        order.extend((0..count).step_by(step).filter(|i| !order.contains(i)).collect::<Vec<_>>());
        if step == 1 {
            return order;
        }
        step /= 2;
    }
}

/// the frame a clip's poster is of.
pub fn poster(clip_frames: usize) -> usize {
    let of = frames(clip_frames);
    of[of.len() / 2]
}

/// the size of a clip's small pictures: its own shape, inside `BOX`, and no larger than the clip.
fn size(w: u32, h: u32) -> (u32, u32) {
    let scale = (BOX.0 as f64 / w.max(1) as f64).min(BOX.1 as f64 / h.max(1) as f64).min(1.0);
    (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
}

/// a picture drawn `by` times `w` x `h` each way, averaged down to it: each pixel the mean of a `by` x `by` block,
/// 8 bits from 16, without the alpha.
fn shrink(big: &[u16], w: u32, h: u32, by: u32) -> Vec<u8> {
    let (w, h, by) = (w as usize, h as usize, by as usize);
    let mut small = Vec::with_capacity(w * h * 3);
    for (y, x, c) in (0..h).flat_map(|y| (0..w).flat_map(move |x| (0..3).map(move |c| (y, x, c)))) {
        let sum: usize = (0..by).flat_map(|j| (0..by).map(move |i| (j, i))).map(|(j, i)| big[((y * by + j) * w * by + x * by + i) * 4 + c] as usize).sum();
        small.push((sum / (by * by) / 257) as u8);
    }
    small
}

/// the maker's work, for a thread of its own: the pictures of each file wanted, in the order wanted, from the cache
/// when they are there and from the file when not. the newest list is the only one that counts, and a file no
/// longer in it is left at the next frame. nothing is made while a clip plays, and nothing on a network volume is
/// read at all, whoever asks (adr 0004). returns when the browser's end is gone.
pub fn serve<R: Reader>(wants: Receiver<Vec<Want>>, got: Sender<Got>, cache: Option<Cache>, playing: &AtomicBool, mut open: impl FnMut(&Path) -> Option<R>) {
    let (mut list, mut at): (Vec<Want>, usize) = (Vec::new(), 0);
    // what has been answered in full and is still on the list: the browser may not have seen the answer yet
    let mut answered: Vec<Want> = Vec::new();
    loop {
        // with nothing left to do, wait for a new list
        let newer = if at < list.len() { wants.try_recv().map_err(|e| e == TryRecvError::Disconnected) } else { wants.recv().map_err(|_| true) };
        match newer {
            Ok(newer) => {
                (list, at) = (newer, 0);
                answered.retain(|w| list.contains(w));
                continue;
            }
            Err(true) => return,
            Err(false) => {}
        }
        let want = list[at].clone();
        at += 1;
        if answered.contains(&want) {
            continue;
        }
        // still wanted, by the newest list
        let mut wanted = || {
            while let Ok(newer) = wants.try_recv() {
                (list, at) = (newer, 0);
            }
            list.contains(&want)
        };
        if make(&want, cache.as_ref(), &got, playing, &mut open, &mut wanted) {
            answered.push(want);
        }
        answered.retain(|w| list.contains(w));
    }
}

/// one file's pictures. true when what was wanted was answered in full, false when it was left part way.
fn make<R: Reader>(want: &Want, cache: Option<&Cache>, got: &Sender<Got>, playing: &AtomicBool, open: &mut impl FnMut(&Path) -> Option<R>, wanted: &mut impl FnMut() -> bool) -> bool {
    let answer = |clip_frames: usize, (width, height): (u32, u32), pictures: Vec<(usize, Vec<u8>)>, done: bool| {
        let _ = got.send(Got { want: want.clone(), clip_frames, width, height, pictures, done });
    };
    if browse::volume_of(&want.path) == Volume::Network {
        answer(0, (0, 0), Vec::new(), true);
        return true;
    }
    // behind everything else: it waits while a clip plays, for as long as the pictures are still wanted
    let mut go_on = || {
        while playing.load(Ordering::Relaxed) && wanted() {
            std::thread::sleep(Duration::from_millis(100));
        }
        wanted()
    };
    let key = Cache::key(&want.path, want.size, want.modified);
    let mut have: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    let mut kept = None;
    if let Some(previews) = cache.and_then(|c| c.get(&key)) {
        let each = previews.width as usize * previews.height as usize * 3;
        have = previews.frames.iter().copied().zip(previews.pixels.chunks_exact(each).map(<[u8]>::to_vec)).collect();
        let shape = (previews.clip_frames, (previews.width, previews.height));
        let full = if want.all { frames(shape.0).iter().all(|f| have.contains_key(f)) } else { !have.is_empty() };
        answer(shape.0, shape.1, have.iter().map(|(frame, picture)| (*frame, picture.clone())).collect(), full);
        if full {
            return true;
        }
        kept = Some(shape);
    }
    if !go_on() {
        return false;
    }
    let Some(mut reader) = open(&want.path) else {
        answer(0, (0, 0), Vec::new(), true);
        return true;
    };
    let (clip_frames, clip) = (reader.frames(), reader.size());
    let small = size(clip.0, clip.1);
    // what the cache held is of another shape than the clip has now: it is made again
    if kept.is_some_and(|shape| shape != (clip_frames, small)) {
        have.clear();
    }
    let by = (clip.0 / small.0).min(clip.1 / small.1).clamp(1, OVER);
    let of = frames(clip_frames);
    let (mut made, mut left) = (false, false);
    for frame in order(of.len()).into_iter().map(|i| of[i]).take(if want.all { COUNT } else { 1 }) {
        if have.contains_key(&frame) {
            continue;
        }
        if !go_on() {
            left = true;
            break;
        }
        let Some(big) = reader.read(frame, small.0 * by, small.1 * by) else { continue };
        let picture = shrink(&big, small.0, small.1, by);
        answer(clip_frames, small, vec![(frame, picture.clone())], false);
        have.insert(frame, picture);
        made = true;
    }
    drop(reader);
    if let Some(cache) = cache.filter(|_| made) {
        let previews = Previews { width: small.0, height: small.1, clip_frames, frames: have.keys().copied().collect(), pixels: have.values().flatten().copied().collect() };
        // an error here can also mean the pictures are stored and no room was made: either way the card has them
        if let Err(why) = cache.put(&key, &previews) {
            eprintln!("eyepiece: the pictures' cache: {why}");
        }
    }
    if !left {
        answer(clip_frames, small, Vec::new(), true);
    }
    !left
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    #[test]
    fn previews_run_from_the_first_frame_to_the_last() {
        assert_eq!(frames(1800)[..3], [0, 78, 156]);
        assert_eq!((frames(1800).len(), frames(1800)[23], poster(1800)), (24, 1799, 939));
        // a clip with fewer frames has fewer pictures; a still has one
        assert_eq!(frames(3), [0, 1, 2]);
        assert_eq!((frames(1), poster(1), frames(0)), (vec![0], 0, vec![0]));
    }

    #[test]
    fn the_poster_is_made_first_then_the_card_scrubs_coarsely() {
        assert_eq!(order(24)[..9], [12, 0, 6, 18, 3, 9, 15, 21, 1]);
        let mut all = order(24);
        all.sort_unstable();
        assert_eq!(all, (0..24).collect::<Vec<_>>());
        assert_eq!((order(1), order(3)), (vec![0], vec![1, 0, 2]));
    }

    #[test]
    fn pictures_keep_the_clips_shape_inside_the_box() {
        assert_eq!((size(3840, 2160), size(1920, 1080), size(4096, 1716)), ((320, 180), (320, 180), (320, 134)));
        // tall, and already small
        assert_eq!((size(1080, 1920), size(64, 64), size(1, 9000)), ((101, 180), (64, 64), (1, 180)));
    }

    #[test]
    fn a_big_picture_is_averaged_down() {
        // 2 x 1 from 4 x 2: the left block is all white, the right half black and half white
        let (white, black) = ([65535u16; 4], [0u16, 0, 0, 65535]);
        let big = [white, white, black, white, white, white, black, white].concat();
        assert_eq!(shrink(&big, 2, 1, 2), [255, 255, 255, 127, 127, 127]);
    }

    /// a clip of ten gray frames, each as bright as its number; it counts how often it is opened and read.
    struct Gray(Arc<AtomicUsize>);

    impl Reader for Gray {
        fn frames(&self) -> usize {
            10
        }
        fn size(&self) -> (u32, u32) {
            (8, 4)
        }
        fn read(&mut self, frame: usize, w: u32, h: u32) -> Option<Vec<u16>> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Some(vec![frame as u16 * 257; (w * h * 4) as usize])
        }
    }

    /// asks for one file's pictures and gathers what comes back until the answer is done.
    fn ask(pictures: &Pictures, path: &Path, all: bool) -> Vec<(usize, u8)> {
        pictures.want(vec![Want { path: path.to_path_buf(), size: 1, modified: SystemTime::UNIX_EPOCH, all }]);
        let mut seen = Vec::new();
        loop {
            let got = pictures.got.recv_timeout(Duration::from_secs(20)).expect("an answer");
            assert!(got.want.path == path && got.want.all == all);
            seen.extend(got.pictures.iter().map(|(frame, picture)| (*frame, picture[0])));
            if got.done {
                assert_eq!((got.clip_frames, got.width, got.height), (10, 8, 4));
                return seen;
            }
        }
    }

    #[test]
    fn the_poster_is_one_frame_read_and_the_rest_come_when_asked() {
        let reads = Arc::new(AtomicUsize::new(0));
        let (pictures, wants, got) = Pictures::new();
        let theirs = reads.clone();
        let worker = std::thread::spawn(move || serve(wants, got, None, &AtomicBool::new(false), |_| Some(Gray(theirs.clone()))));
        let path = std::env::temp_dir().join("eyepiece-no-such-clip.mov");
        // the poster: the middle frame, and nothing else read
        assert_eq!(ask(&pictures, &path, false), [(5, 5)]);
        assert_eq!(reads.load(Ordering::Relaxed), 1);
        // all of them: every frame of the ten once, the poster first
        let all = ask(&pictures, &path, true);
        assert_eq!(all[..3], [(5, 5), (0, 0), (2, 2)]);
        assert_eq!(all.len(), 10);
        drop(pictures);
        worker.join().unwrap();
    }

    #[test]
    fn nothing_on_a_network_volume_is_opened() {
        let (pictures, wants, got) = Pictures::new();
        let worker = std::thread::spawn(move || serve(wants, got, None, &AtomicBool::new(false), |_| -> Option<Gray> { panic!("a file on a share was opened") }));
        pictures.want(vec![Want { path: PathBuf::from(r"\\server\share\a.mov"), size: 1, modified: SystemTime::UNIX_EPOCH, all: true }]);
        let got = pictures.got.recv_timeout(Duration::from_secs(20)).expect("an answer");
        assert!(got.done && got.pictures.is_empty());
        drop(pictures);
        worker.join().unwrap();
    }
}
