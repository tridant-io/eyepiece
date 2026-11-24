//! the cache of hover previews: the small pictures a browser card scrubs through, one file per clip, in the os cache
//! folder. nothing here reads media: a clip is named by its path, size and modified time as the folder listing gives
//! them. eviction removes only files this cache wrote, one at a time, and never a folder.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// the small pictures of one clip: frames spread over it, each `width` x `height`, red green blue, 8 bits each.
#[derive(PartialEq)]
pub struct Previews {
    pub width: u32,
    pub height: u32,
    /// how many frames the clip has, and which frame each picture is
    pub clip_frames: usize,
    pub frames: Vec<usize>,
    /// the pictures one after another: width * height * 3 bytes each
    pub pixels: Vec<u8>,
}

/// what names a clip's previews: its path, size and modified time. a changed file has another key.
#[derive(Debug, PartialEq)]
pub struct Key(u128);

pub struct Cache {
    root: PathBuf,
    /// bytes of cache files kept before eviction starts
    budget: u64,
}

/// the design's cap for hover previews (plan, section 06)
const BUDGET: u64 = 2 << 30;
/// where `open` keeps the cache, under the os cache folder
const PLACE: [&str; 3] = ["eyepiece", "cache", "previews"];
const MAGIC: [u8; 8] = *b"EYEPPREV";
const VERSION: u32 = 1;
/// magic, version, width, height, clip frames, picture count, packed length, checksum
const HEAD: usize = 44;
// far more than a card shows, so a file that claims more is damaged or not ours
const MAX_FRAMES: usize = 64;
const MAX_WIDTH: u32 = 640;
const MAX_HEIGHT: u32 = 360;

/// counts this process's writes, so two writers of one key never share a temporary file
static WRITES: AtomicU32 = AtomicU32::new(0);

impl Cache {
    /// the cache in the os cache folder: `<LOCALAPPDATA>\eyepiece\cache\previews` on windows. None when that
    /// place cannot be named safely.
    pub fn open() -> Option<Cache> {
        // macos will keep it in ~/Library/Caches/eyepiece; there is no mac build yet
        if !cfg!(windows) {
            return None;
        }
        let root = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(PLACE.iter().collect::<PathBuf>());
        safe_root(&root, home().as_deref(), &PLACE).then_some(Cache { root, budget: BUDGET })
    }

    /// a cache in a folder of the caller's choosing, with a size budget in bytes (the tests' way in).
    pub fn at(root: PathBuf, budget: u64) -> Option<Cache> {
        safe_root(&root, home().as_deref(), &[]).then_some(Cache { root, budget })
    }

    pub fn key(path: &Path, size: u64, modified: SystemTime) -> Key {
        let path = path.to_string_lossy();
        // names on windows are not case sensitive: one file, one key
        let path = if cfg!(windows) { path.to_lowercase() } else { path.into_owned() };
        let (before, since) = match modified.duration_since(UNIX_EPOCH) {
            Ok(d) => (0u8, d),
            Err(e) => (1, e.duration()),
        };
        // the fields after the path have fixed widths, so two different keys never run together into the same bytes
        let fields: [&[u8]; 5] =
            [path.as_bytes(), &size.to_le_bytes(), &[before], &since.as_secs().to_le_bytes(), &since.subsec_nanos().to_le_bytes()];
        // fnv-1a, 128 bits
        Key(fields.concat().iter().fold(0x6c62272e07bb014262b821756295c58d, |h, &b| {
            (h ^ b as u128).wrapping_mul(0x0000_0000_0100_0000_0000_0000_0000_013b)
        }))
    }

    /// the previews for this key, if they are there and whole. a damaged or foreign file reads as not there.
    pub fn get(&self, key: &Key) -> Option<Previews> {
        // opened to write as well, only to mark it used
        let mut file = fs::OpenOptions::new().read(true).write(true).open(self.file(key, "previews")).ok()?;
        let previews = read(&mut file)?;
        // eviction takes the file used longest ago: one that cannot be marked only goes sooner
        if let Err(why) = file.set_modified(SystemTime::now()) {
            eprintln!("eyepiece: could not mark hover previews used: {why}");
        }
        Some(previews)
    }

