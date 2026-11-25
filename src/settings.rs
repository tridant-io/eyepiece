//! what is saved between runs: one small text file of `key = value` lines that a person can read and edit. speed,
//! zoom and pan, mirror, flip and rotate, markup and which tabs were open are not saved, on purpose (plan, section 09).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// what playing past the last frame does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loop {
    Off,
    One,
    All,
}

/// how 4:2:0 and 4:2:2 chroma is brought up to full size
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chroma {
    Bilinear,
    Nearest,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// the readout counts in timecode instead of frames
    pub timecode: bool,
    /// the number the first frame gets: 0 or 1
    pub first_frame: usize,
    pub loop_file: Loop,
    pub loop_folder: Loop,
    /// how many seconds loop analysis plays each side of the seam, 0.5 to 10
    pub loop_analysis_seconds: f32,
    /// 0 to 1
    pub volume: f32,
    pub mute: bool,
    /// the controls are pinned up (the pin on the bar)
    pub pin_controls: bool,
    /// the guides: a grid of tiles, and center lines with a circle
    pub grid: bool,
    pub center: bool,
    /// a grid tile's width and height, in the clip's pixels
    pub tile: (u32, u32),
    pub guide_color: [u8; 3],
    pub chroma: Chroma,
    /// what a screenshot carries besides the picture
    pub screenshot_caption: bool,
    pub screenshot_guides: bool,
    pub screenshot_markup: bool,
    pub browser_folder: Option<PathBuf>,
    /// the folders the browser last opened files from, the newest first
    pub browser_recent: Vec<PathBuf>,
    /// what the browser shows of a folder: its column and its cards, the cards, or the column
    pub browser_view: crate::browse::View,
    /// how wide the sidebar, the folders' columns and the metadata column are
    pub browser_widths: crate::browse::Widths,
    /// a file starts playing when it opens (the owner's ask, 2026-11-24); off, it opens paused on its first frame
    pub play_on_open: bool,
    pub window_size: Option<(u32, u32)>,
    /// none means the pictures folder (`shot::folder`)
    pub screenshot_folder: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            timecode: false,
            first_frame: 0,
            loop_file: Loop::One,
            loop_folder: Loop::All,
            loop_analysis_seconds: 3.0,
            volume: 0.8,
            mute: false,
            pin_controls: false,
            grid: false,
            center: false,
            tile: (240, 240),
            guide_color: [0x3d, 0xf0, 0xff],
            chroma: Chroma::Bilinear,
            screenshot_caption: false,
            screenshot_guides: false,
            screenshot_markup: true,
            browser_folder: None,
            browser_recent: Vec::new(),
            browser_view: crate::browse::View::Both,
            browser_widths: crate::browse::Widths::default(),
            play_on_open: true,
            window_size: None,
            screenshot_folder: None,
        }
    }
}

impl Settings {
    /// never fails the program: a missing or unreadable file, or a bad line, gives the default for what it could not read.
    pub fn load() -> Settings {
        file().map_or_else(Settings::default, |f| Settings::load_from(&f))
    }

    pub fn load_from(file: &Path) -> Settings {
        let mut settings = Settings::default();
        for (key, value, _) in entries(&read(file)) {
            // a value that does not read leaves the default
            settings.set(key, value);
        }
        settings
    }

    /// writes beside the file and renames over it, so a crash cannot leave half a file. keys this build does not know are kept.
    pub fn save(&self) -> io::Result<()> {
        self.save_to(&file().ok_or_else(|| io::Error::other("the system names no settings folder"))?)
    }

    pub fn save_to(&self, file: &Path) -> io::Result<()> {
        let pairs = self.pairs();
        let mut text = String::new();
        for (key, value) in &pairs {
            text += format!("{key} = {value}").trim_end();
            text.push('\n');
        }
        // what a newer build wrote stays as it was
        for (key, _, line) in entries(&read(file)) {
            if !pairs.iter().any(|(known, _)| *known == key) {
                text += line;
                text.push('\n');
            }
        }
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("tmp");
        let mut out = fs::File::create(&tmp)?;
        out.write_all(text.as_bytes())?;
        // on the disk before the rename, so not even a power cut leaves half a file
        out.sync_all()?;
        drop(out);
        fs::rename(&tmp, file)
    }

