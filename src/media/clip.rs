//! one open clip: what the container says about it, its exact frame times, and where its frames come from.

use super::dec::Hardware;
use super::ff::{self, Input, Sample, Video};
use super::hap_source::HapLane;
use super::meta;
use super::seq_source::SeqLane;
use super::still;
use super::timeline::Timeline;
use super::video_source::{Times, VideoLane};
use crate::browse::{self, Kind};
use crate::timecode::Timecode;
use crate::view::Look;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub struct Clip {
    /// the file's name without its folder; a sequence's as the browser shows it, name.[0001-0240].exr
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// what to call the codec, as a person would
    pub codec: &'static str,
    pub timeline: Timeline,
    pub lane: Lane,
    /// the frame to show first: a sequence opened by one of its files opens on that file's frame
    pub first_frame: usize,
    /// a sequence's frames that have no file, in order
    pub missing: Vec<usize>,
    /// where the rate the timeline runs at comes from
    pub rate_from: RateFrom,
    /// a sequence's rate as its files state it (exr), when they do
    pub own_rate: Option<(i64, i64)>,
    /// what the frames are called by the clock: the file's own timecode, else counted from 00:00:00:00. none for a
    /// clip with no one rate to count by
    pub clock: Option<Timecode>,
    /// how the file asks the picture to be turned for the screen, as a phone held upright tags the picture it stores
    /// on its side. what the user turns comes on top of it (`Look::then`)
    pub turn: Look,
}

/// where a clip's frame rate comes from, as the bar, the title and the metadata panel say it
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateFrom {
    /// the file's own
    File,
    /// a sequence whose files state none: the setting's, never chosen for it
    Assumed,
    /// chosen for this sequence on the bar
    Set,
}

/// the rate a sequence whose files state none opens at: the setting's (`set_sequence_rate`), 24 until it is read
static SEQUENCE_RATE: Mutex<(i64, i64)> = Mutex::new((24, 1));

/// the rate for sequences whose files state none, from the settings.
pub fn set_sequence_rate(rate: (i64, i64)) {
    *SEQUENCE_RATE.lock().unwrap_or_else(|e| e.into_inner()) = rate;
}

pub fn sequence_rate() -> (i64, i64) {
    *SEQUENCE_RATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// the rates offered for a sequence, as the bar's menu and the settings name them
pub const RATES: [(&str, (i64, i64)); 8] = [("23.976", (24000, 1001)), ("24", (24, 1)), ("25", (25, 1)), ("29.97", (30000, 1001)), ("30", (30, 1)), ("50", (50, 1)), ("59.94", (60000, 1001)), ("60", (60, 1))];

/// a rate typed for a sequence: a number, decimals allowed (12, 47.952), or a ratio (30000/1001), 1 to 240 frames a
/// second. a decimal that rounds a rate of the /1001 kind (23.976, 29.97, 59.94, 47.952) is that exact ratio.
pub fn typed_rate(text: &str) -> Result<(i64, i64), String> {
    let text = text.trim();
    let unread = || "not a rate: type a number, such as 12 or 47.952, or a ratio, such as 30000/1001.".to_string();
    let whole = |digits: &str| Some(digits).filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())).and_then(|d| d.parse::<i64>().ok());
    let (n, d) = match text.split_once('/') {
        Some((n, d)) => (whole(n).ok_or_else(unread)?, whole(d).filter(|d| *d > 0).ok_or_else(unread)?),
        None => {
            let (int, frac) = text.split_once('.').unwrap_or((text, ""));
            let scale = 10i64.checked_pow(frac.len() as u32).ok_or_else(unread)?;
            let n = whole(&format!("{int}{frac}")).filter(|_| !int.is_empty() || !frac.is_empty()).ok_or_else(unread)?;
            // 29.97 is how 30000/1001 is written: a decimal of two places or more that rounds a whole rate over 1.001
            let ntsc = (n as f64 / scale as f64 * 1.001).round() as i64;
            if frac.len() >= 2 && n % scale != 0 && (ntsc as f64 * 1000.0 / 1001.0 * scale as f64).round() as i64 == n {
                (ntsc.checked_mul(1000).ok_or_else(unread)?, 1001)
            } else {
                (n, scale)
            }
        }
    };
    if !(d as i128..=240 * d as i128).contains(&(n as i128)) {
        return Err(format!("{text} fps is outside 1 to 240."));
    }
    let common = gcd(n, d);
    Ok((n / common, d / common))
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// a rate as its decimal, to three places at most: 24, 23.976, 12.5.
pub fn rate_name((n, d): (i64, i64)) -> String {
    meta::decimal(n as f64 / d as f64, 3)
}

/// a rate as people say it (the owner's word, 2026-11-27): its decimal, 30 or 29.97, with its ratio after it only where
/// the decimal is not exactly that rate. 15360/512 is 30, and 29.97 is 30000/1001, but 2997/100 is not that 29.97.
pub fn rate_said((n, d): (i64, i64)) -> String {
    let common = gcd(n, d).max(1);
    let (n, d) = (n / common, d / common);
    let name = rate_name((n, d));
    if d == 1 || typed_rate(&name) == Ok((n, d)) { name } else { format!("{name} · {n}/{d}") }
}

/// how a stream asks to be turned, when that is a quarter turn; otherwise it is shown as stored
fn turn(input: &Input, video: &Video) -> Look {
    input.turn(video.index).and_then(|(degrees, mirror)| Look::upright(degrees, mirror)).unwrap_or_default()
}

/// a clip's clock: its own timecode when the file carries one that reads (`own`: as written, and its track's frame
/// numbers a second when the track says), else counted from nothing. none without one rate to count by.
fn clock(timeline: &Timeline, own: Option<(String, Option<u32>)>) -> Option<Timecode> {
    let rate = timeline.rate().map(|(n, d)| (n as f64 / d as f64).round() as u32).filter(|fps| *fps > 0)?;
    Some(own_clock(timeline, own).unwrap_or(Timecode::counted(rate)))
}

/// the file's own timecode as a clock, when it carries one that reads and the clip has one rate to count by.
pub(super) fn own_clock(timeline: &Timeline, own: Option<(String, Option<u32>)>) -> Option<Timecode> {
    let rate = timeline.rate().map(|(n, d)| (n as f64 / d as f64).round() as u32).filter(|fps| *fps > 0)?;
    own.and_then(|(text, base)| Timecode::parse(&text, base.unwrap_or(rate), rate))
}

/// where a clip's frames come from.
pub enum Lane {
    /// our own hap code: blocks as stored
    Hap(HapLane),
    /// a clip ffmpeg decodes, not started: it waits for the renderer's device, which a hardware decoder writes into
    Waiting(Waiting),
    /// a clip ffmpeg decodes, running
    Video(VideoLane),
    /// stills: a numbered run of image files, or one alone
    Stills(SeqLane),
}