    /// stores a clip's previews, then makes room: while the cache is over its budget, the file used longest ago goes, one
    /// file at a time. an error after they are stored means no room was made this time.
    pub fn put(&self, key: &Key, previews: &Previews) -> io::Result<()> {
        if !whole(previews) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "hover previews the cache cannot hold"));
        }
        let bytes = encode(previews)?;
        fs::create_dir_all(&self.root)?;
        let tmp = self.file(key, &format!("tmp-{}{:06}", std::process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
        let mut out = File::create_new(&tmp)?;
        out.write_all(&bytes)?;
        out.sync_all()?;
        drop(out);
        fs::rename(&tmp, self.file(key, "previews"))?;
        self.evict()
    }

    fn file(&self, key: &Key, extension: &str) -> PathBuf {
        self.root.join(format!("{:032x}.{extension}", key.0))
    }

    /// while the cache files add up to more than the budget, removes the one used longest ago. only files named as
    /// this cache names them count, and the first error stops it.
    fn evict(&self) -> io::Result<()> {
        if !safe_root(&self.root, home().as_deref(), &[]) {
            return Err(io::Error::other(format!("not evicting: {} is not a safe cache folder", self.root.display())));
        }
        let day_ago = SystemTime::now().checked_sub(Duration::from_secs(24 * 60 * 60)).unwrap_or(UNIX_EPOCH);
        let (mut files, mut total) = (Vec::new(), 0);
        for item in fs::read_dir(&self.root)? {
            let item = item?;
            // from the listing, which does not follow links: a folder or a link is never one of ours
            let meta = item.metadata()?;
            let used = meta.modified()?;
            let name = item.file_name().into_string().unwrap_or_default();
            if meta.is_file() && ours(&name, used < day_ago) {
                total += meta.len();
                files.push((used, name, meta.len()));
            }
        }
        files.sort();
        for (_, name, len) in files {
            if total <= self.budget {
                break;
            }
            fs::remove_file(self.root.join(&name))?;
            total -= len;
        }
        Ok(())
    }
}

/// a name this cache writes: `<32 hex>.previews`, or `<32 hex>.tmp-<digits>` when `old`, which a crash left behind
fn ours(name: &str, old: bool) -> bool {
    let Some((stem, extension)) = name.split_once('.') else { return false };
    let hex = stem.len() == 32 && stem.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    let tmp = extension.strip_prefix("tmp-").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    hex && (extension == "previews" || old && tmp)
}

/// whether the cache may live in and evict from `root`: an absolute path at least three folders deep, so never a
/// drive or share root, not the home folder or a folder above it, and ending in `tail`. it is judged as written,
/// without asking the file system, and a `..` anywhere refuses it.
fn safe_root(root: &Path, home: Option<&Path>, tail: &[&str]) -> bool {
    let (Some(root), Some(home)) = (parts(root), home.and_then(parts)) else { return false };
    let tail: Vec<String> = tail.iter().map(|t| t.to_string()).collect();
    // root[0] is the drive or share
    root.len() > 3 && !home.starts_with(&root) && root.ends_with(&tail)
}

/// a path's parts for comparing: the drive or share, then each folder, lowercased on windows, where names are not
/// case sensitive. none when it is relative, climbs with `..`, or starts with a device or `\\?\` prefix.
fn parts(path: &Path) -> Option<Vec<String>> {
    if !path.is_absolute() {
        return None;
    }
    let mut out = Vec::new();
    for part in path.components() {
        let part = match part {
            Component::Prefix(p) if matches!(p.kind(), Prefix::Disk(_) | Prefix::UNC(..)) => p.as_os_str().to_string_lossy().into_owned(),
            Component::Prefix(_) | Component::ParentDir => return None,
            // on windows the root belongs to the drive before it; elsewhere it is the start
            Component::RootDir if out.is_empty() => "/".into(),
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => name.to_string_lossy().into_owned(),
        };
        out.push(if cfg!(windows) { part.to_lowercase() } else { part });
    }
    Some(out)
}

/// the user's home folder: the cache never runs in it or in a folder above it
fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// how many bytes `count` pictures of `width` x `height` take, when that is within what a cache file may hold
fn pixel_len(width: u32, height: u32, count: usize) -> Option<usize> {
    let fits = (1..=MAX_WIDTH).contains(&width) && (1..=MAX_HEIGHT).contains(&height) && (1..=MAX_FRAMES).contains(&count);
    fits.then(|| width as usize * height as usize * 3 * count)
}

