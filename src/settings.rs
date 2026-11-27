//! what is saved between runs: one small text file of `key = value` lines that a person can read and edit. speed,
//! zoom and pan, mirror, flip and rotate and which tabs were open are not saved, on purpose (plan, section 09). a
//! clip's notes are not kept here but beside the clip (`crate::notes`).

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

/// where a file opened from the browser goes: in place of what the tab shows, or into a tab of its own
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opens {
    ThisTab,
    NewTab,
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
    /// how much a pen's line is smoothed as it is drawn, 0 (off) to 1
    pub pen_smoothing: f32,
    /// notes are saved in the clip's folder, in its eyepiece.yaml (adr 0010); off, in eyepiece's own folder
    pub notes_beside: bool,
    /// how long a new group of marks lasts from the frame it is drawn on, in seconds, 0.5 to 60
    pub note_seconds: f32,
    /// a saved copy takes the clip's groups on its frames as notes of its own (the owner's "include annotations",
    /// 2026-11-26); off, it has none
    pub copy_notes: bool,
    pub browser_folder: Option<PathBuf>,
    /// the folders the browser last opened files from, the newest first
    pub browser_recent: Vec<PathBuf>,
    /// what the browser shows of a folder: its column and its cards, the cards, or the column
    pub browser_view: crate::browse::View,
    /// how the browser orders a folder's entries
    pub browser_sort: crate::browse::Sort,
    /// how wide the sidebar, the folders' columns and the metadata column are
    pub browser_widths: crate::browse::Widths,
    /// a file starts playing when it opens (the owner's ask, 2026-11-24); off, it opens paused on its first frame
    pub play_on_open: bool,
    pub browser_opens_in: Opens,
    /// a second launch hands its file to the player open already; off, it opens a window of its own
    pub one_window: bool,
    /// how long the controls stay up once the pointer rests off them, 1 to 10 seconds
    pub controls_after: f32,
    /// the window opens at the size it was left at (`window_size`) when no file gives it one
    pub remember_size: bool,
    /// eyepiece asks for a newer release when it starts; and when one was last answered, in seconds since 1970
    pub update_check: bool,
    pub update_checked: Option<u64>,
    /// the picture area's color round a picture of another shape
    pub background: [u8; 3],
    /// the folder the browser starts in; none starts it where it was last (`browser_folder`)
    pub browser_start: Option<PathBuf>,
    /// how big the preview cache grows before its oldest files go, in GB, 1 to 64
    pub cache_limit: f32,
    /// the picture area's size in physical pixels when the window last closed
    pub window_size: Option<(u32, u32)>,
    /// none means the pictures folder (`shot::folder`)
    pub screenshot_folder: Option<PathBuf>,
    /// a played folder plays its lone stills too, each held `still_seconds` while play runs (0.5 to 60); off, it plays
    /// its videos and sequences alone
    pub folder_stills: bool,
    pub still_seconds: f32,
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
            // gentle: a hand's jitter goes, a wave drawn on purpose stays (the owner's "not too much", 2026-11-25)
            pen_smoothing: 0.3,
            notes_beside: true,
            note_seconds: 3.0,
            copy_notes: true,
            browser_folder: None,
            browser_recent: Vec::new(),
            browser_view: crate::browse::View::Both,
            browser_sort: crate::browse::Sort::default(),
            browser_widths: crate::browse::Widths::default(),
            play_on_open: true,
            browser_opens_in: Opens::ThisTab,
            one_window: true,
            controls_after: 2.4,
            remember_size: true,
            update_check: true,
            update_checked: None,
            background: [0, 0, 0],
            browser_start: None,
            cache_limit: 2.0,
            window_size: None,
            screenshot_folder: None,
            folder_stills: false,
            still_seconds: 5.0,
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
    /// a check for updates is due: on, and none answered in the last day.
    pub fn update_due(&self, now: u64) -> bool {
        self.update_check && self.update_checked.is_none_or(|at| now.saturating_sub(at) >= 24 * 60 * 60)
    }

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
    fn pairs(&self) -> [(&'static str, String); 40] {
        let wxh = |(w, h): (u32, u32)| format!("{w}x{h}");
        let path = |p: &Option<PathBuf>| p.as_ref().map_or(String::new(), |p| p.to_string_lossy().into_owned());
        let recent: Vec<_> = self.browser_recent.iter().map(|p| p.to_string_lossy()).collect();
        let hex = |[r, g, b]: [u8; 3]| format!("#{r:02x}{g:02x}{b:02x}");
        [
            ("readout", word(&READOUT, self.timecode)),
            ("first_frame", self.first_frame.to_string()),
            ("loop_file", word(&LOOPS, self.loop_file)),
            ("loop_folder", word(&LOOPS, self.loop_folder)),
            ("loop_analysis_seconds", self.loop_analysis_seconds.to_string()),
            ("folder_stills", word(&SWITCH, self.folder_stills)),
            ("still_seconds", self.still_seconds.to_string()),
            ("volume", self.volume.to_string()),
            ("mute", word(&SWITCH, self.mute)),
            ("pin_controls", word(&SWITCH, self.pin_controls)),
            ("grid", word(&SWITCH, self.grid)),
            ("center", word(&SWITCH, self.center)),
            ("tile", wxh(self.tile)),
            ("guide_color", hex(self.guide_color)),
            ("chroma", word(&CHROMA, self.chroma)),
            ("background", hex(self.background)),
            ("screenshot_caption", word(&SWITCH, self.screenshot_caption)),
            ("screenshot_guides", word(&SWITCH, self.screenshot_guides)),
            ("screenshot_markup", word(&SWITCH, self.screenshot_markup)),
            ("pen_smoothing", self.pen_smoothing.to_string()),
            ("notes_beside", word(&SWITCH, self.notes_beside)),
            ("note_seconds", self.note_seconds.to_string()),
            ("copy_notes", word(&SWITCH, self.copy_notes)),
            ("browser_folder", path(&self.browser_folder)),
            ("browser_start", path(&self.browser_start)),
            ("browser_recent", recent.join(" | ")),
            ("browser_view", word(&VIEWS, self.browser_view)),
            ("browser_sort", format!("{} {}", word(&crate::browse::SORTS, self.browser_sort.by), word(&crate::browse::WAYS, self.browser_sort.descending))),
            // the sidebar, the columns as far as the last one dragged, the metadata column
            ("browser_widths", {
                let w = &self.browser_widths;
                let columns = w.columns.iter().rposition(|&c| c != crate::browse::COLUMN).map_or(0, |last| last + 1);
                std::iter::once(w.side).chain(w.columns[..columns].iter().copied()).chain([w.meta]).map(|n| n.to_string()).collect::<Vec<_>>().join(" ")
            }),
            ("browser_main_width", self.browser_widths.main.to_string()),
            ("browser_opens_in", word(&OPENS, self.browser_opens_in)),
            ("cache_limit", self.cache_limit.to_string()),
            ("play_on_open", word(&SWITCH, self.play_on_open)),
            ("one_window", word(&SWITCH, self.one_window)),
            ("controls_after", self.controls_after.to_string()),
            ("update_check", word(&SWITCH, self.update_check)),
            ("update_checked", self.update_checked.map_or(String::new(), |at| at.to_string())),
            ("remember_size", word(&SWITCH, self.remember_size)),
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
            "folder_stills" => self.folder_stills = pick(&SWITCH, v)?,
            "still_seconds" => self.still_seconds = number(v, 0.5, 60.0)?,
            "volume" => self.volume = number(v, 0.0, 1.0)?,
            "mute" => self.mute = pick(&SWITCH, v)?,
            "pin_controls" => self.pin_controls = pick(&SWITCH, v)?,
            "grid" => self.grid = pick(&SWITCH, v)?,
            "center" => self.center = pick(&SWITCH, v)?,
            "tile" => self.tile = size(v)?,
            "guide_color" => self.guide_color = color(v)?,
            "chroma" => self.chroma = pick(&CHROMA, v)?,
            "background" => self.background = color(v)?,
            "screenshot_caption" => self.screenshot_caption = pick(&SWITCH, v)?,
            "screenshot_guides" => self.screenshot_guides = pick(&SWITCH, v)?,
            "screenshot_markup" => self.screenshot_markup = pick(&SWITCH, v)?,
            "pen_smoothing" => self.pen_smoothing = number(v, 0.0, 1.0)?,
            "notes_beside" => self.notes_beside = pick(&SWITCH, v)?,
            "note_seconds" => self.note_seconds = number(v, 0.5, 60.0)?,
            "copy_notes" => self.copy_notes = pick(&SWITCH, v)?,
            "browser_folder" => self.browser_folder = folder(v),
            "browser_start" => self.browser_start = folder(v),
            // a bar cannot be in a windows path
            "browser_recent" => self.browser_recent = v.split('|').filter_map(|p| folder(p.trim())).collect(),
            "browser_view" => self.browser_view = pick(&VIEWS, v)?,
            // what it is sorted by, then which way: "date modified descending". a date this system does not keep reads
            // as nothing
            "browser_sort" => {
                let (by, way) = v.rsplit_once(' ')?;
                self.browser_sort = crate::browse::Sort { by: pick(&crate::browse::SORTS, by).filter(|by| by.kept())?, descending: pick(&crate::browse::WAYS, way)? };
            }
            "browser_widths" => {
                // the sidebar, then each column's, then the metadata column's. a column not named has the design's width
                let n: Vec<f32> = v.split_whitespace().map(|n| n.parse().ok()).collect::<Option<_>>()?;
                let (&side, rest) = n.split_first()?;
                let (&meta, columns) = rest.split_last()?;
                let mut widths = crate::browse::Widths { side, meta, main: self.browser_widths.main, ..Default::default() };
                for (depth, &width) in columns.iter().enumerate().take(crate::browse::COLUMNS) {
                    widths.set_column(depth, width);
                }
                self.browser_widths = widths.kept();
            }
            // a line of its own, as the columns between the sidebar's width and the metadata column's are any number
            "browser_main_width" => self.browser_widths.main = number(v, crate::browse::MAIN_LEAST, f32::MAX)?,
            "browser_opens_in" => self.browser_opens_in = pick(&OPENS, v)?,
            "cache_limit" => self.cache_limit = number(v, 1.0, 64.0)?,
            "play_on_open" => self.play_on_open = pick(&SWITCH, v)?,
            "one_window" => self.one_window = pick(&SWITCH, v)?,
            "controls_after" => self.controls_after = number(v, 1.0, 10.0)?,
            "update_check" => self.update_check = pick(&SWITCH, v)?,
            "update_checked" => self.update_checked = if v.is_empty() { None } else { Some(v.parse().ok()?) },
            "remember_size" => self.remember_size = pick(&SWITCH, v)?,
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
const OPENS: [(&str, Opens); 2] = [("this tab", Opens::ThisTab), ("a new tab", Opens::NewTab)];

fn word<T: Copy + PartialEq>(words: &[(&str, T)], value: T) -> String {
    words.iter().find(|w| w.1 == value).map_or("", |w| w.0).to_string()
}

fn pick<T: Copy>(words: &[(&str, T)], v: &str) -> Option<T> {
    words.iter().find(|w| w.0 == v).map(|w| w.1)
}

/// `settings.txt` in the system's settings folder: %APPDATA%\eyepiece on windows
pub fn file() -> Option<PathBuf> {
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
pub fn color(v: &str) -> Option<[u8; 3]> {
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
            pen_smoothing: 0.75,
            notes_beside: false,
            note_seconds: 1.5,
            copy_notes: false,
            browser_folder: Some(PathBuf::from(r"D:\delivery 2026-11-20\wall")),
            browser_recent: vec![PathBuf::from(r"D:\delivery 2026-11-20\wall"), PathBuf::from(r"\\nas\renders = out\totems")],
            browser_view: crate::browse::View::List,
            browser_sort: crate::browse::Sort { by: crate::browse::By::Size, descending: true },
            browser_widths: { let mut w = crate::browse::Widths { side: 180.0, main: 96.5, meta: 300.0, ..Default::default() }; w.set_column(2, 200.5); w },
            play_on_open: false,
            browser_opens_in: Opens::NewTab,
            one_window: false,
            controls_after: 6.5,
            remember_size: false,
            update_check: false,
            update_checked: Some(1_795_000_000),
            background: [0x1e, 0x28, 0x33],
            browser_start: Some(PathBuf::from(r"D:\renders_out")),
            cache_limit: 16.0,
            window_size: Some((1280, 720)),
            screenshot_folder: Some(PathBuf::from(r"\\nas\review = notes")),
            folder_stills: true,
            still_seconds: 2.5,
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
        let by_hand = "\u{feff}volume = 0.5\r\nfrom_a_newer_build = 42\r\nthis line is broken\r\nmute = maybe\r\ntile = 0x32\r\nguide_color=#FF8800\r\nbrowser_sort = date added descending\r\nstill_seconds = 90\r\n";
        fs::write(&file, by_hand).unwrap();
        let loaded = Settings::load_from(&file);
        let defaults = Settings::default();
        assert_eq!((loaded.volume, loaded.guide_color), (0.5, [0xff, 0x88, 0x00]));
        assert_eq!((loaded.mute, loaded.tile), (defaults.mute, defaults.tile));
        // a still is held half a second to a minute: 90 does not read
        assert_eq!((loaded.still_seconds, defaults.still_seconds, defaults.folder_stills), (5.0, 5.0, false));
        // windows keeps no date added: there the sort stays as it was
        let added = crate::browse::Sort { by: crate::browse::By::Added, descending: true };
        assert_eq!(loaded.browser_sort, if cfg!(target_os = "macos") { added } else { defaults.browser_sort });
        loaded.save_to(&file).unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("\nfrom_a_newer_build = 42\n"), "{text}");
        assert!(text.contains("\nmute = off\n") && !text.contains("broken"), "{text}");
        assert_eq!(Settings::load_from(&file), loaded);
        tidy(&dir);
    }
}