/// a numbered run of stills, as a clip needs it
pub(super) struct Run {
    pub(super) name: String,
    pub(super) frames: usize,
    /// the frame of the file the run was found from
    pub(super) at: usize,
    pub(super) missing: Vec<usize>,
    path_of: Box<dyn Fn(usize) -> String + Send + Sync>,
}

/// the file's name without its folder.
fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

/// the numbered run in the file's folder that the file is a frame of, from the folder's listing alone: what the
/// listing lacks is missing, and no other file is opened.
pub(super) fn run_of(file: &Path) -> Option<Run> {
    let name = file.file_name()?.to_str()?;
    let (before, n, ext) = browse::frame_number(name)?;
    // a bare name is in the folder the program runs in
    let folder = file.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    browse::list(folder).ok()?.into_iter().find_map(|entry| {
        let Kind::Sequence { first, last, missing, pad } = entry.kind else { return None };
        // the run's own spelling of the name, which a path typed by hand may not match in case
        let (b, _, x) = browse::frame_number(entry.path.file_name()?.to_str()?)?;
        // runs can share a stem and differ in padding (a.1.exr, a.01.exr): the file's own name has to come back
        let own = b.eq_ignore_ascii_case(before) && x.eq_ignore_ascii_case(ext) && format!("{b}{n:0pad$}{x}").eq_ignore_ascii_case(name);
        if !own || !(first..=last).contains(&n) || missing.binary_search(&n).is_ok() {
            return None;
        }
        let (folder, b, x) = (folder.to_path_buf(), b.to_string(), x.to_string());
        Some(Run {
            name: entry.name,
            frames: (last - first + 1) as usize,
            at: (n - first) as usize,
            missing: missing.iter().map(|m| (m - first) as usize).collect(),
            path_of: Box::new(move |i| folder.join(format!("{b}{:0pad$}{x}", first + i as u64)).to_string_lossy().into_owned()),
        })
    })
}

pub struct Waiting {
    path: String,
    stream: usize,
    samples: Vec<Sample>,
    times: Times,
    size: (usize, usize),
    /// decoded pictures are kept for a step back (`Clip::keep_nothing`)
    keep: bool,
}

/// hap's variants by their container tag. ffmpeg knows only the first three; the rest it reports as no codec at all.
pub(super) fn hap_variant(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "Hap1" => "hap",
        "Hap5" => "hap alpha",
        "HapY" => "hap q",
        "HapM" => "hap q alpha",
        "HapA" => "hap alpha-only",
        "Hap7" => "hap r",
        "HapH" => "hap hdr",
        _ => return None,
    })
}

/// what to call a codec that plays, as a person would. None for one that does not play yet.
pub(super) fn codec_name(video: &Video) -> Option<&'static str> {
    hap_variant(&ff::fourcc(video.tag)).or(match video.codec_name.as_str() {
        "h264" => Some("h.264"),
        "hevc" => Some("hevc"),
        "prores" => Some("prores"),
        "notchlc" => Some("notchlc"),
        "vp9" => Some("vp9"),
        _ => None,
    })
}

/// a clip's stored samples in the order they are read and, when the container's sample table cannot say them, the
/// times its frames are shown. mov and mp4 list every sample in the moov, but a fragmented mp4 lists none or only its
/// first there, and ffmpeg's table of the rest can stop short; mkv lists only key frames to seek to. those two, and a
/// table whose decode times are not evenly spaced (with b-frames these do not say when frames are shown), are counted
/// by the packet pass (`Input::count`), on a reader of its own. other containers without a table are not counted:
/// their seeking is not by key frame.
pub(super) fn stored(path: &str, input: &Input, video: &Video, count: &Count) -> Result<(Vec<Sample>, Option<Vec<i64>>), String> {
    let samples = input.samples(video.index);
    let format = input.format_name();
    let (mov, mkv) = (format.starts_with("mov,"), format == "matroska,webm");
    let whole = !mkv && (!mov || input.listed(video.index) == samples.len() as i64);
    // hap frames stand alone, so a decode time is when the frame is shown; evenly spaced decode times are as evenly
    // spaced as the frames shown
    let step = |p: &[Sample]| p[1].time - p[0].time;
    let shown_by_table = hap_variant(&ff::fourcc(video.tag)).is_some() || samples.len() < 2 || (step(&samples) > 0 && samples.windows(2).all(|p| step(p) == step(&samples)));
    if (whole && shown_by_table) || !(mov || mkv) {
        return Ok((samples, None));
    }
    // counted once in a run: the file is known after that by its path, size and time
    let stamp = std::fs::metadata(path).map(|m| (path.to_string(), m.len(), m.modified().ok())).map_err(|e| format!("{path}: {e}"))?;
    let known = KNOWN.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(of, _)| *of == stamp).map(|(_, found)| found.clone());
    if let Some(found) = known {
        return Ok((found.0.clone(), Some(found.1.clone())));
    }
    let Count::Read { counted, stop } = count else { return Err(format!("{path}: {UNCOUNTED}")) };
    let found = Arc::new(Input::open(path)?.count(video.index, counted, stop)?);
    let mut known = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    // a few files' worth: the clip on screen, its neighbors, the ones just looked at
    if known.len() >= KEPT {
        known.remove(0);
    }
    known.push((stamp, found.clone()));
    Ok((found.0.clone(), Some(found.1.clone())))
}

/// what the packet pass found, for the files this run has counted last: each by its path, size and modified time
type Counted = (Vec<Sample>, Vec<i64>);
static KNOWN: Mutex<Vec<((String, u64, Option<SystemTime>), Arc<Counted>)>> = Mutex::new(Vec::new());
const KEPT: usize = 16;

/// whether a file that lists no frames up front may be read through to count them.
pub enum Count<'a> {
    /// yes, for a file that is opened to play: `counted` follows how far the count has got, and setting `stop`
    /// ends it
    Read { counted: &'a AtomicUsize, stop: &'a AtomicBool },
    /// only if it has been counted in this run already: a card's picture or a line of metadata is not worth
    /// reading a whole file for
    Known,
}

/// why a file is not opened when it would have to be counted first and may not be
pub const UNCOUNTED: &str = "its frames are not listed up front, and counting them reads the whole file. opening it does that.";