    /// every value as it is written, in the file's order
    fn pairs(&self) -> [(&'static str, String); 23] {
        let wxh = |(w, h): (u32, u32)| format!("{w}x{h}");
        let path = |p: &Option<PathBuf>| p.as_ref().map_or(String::new(), |p| p.to_string_lossy().into_owned());
        let recent: Vec<_> = self.browser_recent.iter().map(|p| p.to_string_lossy()).collect();
        let [r, g, b] = self.guide_color;
        [
            ("readout", word(&READOUT, self.timecode)),
            ("first_frame", self.first_frame.to_string()),
            ("loop_file", word(&LOOPS, self.loop_file)),
            ("loop_folder", word(&LOOPS, self.loop_folder)),
            ("loop_analysis_seconds", self.loop_analysis_seconds.to_string()),
            ("volume", self.volume.to_string()),
            ("mute", word(&SWITCH, self.mute)),
            ("pin_controls", word(&SWITCH, self.pin_controls)),
            ("grid", word(&SWITCH, self.grid)),
            ("center", word(&SWITCH, self.center)),
            ("tile", wxh(self.tile)),
            ("guide_color", format!("#{r:02x}{g:02x}{b:02x}")),
            ("chroma", word(&CHROMA, self.chroma)),
            ("screenshot_caption", word(&SWITCH, self.screenshot_caption)),
            ("screenshot_guides", word(&SWITCH, self.screenshot_guides)),
            ("screenshot_markup", word(&SWITCH, self.screenshot_markup)),
            ("browser_folder", path(&self.browser_folder)),
            ("browser_recent", recent.join(" | ")),
            ("browser_view", word(&VIEWS, self.browser_view)),
            ("browser_widths", format!("{} {} {}", self.browser_widths.side, self.browser_widths.column, self.browser_widths.meta)),
            ("play_on_open", word(&SWITCH, self.play_on_open)),
            ("window_size", self.window_size.map_or(String::new(), wxh)),
            ("screenshot_folder", path(&self.screenshot_folder)),
        ]
    }

    /// takes one value from the file. none when it does not read, and then what was there stays
    fn set(&mut self, key: &str, v: &str) -> Option<()> {
        match key {
            "readout" => self.timecode = pick(&READOUT, v)?,
            "first_frame" => self.first_frame = v.parse().ok().filter(|n| *n <= 1)?,
            "loop_file" => self.loop_file = pick(&LOOPS, v)?,
            "loop_folder" => self.loop_folder = pick(&LOOPS, v)?,
            "loop_analysis_seconds" => self.loop_analysis_seconds = number(v, 0.5, 10.0)?,
            "volume" => self.volume = number(v, 0.0, 1.0)?,
            "mute" => self.mute = pick(&SWITCH, v)?,
            "pin_controls" => self.pin_controls = pick(&SWITCH, v)?,
            "grid" => self.grid = pick(&SWITCH, v)?,
            "center" => self.center = pick(&SWITCH, v)?,
            "tile" => self.tile = size(v)?,
            "guide_color" => self.guide_color = color(v)?,
            "chroma" => self.chroma = pick(&CHROMA, v)?,
            "screenshot_caption" => self.screenshot_caption = pick(&SWITCH, v)?,
            "screenshot_guides" => self.screenshot_guides = pick(&SWITCH, v)?,
            "screenshot_markup" => self.screenshot_markup = pick(&SWITCH, v)?,
            "browser_folder" => self.browser_folder = folder(v),
            // a bar cannot be in a windows path
            "browser_recent" => self.browser_recent = v.split('|').filter_map(|p| folder(p.trim())).collect(),
            "browser_view" => self.browser_view = pick(&VIEWS, v)?,
            "browser_widths" => {
                let [side, column, meta]: [f32; 3] = v.split_whitespace().map(|n| n.parse().ok()).collect::<Option<Vec<f32>>>()?.try_into().ok()?;
                self.browser_widths = crate::browse::Widths { side, column, meta }.kept();
            }
            "play_on_open" => self.play_on_open = pick(&SWITCH, v)?,
            "window_size" => self.window_size = if v.is_empty() { None } else { Some(size(v)?) },
            "screenshot_folder" => self.screenshot_folder = folder(v),
            _ => return None,
        }
        Some(())
    }
}

// the words the file uses, both ways
const READOUT: [(&str, bool); 2] = [("frames", false), ("timecode", true)];
const SWITCH: [(&str, bool); 2] = [("off", false), ("on", true)];
const LOOPS: [(&str, Loop); 3] = [("off", Loop::Off), ("one", Loop::One), ("all", Loop::All)];
const CHROMA: [(&str, Chroma); 2] = [("bilinear", Chroma::Bilinear), ("nearest", Chroma::Nearest)];
const VIEWS: [(&str, crate::browse::View); 3] = [("both", crate::browse::View::Both), ("gallery", crate::browse::View::Gallery), ("list", crate::browse::View::List)];

fn word<T: Copy + PartialEq>(words: &[(&str, T)], value: T) -> String {
    words.iter().find(|w| w.1 == value).map_or("", |w| w.0).to_string()
}

fn pick<T: Copy>(words: &[(&str, T)], v: &str) -> Option<T> {
    words.iter().find(|w| w.0 == v).map(|w| w.1)
}

/// `settings.txt` in the system's settings folder: %APPDATA%\eyepiece on windows
fn file() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = PathBuf::from(std::env::var_os("APPDATA")?);
    // macos. linux comes in phase 5
    #[cfg(not(windows))]
    let base = PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support");
    Some(base.join("eyepiece").join("settings.txt"))
}