/// previews the cache can hold and read back: within the limits, the pixels as long as the shape, the frames in the clip
fn whole(previews: &Previews) -> bool {
    pixel_len(previews.width, previews.height, previews.frames.len()) == Some(previews.pixels.len())
        && previews.frames.iter().all(|&f| f < previews.clip_frames)
}

/// fnv-1a over the pixels eight bytes at a time. each step is one to one, so a changed byte always changes the sum
fn checksum(bytes: &[u8]) -> u64 {
    let mix = |h: u64, w: u64| (h ^ w).wrapping_mul(0x100000001b3);
    let (words, rest) = bytes.as_chunks::<8>();
    let h = words.iter().fold(0xcbf29ce484222325, |h, &w| mix(h, u64::from_le_bytes(w)));
    rest.iter().fold(h, |h, &b| mix(h, b as u64))
}

/// a cache file: the head, the frame numbers, then the pixels packed with snappy. numbers are little endian
fn encode(previews: &Previews) -> io::Result<Vec<u8>> {
    let packed = snap::raw::Encoder::new().compress_vec(&previews.pixels).map_err(io::Error::other)?;
    let head: [&[u8]; 8] = [
        &MAGIC,
        &VERSION.to_le_bytes(),
        &previews.width.to_le_bytes(),
        &previews.height.to_le_bytes(),
        &(previews.clip_frames as u64).to_le_bytes(),
        &(previews.frames.len() as u32).to_le_bytes(),
        &(packed.len() as u32).to_le_bytes(),
        &checksum(&previews.pixels).to_le_bytes(),
    ];
    let mut out = head.concat();
    for &f in &previews.frames {
        out.extend_from_slice(&(f as u64).to_le_bytes());
    }
    out.extend_from_slice(&packed);
    Ok(out)
}