/// a clip's frames: the samples in the order they are read, the times of the frames shown, and how long every frame
/// lasts when they all last the same. `shown` is the times the packet pass found, when it ran.
pub(super) fn frames(video: &Video, mut samples: Vec<Sample>, shown: Option<Vec<i64>>) -> (Vec<Sample>, Timeline, Option<i64>) {
    if let Some(times) = shown {
        let timeline = Timeline { time_base: video.time_base, times };
        if hap_variant(&ff::fourcc(video.tag)).is_some() {
            samples.retain(|s| !s.hidden);
            samples.sort_by_key(|s| s.time);
        }
        let step = timeline.step();
        return (samples, timeline, step);
    }
    if hap_variant(&ff::fourcc(video.tag)).is_some() {
        // every hap frame stands alone, so file order is free to differ from time order: time order is what plays.
        // what an edit list hides is not part of the clip
        samples.retain(|s| !s.hidden);
        samples.sort_by_key(|s| s.time);
        let timeline = Timeline::new(video.time_base, &samples);
        let step = timeline.step();
        return (samples, timeline, step);
    }
    // the samples stay in decode order. their decode times are evenly spaced exactly when the frames are
    let step = Timeline::new(video.time_base, &samples).step();
    // the clip's frames are the samples that are shown: what an edit list hides is decoded and never counted
    let shown: Vec<Sample> = samples.iter().filter(|s| !s.hidden).copied().collect();
    let timeline = match step {
        Some(step) => Timeline::even(video.time_base, shown.len(), step),
        None => Timeline::new(video.time_base, &shown),
    };
    (samples, timeline, step)
}

impl Clip {
    /// reads the header. a hap clip or stills start reading at once; any other waits for `start`. (the tests' way
    /// in: the player opens with `open_counting` or `open_known`.)
    #[cfg(test)]
    pub fn open(path: &str) -> Result<Clip, String> {
        Clip::open_counting(path, &AtomicUsize::new(0), &AtomicBool::new(false))
    }

    /// `open`, for a worker: while the packet pass reads a file that lists no frames up front, `counted` follows how
    /// many it has counted so far, and setting `stop` ends it.
    pub fn open_counting(path: &str, counted: &AtomicUsize, stop: &AtomicBool) -> Result<Clip, String> {
        Clip::open_with(path, &Count::Read { counted, stop })
    }

    /// `open`, for the browser's pictures and the like: a file that would have to be counted first is refused,
    /// unless this run has counted it already.
    pub fn open_known(path: &str) -> Result<Clip, String> {
        Clip::open_with(path, &Count::Known)
    }

    fn open_with(path: &str, count: &Count) -> Result<Clip, String> {
        if still::is_still(path) {
            return Clip::open_stills(path, sequence_rate());
        }
        let input = Input::open(path)?;
        let video = input.first_video().ok_or_else(|| format!("{path}: no picture in this file"))?;
        let tag = ff::fourcc(video.tag);
        let name = file_name(path);
        let codec = codec_name(&video).ok_or_else(|| format!("{path}: {} ({tag}) does not play yet. this build plays hap, notchlc, prores, h.264, hevc and vp9.", video.codec_name))?;
        let (samples, shown) = stored(path, &input, &video, count)?;
        let counted = shown.is_some();
        let (samples, timeline, step) = frames(&video, samples, shown);
        let lane = if hap_variant(&tag).is_some() {
            Lane::Hap(HapLane::open(path, samples, video.width, video.height)?)
        } else {
            let times = match (step, samples.len()) {
                (Some(step), _) => Times::Even(step),
                // a container that lists no frames and is not counted (mpeg-ts, for one)
                (None, 0) => return Err(format!("{path}: this kind of file does not list its frames up front. it does not play yet.")),
                (None, 1) => Times::Even(1),
                (None, _) if counted => Times::Shown(timeline.times.clone()),
                // a table of uneven decode times in a container that is not counted gives no times to show frames by
                (None, _) => return Err(format!("{path}: its frames do not all last the same. variable frame rates play only from mov, mp4 and mkv.")),
            };
            let size = (video.width, video.height);
            // on macos the system decodes prores 422 three to five times faster than software does (phase 0), so
            // it waits for the device like the rest. 4444 stays in software there: with alpha the system's chroma
            // came back wrong
            let system_prores = cfg!(target_os = "macos") && !matches!(tag.as_str(), "ap4h" | "ap4x");
            if codec == "notchlc" || (codec == "prores" && !system_prores) {
                // these decode in software here, so they need not wait for the graphics device: the first frame is
                // being decoded while the device is made, which takes longer than anything else in a start
                Lane::Video(VideoLane::open(path, video.index, samples, times, size, None, true)?)
            } else {
                Lane::Waiting(Waiting { path: path.to_string(), stream: video.index, samples, times, size, keep: true })
            }
        };
        let clock = clock(&timeline, meta::timecode_tag(&input, video.index).map(|text| (text, input.timecode_base())));
        Ok(Clip { name, width: video.width, height: video.height, codec, timeline, lane, first_frame: 0, missing: Vec::new(), rate_from: RateFrom::File, own_rate: None, clock, turn: turn(&input, &video) })
    }

    /// a still, or the numbered run of stills it is a frame of, opened on its frame. the file's own picture gives the
    /// size and codec, and is the first frame shown. a run runs at its files' own rate when they state one (exr), else
    /// at `assumed`.
    fn open_stills(path: &str, assumed: (i64, i64)) -> Result<Clip, String> {
        let image = Arc::new(still::decode(path)?);
        let (width, height, codec, rate) = (image.width, image.height, image.codec, image.rate);
        let alone = path.to_string();
        let run = run_of(Path::new(path)).unwrap_or_else(|| Run { name: file_name(path), frames: 1, at: 0, missing: Vec::new(), path_of: Box::new(move |_| alone.clone()) });
        let rate_from = if rate.is_none() && run.frames > 1 { RateFrom::Assumed } else { RateFrom::File };
        let (num, den) = rate.unwrap_or(assumed);
        let timeline = Timeline::even((den, num), run.frames, 1);
        let lane = SeqLane::open(run.missing.clone(), (run.at, image), run.path_of)?;
        let clock = clock(&timeline, None);
        Ok(Clip { name: run.name, width, height, codec, timeline, lane: Lane::Stills(lane), first_frame: run.at, missing: run.missing, rate_from, own_rate: rate, clock, turn: Look::default() })
    }

    /// an image sequence at another rate, which `from` says where it came from: its frames stay as they are, and its
    /// clock counts at the new rate. a video, or a still alone, keeps its own.
    pub fn set_rate(&mut self, (num, den): (i64, i64), from: RateFrom) {
        if !matches!(self.lane, Lane::Stills(_)) || self.timeline.frames() < 2 {
            return;
        }
        self.timeline = Timeline::even((den, num), self.timeline.frames(), 1);
        self.clock = clock(&self.timeline, None);
        self.rate_from = from;
    }

    /// the picture's size upright, as the file asks it shown
    pub fn upright(&self) -> (usize, usize) {
        self.turn.shown((self.width, self.height))
    }

    /// starts decoding, for a clip that was waiting. `hardware` is the renderer's device, when there is one.
    pub fn start(&mut self, hardware: Option<Hardware>) -> Result<(), String> {
        if let Lane::Waiting(w) = &mut self.lane {
            let times = std::mem::replace(&mut w.times, Times::Even(1));
            let lane = VideoLane::open(&w.path, w.stream, std::mem::take(&mut w.samples), times, w.size, hardware, w.keep)?;
            self.lane = Lane::Video(lane);
        }
        Ok(())
    }