/// the file as text, or nothing. bytes that are not utf-8 spoil only their own line
fn read(file: &Path) -> String {
    let text = String::from_utf8_lossy(&fs::read(file).unwrap_or_default()).into_owned();
    // notepad may start the file with a byte order mark
    text.trim_start_matches('\u{feff}').to_string()
}

/// the `key = value` lines, trimmed, each with the line itself. a line without `=` is not one
fn entries(text: &str) -> impl Iterator<Item = (&str, &str, &str)> {
    text.lines().filter_map(|line| line.split_once('=').map(|(k, v)| (k.trim(), v.trim(), line)))
}

/// a number inside a range: out of it, or not a number at all, it does not read
fn number(v: &str, lo: f32, hi: f32) -> Option<f32> {
    v.parse().ok().filter(|n| (lo..=hi).contains(n))
}

/// `width x height`, neither of them zero
fn size(v: &str) -> Option<(u32, u32)> {
    let (w, h) = v.split_once('x')?;
    let side = |s: &str| s.trim().parse().ok().filter(|n| *n > 0);
    Some((side(w)?, side(h)?))
}

/// `#rrggbb`
fn color(v: &str) -> Option<[u8; 3]> {
    let hex = v.strip_prefix('#').filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

fn folder(v: &str) -> Option<PathBuf> {
    (!v.is_empty()).then(|| PathBuf::from(v))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a folder of its own for one test, not made yet: `save_to` makes it
    fn scratch(test: &str) -> PathBuf {
        std::env::temp_dir().join(format!("eyepiece-settings-{test}-{}", std::process::id()))
    }

    /// removes the settings file, then the empty folder. a failed test leaves them for a look
    fn tidy(dir: &Path) {
        fs::remove_file(dir.join("settings.txt")).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn defaults_when_missing() {
        let dir = scratch("missing");
        assert_eq!(Settings::load_from(&dir.join("settings.txt")), Settings::default());
        assert!(!dir.exists());
    }

    #[test]
    fn round_trip() {
        let dir = scratch("round-trip");
        let file = dir.join("settings.txt");
        // every field away from its default, so one that does not survive shows
        let saved = Settings {
            timecode: true,
            first_frame: 1,
            loop_file: Loop::Off,
            loop_folder: Loop::One,
            loop_analysis_seconds: 1.5,
            volume: 0.25,
            mute: true,
            pin_controls: true,
            grid: true,
            center: true,
            tile: (64, 32),
            guide_color: [0xff, 0x00, 0x80],
            chroma: Chroma::Nearest,
            screenshot_caption: true,
            screenshot_guides: true,
            screenshot_markup: false,
            browser_folder: Some(PathBuf::from(r"D:\delivery 2026-11-20\wall")),
            browser_recent: vec![PathBuf::from(r"D:\delivery 2026-11-20\wall"), PathBuf::from(r"\\nas\renders = out\totems")],
            browser_view: crate::browse::View::List,
            browser_widths: crate::browse::Widths { side: 180.0, column: 200.5, meta: 300.0 },
            play_on_open: false,
            window_size: Some((1280, 720)),
            screenshot_folder: Some(PathBuf::from(r"\\nas\review = notes")),
        };
        saved.save_to(&file).unwrap();
        assert_eq!(Settings::load_from(&file), saved);
        assert!(!dir.join("settings.tmp").exists());
        tidy(&dir);
    }

    #[test]
    fn unknown_keys_are_kept_and_bad_lines_give_defaults() {
        let dir = scratch("odd");
        let file = dir.join("settings.txt");
        fs::create_dir_all(&dir).unwrap();
        let by_hand = "\u{feff}volume = 0.5\r\nfrom_a_newer_build = 42\r\nthis line is broken\r\nmute = maybe\r\ntile = 0x32\r\nguide_color=#FF8800\r\n";
        fs::write(&file, by_hand).unwrap();
        let loaded = Settings::load_from(&file);
        let defaults = Settings::default();
        assert_eq!((loaded.volume, loaded.guide_color), (0.5, [0xff, 0x88, 0x00]));
        assert_eq!((loaded.mute, loaded.tile), (defaults.mute, defaults.tile));
        loaded.save_to(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("\nfrom_a_newer_build = 42\n"), "{text}");
        assert!(text.contains("\nmute = off\n") && !text.contains("broken"), "{text}");
        assert_eq!(Settings::load_from(&file), loaded);
        tidy(&dir);
    }
}