/// a clip's previews from a cache file. every length is checked against the limits and the file's own length before anything is
/// allocated from it, and the pixels against their checksum: anything off is none.
fn read(file: &mut File) -> Option<Previews> {
    let mut head = [0; HEAD];
    file.read_exact(&mut head).ok()?;
    let u32_at = |at: usize| u32::from_le_bytes(head[at..at + 4].try_into().unwrap());
    let u64_at = |at: usize| u64::from_le_bytes(head[at..at + 8].try_into().unwrap());
    let (width, height, count, packed) = (u32_at(12), u32_at(16), u32_at(28) as usize, u32_at(32) as usize);
    let len = pixel_len(width, height, count)?;
    if head[..8] != MAGIC || u32_at(8) != VERSION || packed > snap::raw::max_compress_len(len) {
        return None;
    }
    let body = 8 * count + packed;
    if file.metadata().ok()?.len() != (HEAD + body) as u64 {
        return None;
    }
    let mut rest = vec![0; body];
    file.read_exact(&mut rest).ok()?;
    let (frames, packed) = rest.split_at(8 * count);
    if snap::raw::decompress_len(packed).ok()? != len {
        return None;
    }
    let mut pixels = vec![0; len];
    snap::raw::Decoder::new().decompress(packed, &mut pixels).ok()?;
    let frames = frames.as_chunks::<8>().0.iter().map(|&f| usize::try_from(u64::from_le_bytes(f)).ok()).collect::<Option<_>>()?;
    let previews = Previews { width, height, clip_frames: usize::try_from(u64_at(20)).ok()?, frames, pixels };
    (checksum(&previews.pixels) == u64_at(36) && whole(&previews)).then_some(previews)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a folder of its own for one test, under the system temp folder. not made yet: the cache makes it
    fn scratch(test: &str) -> PathBuf {
        std::env::temp_dir().join(format!("eyepiece-preview-{test}-{}", std::process::id()))
    }

    /// removes what one test made: each file, each empty folder, then the test's folder. nothing recursive, and a failed
    /// test leaves its files for a look
    fn tidy(dir: &Path) {
        assert!(dir.file_name().unwrap().to_string_lossy().starts_with("eyepiece-preview-"));
        for item in fs::read_dir(dir).unwrap() {
            let item = item.unwrap();
            if item.file_type().unwrap().is_dir() {
                fs::remove_dir(item.path()).unwrap();
            } else {
                fs::remove_file(item.path()).unwrap();
            }
        }
        fs::remove_dir(dir).unwrap();
    }

    fn previews(width: u32, height: u32, count: usize, mut fill: impl FnMut() -> u8) -> Previews {
        let pixels = (0..width as usize * height as usize * 3 * count).map(|_| fill()).collect();
        Previews { width, height, clip_frames: 1000, frames: (0..count).map(|i| i * 40).collect(), pixels }
    }

    /// bytes from a seed that snappy cannot pack (xorshift)
    fn noise(mut x: u64) -> impl FnMut() -> u8 {
        move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        }
    }

    fn key(name: &str) -> Key {
        Cache::key(Path::new(name), 1, UNIX_EPOCH)
    }

    /// makes a file look last used `hours` ago
    fn used_ago(file: &Path, hours: u64) {
        let then = SystemTime::now() - Duration::from_secs(hours * 60 * 60);
        File::options().write(true).open(file).unwrap().set_modified(then).unwrap();
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir).unwrap().map(|i| i.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn round_trip() {
        let dir = scratch("round-trip");
        let cache = Cache::at(dir.clone(), u64::MAX).unwrap();
        let (k, s) = (key("a.mov"), previews(64, 36, 24, noise(1)));
        assert!(cache.get(&k).is_none());
        cache.put(&k, &s).unwrap();
        assert!(cache.get(&k).unwrap() == s);
        // one file, and no temporary one left
        assert_eq!(names(&dir), [format!("{:032x}.previews", k.0)]);
        tidy(&dir);
    }

    #[test]
    fn two_writers_of_one_key() {
        let dir = scratch("two-writers");
        let k = &key("a.mov");
        let (a, b) = (&previews(16, 9, 24, noise(1)), &previews(16, 9, 24, noise(2)));
        std::thread::scope(|threads| {
            for s in [a, b] {
                let cache = Cache::at(dir.clone(), u64::MAX).unwrap();
                threads.spawn(move || (0..20).for_each(|_| cache.put(k, s).unwrap()));
            }
            // a reader meanwhile sees nothing yet, or one clip's previews whole
            let cache = Cache::at(dir.clone(), u64::MAX).unwrap();
            threads.spawn(move || {
                for _ in 0..200 {
                    assert!(cache.get(k).is_none_or(|s| s == *a || s == *b));
                }
            });
        });
        let got = Cache::at(dir.clone(), u64::MAX).unwrap().get(k).unwrap();
        assert!(got == *a || got == *b);
        assert_eq!(names(&dir).len(), 1);
        tidy(&dir);
    }

    #[test]
    fn damaged_files_read_as_none() {
        let dir = scratch("damaged");
        let cache = Cache::at(dir.clone(), u64::MAX).unwrap();
        let k = key("a.mov");
        cache.put(&k, &previews(32, 18, 24, noise(3))).unwrap();
        let file = cache.file(&k, "previews");
        let good = fs::read(&file).unwrap();
        let set = |at: usize, value: &[u8]| {
            let mut bad = good.clone();
            bad[at..at + value.len()].copy_from_slice(value);
            bad
        };
        // the last byte is a pixel in a run snappy stored as it is, so it unpacks and only the checksum can tell
        let mut flipped = good.clone();
        *flipped.last_mut().unwrap() ^= 1;
        let packed = &flipped[HEAD + 8 * 24..];
        assert!(snap::raw::Decoder::new().decompress_vec(packed).is_ok());
        let damaged = [
            ("empty", Vec::new()),
            ("half a head", good[..HEAD / 2].to_vec()),
            ("the head alone", good[..HEAD].to_vec()),
            ("one byte short", good[..good.len() - 1].to_vec()),
            ("one byte over", [good.as_slice(), &[0]].concat()),
            ("magic", set(0, b"EYEPSTRX")),
            ("version", set(8, &2u32.to_le_bytes())),
            ("too wide", set(12, &641u32.to_le_bytes())),
            ("no height", set(16, &0u32.to_le_bytes())),
            ("frames outside the clip", set(20, &10u64.to_le_bytes())),
            ("a count that lies", set(28, &64u32.to_le_bytes())),
            ("a packed length that lies", set(32, &(good.len() as u32).to_le_bytes())),
            ("a huge packed length", set(32, &u32::MAX.to_le_bytes())),
            ("checksum", set(36, &0u64.to_le_bytes())),
            ("a flipped pixel", flipped),
        ];
        for (what, bytes) in damaged {
            fs::write(&file, bytes).unwrap();
            assert!(cache.get(&k).is_none(), "{what}");
        }
        fs::write(&file, &good).unwrap();
        assert!(cache.get(&k).is_some());
        tidy(&dir);
    }

    #[test]
    fn put_refuses_what_would_not_read_back() {
        let dir = scratch("refuses");
        let cache = Cache::at(dir.clone(), u64::MAX).unwrap();
        let mut short = previews(8, 8, 2, noise(4));
        short.pixels.pop();
        let mut outside = previews(8, 8, 2, noise(4));
        outside.clip_frames = 40;
        for s in [short, outside, previews(641, 1, 1, noise(4)), previews(1, 361, 1, noise(4)), previews(1, 1, 65, noise(4))] {
            assert_eq!(cache.put(&key("a.mov"), &s).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
        assert!(!dir.exists());
    }

    #[test]
    fn key_changes_with_the_file() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let k = |path: &str, size, time| Cache::key(Path::new(path), size, time);
        let base = k(r"D:\shots\a.mov", 100, t);
        assert_eq!(k(r"D:\shots\a.mov", 100, t), base);
        assert_ne!(k(r"D:\shots\a.mov", 101, t), base);
        // windows keeps file times in steps of 100 ns
        assert_ne!(k(r"D:\shots\a.mov", 100, t + Duration::from_nanos(100)), base);
        assert_ne!(k(r"D:\shots\a.mov", 100, t + Duration::from_secs(1)), base);
        assert_ne!(k(r"D:\shots\b.mov", 100, t), base);
        // before 1970 is not after it
        let five = Duration::from_secs(5);
        assert_ne!(k(r"D:\shots\a.mov", 100, UNIX_EPOCH - five), k(r"D:\shots\a.mov", 100, UNIX_EPOCH + five));
        #[cfg(windows)]
        assert_eq!(k(r"d:\SHOTS\A.MOV", 100, t), base);
    }

    #[test]
    fn eviction_takes_the_least_recently_used_and_only_ours() {
        let dir = scratch("evict");
        let all = Cache::at(dir.clone(), u64::MAX).unwrap();
        let flat = |v: u8| previews(8, 8, 2, move || v);
        let keys = [key("a.mov"), key("b.mov"), key("c.mov"), key("d.mov")];
        for (i, k) in keys[..3].iter().enumerate() {
            all.put(k, &flat(i as u8)).unwrap();
        }
        for (k, hours) in keys.iter().zip([3, 2, 1]) {
            used_ago(&all.file(k, "previews"), hours);
        }
        // not ours, and older than anything, so they would go first if they counted
        let hex = "0123456789abcdef0123456789abcdef";
        let foreign = [
            "notes.txt".to_string(),
            format!("{}.previews", &hex[1..]),
            // not the folder's name in capitals: on windows that would be the folder
            "FEDCBA9876543210FEDCBA9876543210.previews".to_string(),
            format!("{hex}.previews.bak"),
            format!("{hex}.tmp-12x"),
            format!("{hex}.tmp-"),
            format!("{hex}.txt"),
        ];
        let folder = dir.join(format!("{hex}.previews"));
        fs::create_dir(&folder).unwrap();
        let inside = folder.join(format!("{hex}.previews"));
        let made = |file: &Path, hours| {
            fs::write(file, [7; 1000]).unwrap();
            used_ago(file, hours);
        };
        for name in &foreign {
            made(&dir.join(name), 72);
        }
        made(&inside, 72);
        // ours: a temporary file a crash left two days ago goes, one being written now does not count
        let (left, writing) = (dir.join(format!("{hex}.tmp-4242000001")), dir.join(format!("{hex}.tmp-4242000002")));
        made(&left, 48);
        made(&writing, 0);
        // room for two: storing the fourth makes it
        let len = |k: &Key| fs::metadata(all.file(k, "previews")).unwrap().len();
        Cache::at(dir.clone(), 2 * len(&keys[2])).unwrap().put(&keys[3], &flat(3)).unwrap();
        assert_eq!(len(&keys[3]), len(&keys[2]));
        let there = |k: &Key| all.file(k, "previews").exists();
        assert_eq!(keys.each_ref().map(there), [false, false, true, true]);
        assert!(!left.exists() && writing.exists());
        for name in &foreign {
            assert_eq!(fs::metadata(dir.join(name)).unwrap().len(), 1000, "{name}");
        }
        assert_eq!(fs::metadata(&inside).unwrap().len(), 1000);
        fs::remove_file(&inside).unwrap();
        tidy(&dir);
    }

    #[test]
    fn get_marks_the_file_used() {
        let dir = scratch("used");
        let all = Cache::at(dir.clone(), u64::MAX).unwrap();
        let (a, b) = (key("a.mov"), key("b.mov"));
        all.put(&a, &previews(8, 8, 2, || 1)).unwrap();
        all.put(&b, &previews(8, 8, 2, || 2)).unwrap();
        used_ago(&all.file(&a, "previews"), 2);
        used_ago(&all.file(&b, "previews"), 1);
        assert!(all.get(&a).is_some());
        let one = fs::metadata(all.file(&a, "previews")).unwrap().len();
        Cache::at(dir.clone(), one).unwrap().evict().unwrap();
        assert!(all.file(&a, "previews").exists() && !all.file(&b, "previews").exists());
        tidy(&dir);
    }

    #[test]
    fn eviction_refuses_an_unsafe_root() {
        // what an empty LOCALAPPDATA would give: relative, so it names the working folder
        let cache = Cache { root: PathBuf::from(r"eyepiece\cache\previews"), budget: 0 };
        assert!(cache.evict().is_err());
        assert!(Cache::at(PathBuf::new(), 0).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn safe_root_refuses_roots_and_homes() {
        let home = Some(Path::new(r"C:\Users\admin"));
        let ok = |root: &str| safe_root(Path::new(root), home, &[]);
        let place = |root: &str| safe_root(Path::new(root), home, &PLACE);
        for refused in [
            "",
            r"eyepiece\cache\previews",
            r"a\b\c\d",
            r"\a\b\c",
            r"C:a\b\c",
            r"C:\",
            r"C:",
            r"\\nas\share",
            r"\\nas\share\",
            r"\\?\C:\",
            r"\\?\C:\Users\admin\AppData\Local\eyepiece",
            r"\\?\UNC\nas\share\",
            r"\\.\C:\a\b\c",
            r"C:\Users",
            r"C:\Users\admin",
            r"c:\USERS\Admin\",
            r"C:/Users/admin",
            r"C:\Users\admin\.",
            r"C:\x\y",
            r"C:\Users\admin\AppData\Local\..\..\..\x",
        ] {
            assert!(!ok(refused), "{refused}");
        }
        assert!(ok(r"C:\Users\admin\AppData\Local\Temp\eyepiece-x"));
        assert!(ok(r"\\nas\share\a\b\c"));
        // a home deeper than three folders: it and the folders above it are still refused
        let deep = Some(Path::new(r"D:\a\b\c\home"));
        assert!(!safe_root(Path::new(r"D:\a\b\c"), deep, &[]) && !safe_root(Path::new(r"D:\a\b\c\home"), deep, &[]));
        assert!(safe_root(Path::new(r"D:\a\b\c\home\x"), deep, &[]));
        // with no home to compare against, nothing is safe
        assert!(!safe_root(Path::new(r"C:\a\b\c"), None, &[]));
        // open's own place, and nowhere else
        assert!(place(r"C:\Users\admin\AppData\Local\eyepiece\cache\previews"));
        assert!(place(r"c:\users\admin\appdata\local\EYEPIECE\Cache\Previews"));
        assert!(!place(r"C:\Users\admin\AppData\Local\eyepiece\cache\shaders"));
        assert!(!place(r"C:\Users\admin\AppData\Local\eyepiece\cache"));
        assert!(!place(r"C:\eyepiece\cache\previews\x"));
        assert!(!place(r"C:\Users\admin\eyepiece\cache\previews\.."));
    }

    #[cfg(windows)]
    #[test]
    fn open_is_in_local_app_data() {
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        assert_eq!(Cache::open().unwrap().root, local.join(r"eyepiece\cache\previews"));
    }

    #[test]
    fn cache_is_shared_between_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Cache>();
    }
}