    /// for a reader that never steps back (a card's pictures, the timebar's): its decoder, once started, keeps no
    /// pictures behind it, so a hardware decoder is asked for that many fewer surfaces: 21 fewer at 4K, each 25 MB at
    /// 10 bits. a clip that does not wait to start (prores, notchlc) is as it was.
    pub fn keep_nothing(&mut self) {
        if let Lane::Waiting(w) = &mut self.lane {
            w.keep = false;
        }
    }

    /// the frame rate as text: the file's own ratio, one derived from rounded times, one assumed or set for a sequence,
    /// or that it has none.
    pub fn rate_text(&self) -> String {
        rate_words(&self.timeline, self.rate_from)
    }
}

fn rate_words(timeline: &Timeline, from: RateFrom) -> String {
    let text = match (timeline.rate(), timeline.derived()) {
        (Some(rate), _) => format!("{} fps", rate_said(rate)),
        (None, Some(fps)) => format!("{} fps derived", meta::decimal(fps, 3)),
        (None, None) if timeline.frames() < 2 => "still".to_string(),
        (None, None) => "variable rate".to_string(),
    };
    match from {
        RateFrom::File => text,
        RateFrom::Assumed => format!("{text} assumed"),
        RateFrom::Set => format!("{text} set"),
    }
}

/// what a played folder's list says of one of its clips: its size upright, and its frames and rate, which a file that
/// would have to be read through to count them does not give.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub width: usize,
    pub height: usize,
    pub frames: Option<usize>,
    pub rate: Option<String>,
    /// frames a second, for telling whether two clips run at one rate however their files spell it
    pub fps: Option<f64>,
}

impl Facts {
    /// true when both clips state a size and the sizes differ.
    pub fn other_size(&self, than: &Facts) -> bool {
        (self.width, self.height) != (than.width, than.height)
    }

    /// true when both clips state a rate and the rates differ: 15360/256 and 60000/1000 are one rate, 60000/1001
    /// is another.
    pub fn other_rate(&self, than: &Facts) -> bool {
        matches!((self.fps, than.fps), (Some(a), Some(b)) if (a - b).abs() > 1e-9 * a.max(b))
    }
}

/// a clip's facts from its header alone, without opening it to play: no reader is started and no frame is read. a
/// still is decoded for its size, and a sequence's frames come from its folder's listing.
pub fn facts(path: &str) -> Result<Facts, String> {
    if still::is_still(path) {
        let image = still::decode(path)?;
        let frames = run_of(Path::new(path)).map_or(1, |run| run.frames);
        let (num, den) = image.rate.unwrap_or_else(sequence_rate);
        let rate = rate_words(&Timeline::even((den, num), frames, 1), if image.rate.is_none() && frames > 1 { RateFrom::Assumed } else { RateFrom::File });
        return Ok(Facts { width: image.width, height: image.height, frames: Some(frames), rate: Some(rate), fps: (frames > 1).then(|| num as f64 / den as f64) });
    }
    let input = Input::open(path)?;
    let video = input.first_video().ok_or_else(|| format!("{path}: no picture in this file"))?;
    let timeline = stored(path, &input, &video, &Count::Known).ok().map(|(samples, shown)| frames(&video, samples, shown).1);
    let fps = timeline.as_ref().filter(|t| t.frames() > 1).map(Timeline::fps);
    let (width, height) = turn(&input, &video).shown((video.width, video.height));
    Ok(Facts { width, height, frames: timeline.as_ref().map(Timeline::frames), rate: timeline.as_ref().map(|t| rate_words(t, RateFrom::File)), fps })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a file's own timecode is the clip's clock, drop-frame and counted as its track counts; a file without one is
    /// counted from nothing at its own rate.
    #[test]
    fn a_clips_clock_is_the_files_own_timecode() {
        let own = Clip::open(&media("av_tc_64.mov")).unwrap();
        assert_eq!(own.clock, Some(Timecode { start: 107_892, base: 30, drop: true, rate: 30 }));
        assert_eq!(Input::open(&media("av_tc_64.mov")).unwrap().timecode_base(), Some(30));
        let none = Clip::open(&media("hap_64.mov")).unwrap();
        assert_eq!(none.clock, Some(Timecode::counted(none.timeline.fps().round() as u32)));
        assert_eq!(Input::open(&media("hap_64.mov")).unwrap().timecode_base(), None);
    }
    use crate::media::dec::Picture;
    use crate::media::source::{Frame, Source};
    use crate::media::still::Samples;
    use crate::play::Transport;
    use std::time::{Duration, Instant};

    fn media(name: &str) -> String {
        format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn a_clip_reports_what_the_file_says() {
        let c = Clip::open(&media("hapq_64.mov")).expect("open");
        assert_eq!((c.name.as_str(), c.width, c.height, c.codec, c.timeline.frames()), ("hapq_64.mov", 64, 64, "hap q", 3));
        // the rate is the file's own ratio, not a rounded 59.94
        assert_eq!(c.rate_text(), "59.94 fps");
        let c = Clip::open(&media("hapalpha_64.mov")).expect("open");
        assert_eq!((c.codec, c.timeline.frames()), ("hap alpha", 2));
        let c = Clip::open(&media("h264_64.mp4")).expect("open");
        assert_eq!((c.codec, c.timeline.frames(), c.rate_text().as_str()), ("h.264", 12, "30 fps"));
        let c = Clip::open(&media("prores_64.mov")).expect("open");
        assert_eq!((c.codec, c.timeline.frames()), ("prores", 4));
    }

    /// a file tagged to be turned is turned as ffmpeg's own player turns it, measured (2026-11-24) with its autorotate
    /// on clips tagged by -display_rotation, -display_hflip and -display_vflip: each matrix as ffmpeg writes it, then
    /// the quarter turns clockwise and the mirror first that it makes of it. the test clips are read for their tag
    #[test]
    fn a_clip_is_turned_as_its_file_asks() {
        let matrix = |a: i32, b: i32, c: i32, d: i32| ff::rotation([a << 16, b << 16, 0, c << 16, d << 16, 0, 0, 0, 1 << 30]);
        let made = |(degrees, mirror)| Look::upright(degrees, mirror).map(|look| (look.turns, look.mirror));
        // rotation 90, 180 and 270, then 90 with hflip, then vflip alone
        let asked = [matrix(0, -1, 1, 0), matrix(-1, 0, 0, -1), matrix(0, 1, -1, 0), matrix(0, -1, -1, 0), matrix(1, 0, 0, -1)];
        assert_eq!(asked.map(made), [(3, false), (2, false), (1, false), (1, true), (2, true)].map(Some));
        for (name, turns, upright) in [("h264_320.mp4", 0, (320, 180)), ("h264_320_turn90.mp4", 1, (180, 320)), ("h264_320_turn180.mp4", 2, (320, 180)), ("h264_320_turn270.mp4", 3, (180, 320))] {
            let c = Clip::open(&media(name)).expect("open");
            assert_eq!((c.turn, (c.width, c.height), c.upright()), (Look { turns, ..Look::default() }, (320, 180), upright), "{name}");
        }
    }

    #[test]
    fn hap_frames_arrive_and_the_last_is_reachable() {
        let c = Clip::open(&media("hap_64.mov")).expect("open");
        let Lane::Hap(hap) = &c.lane else { panic!("not the hap lane") };
        for frame in [0, 2, 1] {
            hap.source.want(&[frame]);
            match hap.source.get(frame, Duration::from_secs(5)) {
                Frame::Ready(blocks) => assert_eq!(blocks.len(), 64 / 4 * (64 / 4) * 8),
                Frame::Pending => panic!("frame {frame} never arrived"),
                Frame::Failed(why) => panic!("frame {frame}: {why}"),
            }
        }
    }

    /// a number that changes when a picture's luma changes.
    fn luma(picture: &Picture, width: usize) -> u64 {
        let layout = picture.layout().expect("layout");
        let (data, stride, rows) = picture.plane(0, layout.chroma_shift);
        let row_bytes = width * (layout.stored.texel_bits as usize / 8);
        (0..rows).flat_map(|r| &data[r * stride..r * stride + row_bytes]).fold(0xcbf29ce484222325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100000001b3))
    }

    /// the frames of a clip, asked for in `order`, decoded in software.
    fn frames(name: &str, order: &[usize]) -> Vec<u64> {
        let mut c = Clip::open(&media(name)).expect("open");
        c.start(None).unwrap_or_else(|e| panic!("{name}: {e}"));
        let Lane::Video(video) = &c.lane else { panic!("not the video lane") };
        assert!(!video.layout.hardware);
        order
            .iter()
            .map(|&frame| {
                video.source.want(&[frame]);
                match video.source.get(frame, Duration::from_secs(10)) {
                    Frame::Ready(picture) => luma(&picture, c.width),
                    Frame::Pending => panic!("{name}: frame {frame} never arrived"),
                    Frame::Failed(why) => panic!("{name}: frame {frame}: {why}"),
                }
            })
            .collect()
    }

    /// mkv and webm list no frames up front, and a fragmented mp4's table can stop short (this one's at its first
    /// fragment, 4 of 12): their packets are counted. mkv keeps time in milliseconds, which cannot hold a 30th of a
    /// second, so its rate is derived from its frames' times, and says so.
    #[test]
    fn files_that_do_not_list_all_their_frames_are_counted() {
        // what the header alone gives: no table in mkv, the first fragment's samples in the fragmented mp4
        for (name, listed) in [("h264_64.mkv", 0), ("h264_frag_64.mp4", 4)] {
            let input = Input::open(&media(name)).unwrap();
            assert_eq!(input.samples(input.first_video().unwrap().index).len(), listed, "{name}");
        }
        for (name, codec, rate) in [("h264_64.mkv", "h.264", "30 fps derived"), ("vp9_64.webm", "vp9", "30 fps derived"), ("h264_frag_64.mp4", "h.264", "30 fps")] {
            let c = Clip::open(&media(name)).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!((c.codec, c.timeline.frames(), c.rate_text().as_str()), (codec, 12, rate), "{name}");
        }
    }

    /// a counted sample's time is when it is shown, which mov's and mkv's seeking go by: a seek to a key frame lands
    /// on it, not on the one before. here key frames are every fourth, with b-frames
    #[test]
    fn counted_key_frames_carry_the_time_they_are_shown() {
        for (name, keys) in [("h264_64.mkv", [0, 133, 267]), ("h264_frag_64.mp4", [1024, 3072, 5120])] {
            let input = Input::open(&media(name)).unwrap();
            let video = input.first_video().unwrap();
            let (samples, _) = stored(&media(name), &input, &video, &Count::Read { counted: &AtomicUsize::new(0), stop: &AtomicBool::new(false) }).unwrap();
            let times: Vec<i64> = samples.iter().filter(|s| s.key).map(|s| s.time).collect();
            assert_eq!(times, keys, "{name}");
        }
    }

    /// a mov or mp4 that lists its samples in the moov is read from that table alone: the packet pass, a read of the
    /// whole file, never runs for it
    #[test]
    fn a_table_in_the_moov_is_not_counted() {
        let tables = ["hap_64.mov", "hapq_64.mov", "hapalpha_64.mov", "prores_64.mov", "prores4444_64.mov", "av_tc_64.mov", "tone48_stereo.mov", "tone44_mono.mov"];
        let mp4s = ["h264_64.mp4", "h264_320.mp4", "h264_cut_320.mp4", "h264_422_64.mp4", "h264_709_64.mp4", "h264_720.mp4", "h264_vui709_64.mp4", "hevc10_64.mp4", "hevc10_320.mp4"];
        for name in tables.iter().chain(&mp4s) {
            let input = Input::open(&media(name)).unwrap();
            let video = input.first_video().unwrap();
            let (_, shown) = stored(&media(name), &input, &video, &Count::Known).unwrap();
            assert!(shown.is_none(), "{name} was counted");
        }
    }

    /// a file that lists no frames is read through only when it is opened to play, and then once: after that this
    /// run knows its frames, whoever opens it next. (each kind is its own copy here, so no other test has counted it.)
    #[test]
    fn a_file_is_counted_once_and_only_to_play_it() {
        for name in ["h264_64.mkv", "h264_frag_64.mp4", "h264_vfr_64.mp4", "vp9_64.webm"] {
            let copy = std::env::temp_dir().join(format!("eyepiece-count-{}-{name}", std::process::id()));
            std::fs::copy(media(name), &copy).unwrap();
            let path = copy.to_str().unwrap();
            let refused = Clip::open_known(path).err().unwrap_or_else(|| panic!("{name} was counted for a picture"));
            assert!(refused.ends_with(UNCOUNTED), "{refused}");
            let counted = AtomicUsize::new(0);
            let frames = Clip::open_counting(path, &counted, &AtomicBool::new(false)).expect("open").timeline.frames();
            assert_eq!((frames, counted.load(std::sync::atomic::Ordering::Relaxed) >= 12), (12, true), "{name}");
            // known now: the same frames, with no second count
            let again = AtomicUsize::new(0);
            assert_eq!(Clip::open_counting(path, &again, &AtomicBool::new(false)).expect("open again").timeline.frames(), 12);
            assert_eq!(again.load(std::sync::atomic::Ordering::Relaxed), 0, "{name} was counted twice");
            assert_eq!(Clip::open_known(path).expect("known").timeline.frames(), 12);
            // written over, it is another file: not known
            std::fs::write(&copy, std::fs::read(media("h264_frag_64.mp4")).unwrap().iter().chain(&[0u8; 8]).copied().collect::<Vec<u8>>()).unwrap();
            assert!(Clip::open_known(path).is_err(), "{name}: a changed file was taken for the one counted");
            std::fs::remove_file(&copy).unwrap();
        }
        // a file with its table opens either way
        assert!(Clip::open_known(&media("h264_64.mp4")).is_ok());
    }

    /// a folder's list says of a clip what opening it would, from its header alone
    #[test]
    fn a_clips_facts_are_its_headers() {
        for name in ["hapq_64.mov", "h264_320.mp4", "h264_320_turn90.mp4", "prores_64.mov", "hevc10_64.mp4", "seq.0001.png", "still_8bit.png", "still_half.exr"] {
            let clip = Clip::open(&media(name)).unwrap_or_else(|e| panic!("{e}"));
            let found = facts(&media(name)).unwrap();
            assert_eq!(((found.width, found.height), found.frames, found.rate.clone()), (clip.upright(), Some(clip.timeline.frames()), Some(clip.rate_text())), "{name}");
            assert_eq!(found.fps, (clip.timeline.frames() > 1).then(|| clip.timeline.fps()), "{name}");
        }
        // a rate is another by its value, not by how the file spells it
        let at = |fps: f64| Facts { width: 64, height: 64, frames: Some(2), rate: None, fps: Some(fps) };
        assert!(!at(15360.0 / 256.0).other_rate(&at(60000.0 / 1000.0)) && at(60000.0 / 1001.0).other_rate(&at(60.0)));
        assert!(!at(60.0).other_rate(&Facts { fps: None, ..at(60.0) }) && Facts { width: 32, ..at(60.0) }.other_size(&at(60.0)));
        // a file that would have to be counted says its size and no more
        let copy = std::env::temp_dir().join(format!("eyepiece-facts-{}.mkv", std::process::id()));
        std::fs::copy(media("h264_64.mkv"), &copy).unwrap();
        assert_eq!(facts(copy.to_str().unwrap()).unwrap(), Facts { width: 64, height: 64, frames: None, rate: None, fps: None });
        std::fs::remove_file(&copy).unwrap();
    }

    /// a count that is told to stop ends with an error, and the file stays uncounted
    #[test]
    fn a_count_can_be_stopped() {
        let copy = std::env::temp_dir().join(format!("eyepiece-stop-{}.mkv", std::process::id()));
        std::fs::copy(media("h264_64.mkv"), &copy).unwrap();
        let path = copy.to_str().unwrap();
        assert!(Clip::open_counting(path, &AtomicUsize::new(0), &AtomicBool::new(true)).is_err());
        assert!(Clip::open_known(path).is_err());
        std::fs::remove_file(&copy).unwrap();
    }

    /// opening can run on a worker, which says how far the count has got
    #[test]
    fn a_clip_opens_on_a_worker_and_says_how_far_it_has_counted() {
        let counted = Arc::new(AtomicUsize::new(0));
        let theirs = counted.clone();
        let copy = std::env::temp_dir().join(format!("eyepiece-worker-{}.mkv", std::process::id()));
        std::fs::copy(media("h264_64.mkv"), &copy).unwrap();
        let path = copy.to_str().unwrap().to_string();
        let clip = std::thread::spawn(move || Clip::open_counting(&path, &theirs, &AtomicBool::new(false))).join().unwrap().expect("open");
        assert_eq!((clip.timeline.frames(), counted.load(std::sync::atomic::Ordering::Relaxed)), (12, 12));
        drop(clip);
        std::fs::remove_file(&copy).unwrap();
    }

    /// six frames a 30th of a second apart, then six a 15th, with b-frames: the clip opens with every frame
    #[test]
    fn frames_that_do_not_all_last_the_same_open() {
        let c = Clip::open(&media("h264_vfr_64.mp4")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((c.codec, c.timeline.frames(), c.rate_text().as_str()), ("h.264", 12, "variable rate"));
    }

    /// any frame, asked for in any order, is the same picture as when the clip is decoded straight through: the proof
    /// that going back to a key frame and decoding forward lands on the right frame. the h.264 clip has frames stored
    /// out of display order; the hevc clip has key frames with pictures that are shown before them. the h.264 clip
    /// again in mkv and as a fragmented mp4, vp9 in webm and frames of two lengths are found by their counted packets.
    #[test]
    fn any_frame_in_any_order_is_the_frame_asked_for() {
        for (name, count) in [("h264_64.mp4", 12), ("hevc10_64.mp4", 12), ("prores_64.mov", 4), ("h264_64.mkv", 12), ("h264_frag_64.mp4", 12), ("vp9_64.webm", 12), ("h264_vfr_64.mp4", 12)] {
            let straight = frames(name, &(0..count).collect::<Vec<_>>());
            let mut distinct = straight.clone();
            distinct.sort();
            distinct.dedup();
            assert_eq!(distinct.len(), count, "{name}: the test clip's frames should all differ");
            let order: Vec<usize> = [0, 7, 3, 11, 10, 9, 2, 5, 4, 8, 1, 6, 11, 0].into_iter().filter(|&f| f < count).collect();
            let jumped = frames(name, &order);
            for (frame, picture) in order.iter().zip(jumped) {
                assert_eq!(picture, straight[*frame], "{name}: frame {frame} reached by a jump is not frame {frame}");
            }
        }
    }

    /// a clip cut out of a longer one without re-encoding keeps the frames its first shown frame was built from, and
    /// hides them with an edit list. they are not frames of the clip: the count leaves them out, frame 0 is the first
    /// frame shown, and the last frame can be reached. (tests/media/h264_cut_320.mp4 is h264_320.mp4 from its seventh
    /// frame on: 24 samples stored, 18 shown.)
    #[test]
    fn frames_an_edit_list_hides_are_not_frames_of_the_clip() {
        let cut = Clip::open(&media("h264_cut_320.mp4")).expect("open");
        assert_eq!(cut.timeline.frames(), 18);
        assert_eq!(cut.rate_text(), "30 fps");
        let whole = frames("h264_320.mp4", &(0..24).collect::<Vec<_>>());
        let order = [0, 17, 9, 1, 16, 2, 17, 0];
        let got = frames("h264_cut_320.mp4", &order);
        for (frame, picture) in order.iter().zip(got) {
            assert_eq!(picture, whole[frame + 6], "frame {frame} of the cut clip is not frame {} of the clip it was cut from", frame + 6);
        }
    }

    /// the red of a still frame's first pixel, or why the frame cannot be shown.
    fn red(c: &Clip, frame: usize) -> Result<u8, String> {
        let Lane::Stills(stills) = &c.lane else { panic!("not the stills lane") };
        stills.source.want(&[frame]);
        match stills.source.get(frame, Duration::from_secs(10)) {
            Frame::Ready(image) => Ok(image.data[0]),
            Frame::Pending => panic!("frame {frame} never arrived"),
            Frame::Failed(why) => Err(why),
        }
    }

    /// tests/media/seq.0001.png to seq.0005.png, with seq.0003.png left out: frame n (from 1) is red 40n.
    #[test]
    fn a_numbered_run_of_stills_is_one_clip_with_its_gap() {
        let c = Clip::open(&media("seq.0001.png")).expect("open");
        assert_eq!((c.name.as_str(), c.width, c.height, c.codec, c.timeline.frames(), c.first_frame), ("seq.[0001-0005].png", 16, 16, "png", 5, 0));
        assert_eq!((c.missing.as_slice(), c.rate_text().as_str()), ([2].as_slice(), "24 fps assumed"));
        // the frame with no file fails as missing, and the clip plays on
        let missing = Err("this frame's file is missing".to_string());
        for (frame, expect) in [(0, Ok(40)), (4, Ok(200)), (2, missing), (3, Ok(160)), (1, Ok(80))] {
            assert_eq!(red(&c, frame), expect, "frame {frame}");
        }
        // opened by its third file, it starts on that file's frame
        let c = Clip::open(&media("seq.0004.png")).expect("open");
        assert_eq!((c.timeline.frames(), c.first_frame), (5, 3));
        assert_eq!(red(&c, 3), Ok(160));
    }

    #[test]
    fn a_lone_still_is_a_clip_of_one_frame() {
        let c = Clip::open(&media("still_8bit.png")).expect("open");
        assert_eq!((c.name.as_str(), c.codec, c.timeline.frames(), c.first_frame, c.rate_text().as_str()), ("still_8bit.png", "png", 1, 0, "still"));
        assert!(c.missing.is_empty());
        let Lane::Stills(stills) = &c.lane else { panic!("not the stills lane") };
        assert_eq!((stills.samples, stills.bits), (Samples::U8, 8));
        assert_eq!(red(&c, 0), Ok(0));
    }

    #[test]
    fn a_run_of_exr_plays_at_the_rate_its_files_state() {
        let dir = std::env::temp_dir().join(format!("eyepiece-test-{}-exr-run", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files = [dir.join("shot.0001.exr"), dir.join("shot.0002.exr")];
        for file in &files {
            std::fs::copy(media("still_float.exr"), file).unwrap();
        }
        let mut c = Clip::open(files[1].to_str().unwrap()).expect("open");
        assert_eq!((c.name.as_str(), c.timeline.frames(), c.first_frame, c.rate_text().as_str()), ("shot.[0001-0002].exr", 2, 1, "29.97 fps"));
        // whatever the rate for sequences is, and set to another and back to its own
        assert_eq!(Clip::open_stills(files[0].to_str().unwrap(), (25, 1)).expect("open").rate_text(), "29.97 fps");
        assert_eq!((c.rate_from, c.own_rate), (RateFrom::File, Some((30000, 1001))));
        c.set_rate((25, 1), RateFrom::Set);
        assert_eq!(c.rate_text(), "25 fps set");
        c.set_rate((30000, 1001), RateFrom::File);
        assert_eq!(c.rate_text(), "29.97 fps");
        drop(c);
        for file in &files {
            std::fs::remove_file(file).unwrap();
        }
        std::fs::remove_dir(&dir).unwrap();
    }

    /// a sequence plays at another rate when one is chosen for it (the owner's ask, 2026-11-26): its frames stay, its
    /// clock counts at the new rate, and its rate says where it came from: the file's own says nothing more, the
    /// setting's is assumed, one chosen is set. a video and a still alone keep their own
    #[test]
    fn a_sequence_takes_another_rate_and_says_where_it_came_from() {
        let mut c = Clip::open_stills(&media("seq.0001.png"), (24, 1)).expect("open");
        assert_eq!((c.rate_from, c.own_rate, c.rate_text().as_str()), (RateFrom::Assumed, None, "24 fps assumed"));
        // opened while the rate for sequences is another
        assert_eq!(Clip::open_stills(&media("seq.0002.png"), (25, 1)).expect("open").rate_text(), "25 fps assumed");
        c.set_rate((30000, 1001), RateFrom::Set);
        assert_eq!((c.timeline.frames(), c.missing.as_slice(), c.rate_text().as_str()), (5, [2].as_slice(), "29.97 fps set"));
        assert_eq!(c.clock, Some(Timecode::counted(30)));
        c.set_rate((25, 1), RateFrom::Set);
        assert_eq!((c.timeline.rate(), c.timeline.fps(), c.rate_text().as_str()), (Some((25, 1)), 25.0, "25 fps set"));
        assert_eq!(c.clock.map(|clock| clock.at(4)), Some("00:00:00:04".to_string()));
        let mut video = Clip::open(&media("hapq_64.mov")).expect("open");
        video.set_rate((25, 1), RateFrom::Set);
        assert_eq!(video.rate_text(), "59.94 fps");
        let mut still = Clip::open(&media("still_8bit.png")).expect("open");
        still.set_rate((25, 1), RateFrom::Set);
        assert_eq!(still.rate_text(), "still");
    }

    /// a rate is said as people say it, its ratio after only where its decimal is not exactly it (the owner's word,
    /// 2026-11-27: "idk what the first two numbers mean")
    #[test]
    fn a_rate_is_said_as_its_decimal_and_its_ratio_only_where_that_is_not_exact() {
        let said = [((15360, 512), "30"), ((24, 1), "24"), ((25, 2), "12.5"), ((30000, 1001), "29.97"), ((24000, 1001), "23.976"), ((2997, 100), "29.97 · 2997/100"), ((1000000, 33333), "30 · 1000000/33333")];
        for (rate, words) in said {
            assert_eq!(rate_said(rate), words, "{rate:?}");
        }
    }

    /// a rate typed for a sequence: a number, decimals allowed, or a ratio, from 1 to 240 frames a second. a decimal
    /// that rounds a rate of the /1001 kind is that exact ratio. anything else is refused, in words
    #[test]
    fn a_typed_rate_is_a_number_or_a_ratio() {
        let typed = [("24", (24, 1)), ("12", (12, 1)), ("15", (15, 1)), ("120", (120, 1)), ("1", (1, 1)), ("240", (240, 1)), (" 25 ", (25, 1)), ("25.000", (25, 1)), ("12.5", (25, 2)), ("29.9", (299, 10))];
        let ratios = [("30000/1001", (30000, 1001)), ("24000/1001", (24000, 1001)), ("60/2", (30, 1)), ("2997/100", (2997, 100))];
        let ntsc = [("23.976", (24000, 1001)), ("23.98", (24000, 1001)), ("29.97", (30000, 1001)), ("59.94", (60000, 1001)), ("47.952", (48000, 1001)), ("119.88", (120000, 1001))];
        for (text, rate) in typed.into_iter().chain(ratios).chain(ntsc) {
            assert_eq!(typed_rate(text), Ok(rate), "{text}");
        }
        for text in ["", "fast", ".", "1.2.3", "24/", "/1001", "30/0", "-24", "99999999999999999999"] {
            assert!(typed_rate(text).is_err_and(|why| why.contains("ratio")), "{text}");
        }
        for text in ["0", "0.5", "0.999", "241", "1000", "480/1"] {
            assert!(typed_rate(text).is_err_and(|why| why.contains("1 to 240")), "{text}");
        }
        // the rates offered are what they are called
        for (name, rate) in RATES {
            assert_eq!((typed_rate(name), rate_name(rate)), (Ok(rate), name.to_string()));
        }
        assert_eq!([(25, 2), (48000, 1001), (2997, 100), (120, 1)].map(rate_name), ["12.5", "47.952", "29.97", "120"].map(String::from));
    }

    #[test]
    fn something_that_is_not_media_is_an_error() {
        assert!(Clip::open(&media("hap_64.frame0.rgb")).is_err());
        assert!(Clip::open(&media("no such file.mov")).is_err());
    }

    /// gate 5, against the player's own way of holding a file: a render must be able to overwrite it in place, rename
    /// a new file over it, and delete and recreate it, all while the clip is open. hap reads through our own handle
    /// and the others through ffmpeg on our handle, so both are held here; a still is read whole and let go.
    #[test]
    fn a_render_can_replace_a_clip_that_is_open() {
        for name in ["hap_64.mov", "h264_64.mp4", "still_8bit.png"] {
            let dir = std::env::temp_dir().join(format!("eyepiece-test-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let (path, beside) = (dir.join(name), dir.join("beside.tmp"));
            let original = std::fs::read(media(name)).unwrap();
            std::fs::write(&path, &original).unwrap();
            let mut clip = Clip::open(path.to_str().unwrap()).expect("open");
            clip.start(None).expect("start");

            std::fs::write(&path, &original).expect("overwrite in place");
            std::fs::write(&beside, &original).unwrap();
            std::fs::rename(&beside, &path).expect("rename a new file over it");
            std::fs::remove_file(&path).expect("delete it");
            std::fs::write(&path, &original).expect("and recreate it");

            drop(clip);
            std::fs::remove_file(&path).unwrap();
            std::fs::remove_dir(&dir).unwrap();
        }
    }

    /// tells a reader what is wanted, as the render loop does every refresh, and says whether `frame` came within
    /// `patience`. a clip played here has no faults, so a frame that fails is a bug, and stops the run.
    fn arrived<T: Send + Sync + 'static>(source: &Source<T>, want: &[usize], frame: usize, patience: Duration) -> bool {
        source.want(want);
        match source.get(frame, patience) {
            Frame::Ready(_) => true,
            Frame::Pending => false,
            Frame::Failed(why) => panic!("frame {frame}: {why}"),
        }
    }

    fn median<T: Copy + Ord>(mut values: Vec<T>) -> T {
        values.sort();
        values[values.len() / 2]
    }

    /// plays clips as the render loop does on a 60 Hz display: each refresh tells the reader `Transport::ahead` and
    /// waits half a refresh for the frame due, and play moves on only once that frame is shown. for each run it prints
    /// how long the middle frame took from standing still with nothing else loading (a lone frame: one a step reaches
    /// when it was not ready), then, for the first pass and the second, the frames ready
    /// when first due out of the frames due, and how long the pass took. the clips are the files in EYEPIECE_PLAY,
    /// separated by `;` (any frame of a sequence, or a video file), each run EYEPIECE_RUNS times (3 if unset):
    /// `cargo test --release plays_at_speed -- --ignored --nocapture`. in software, unless EYEPIECE_SYSTEM is set
    /// on macos, where the system's decoder needs no device of ours.
    #[test]
    #[ignore]
    fn plays_at_speed() {
        let hz = 60.0;
        let refresh = Duration::from_secs_f64(1.0 / hz);
        let runs: usize = std::env::var("EYEPIECE_RUNS").map_or(3, |r| r.parse().expect("EYEPIECE_RUNS is a number"));
        for path in std::env::var("EYEPIECE_PLAY").expect("EYEPIECE_PLAY names the clips").split(';') {
            let mut results = Vec::new();
            for run in 0..runs {
                let mut clip = Clip::open(path).expect("open");
                #[cfg(windows)]
                let system = None;
                #[cfg(not(windows))]
                let system = std::env::var_os("EYEPIECE_SYSTEM").map(|_| Hardware);
                clip.start(system).expect("start");
                let look = |want: &[usize], frame: usize, patience: Duration| match &clip.lane {
                    Lane::Stills(s) => arrived(&s.source, want, frame, patience),
                    Lane::Video(v) => arrived(&v.source, want, frame, patience),
                    Lane::Hap(h) => arrived(&h.source, want, frame, patience),
                    Lane::Waiting(_) => unreachable!("started"),
                };
                let frames = clip.timeline.frames();
                let mut t = Transport::new(frames, clip.timeline.fps(), hz);
                let long = Duration::from_secs(30);
                t.go(clip.first_frame as i64);
                // the reader settles on the first frame and the one after before it is asked for another (of the many
                // frames wanted while standing still, a reader with one worker takes on only the first few)
                let near = t.ahead();
                assert!(near.iter().take(2).all(|&f| look(&near, f, long)), "the first frames arrived");
                t.go((frames / 2) as i64);
                let asked = Instant::now();
                assert!(look(&t.ahead(), t.frame(), long), "the middle frame arrived");
                let lone = asked.elapsed();
                t.go(0);
                assert!(look(&t.ahead(), 0, long), "frame 0 arrived");
                t.set_playing(true);
                // per pass: frames ready when first due, frames due, and how long it took
                let mut passes = [(0, 0, Duration::ZERO); 2];
                let (mut shown, mut last, mut due) = (None, None, 0);
                let (started, mut next) = (Instant::now(), Instant::now());
                while due < 2 * frames {
                    let frame = t.frame();
                    let ready = look(&t.ahead(), frame, if shown == Some(frame) { Duration::ZERO } else { refresh / 2 });
                    if last != Some(frame) {
                        last = Some(frame);
                        due += 1;
                        let pass = &mut passes[(due - 1) / frames];
                        pass.0 += ready as usize;
                        pass.1 += 1;
                        if due == frames + 1 {
                            passes[0].2 = started.elapsed();
                        }
                    }
                    if ready {
                        shown = Some(frame);
                        t.tick();
                    }
                    next += refresh;
                    std::thread::sleep(next.saturating_duration_since(Instant::now()));
                }
                passes[1].2 = started.elapsed() - passes[0].2;
                let ms = |d: Duration| d.as_millis();
                println!("{} run {run}: lone frame {} ms; first pass {}/{} in {} ms; second pass {}/{} in {} ms", clip.name, ms(lone), passes[0].0, passes[0].1, ms(passes[0].2), passes[1].0, passes[1].1, ms(passes[1].2));
                results.push([ms(lone), passes[0].0 as u128, ms(passes[0].2), passes[1].0 as u128, ms(passes[1].2)]);
            }
            let m: Vec<u128> = (0..5).map(|i| median(results.iter().map(|r| r[i]).collect())).collect();
            println!("{path} median of {runs}: lone frame {} ms; first pass {} on time in {} ms; second pass {} on time in {} ms", m[0], m[1], m[2], m[3], m[4]);
        }
    }
}
