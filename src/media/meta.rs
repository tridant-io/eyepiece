//! the facts the metadata panel shows. each comes from the file, or says the file does not carry it and shows the
//! assumption in use (adr 0003). frames and rate are counted from the sample table the way the player counts them.

use super::clip::{self, hap_variant, Count};
use super::color::Matrix;
use super::ff::{self, Input, Sample};
use super::hap::{self, Tex};
use super::still::{self, Samples};
use super::timeline::Timeline;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::UNIX_EPOCH;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Plain,
    /// amber in the panel: a fault in this file
    Fault,
    /// gray in the panel: the file does not say; the value is the assumption in use
    Assumed,
}

#[derive(Debug)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
    pub tone: Tone,
}

/// groups are separated by a rule in the panel
pub struct Meta {
    pub groups: Vec<Vec<Row>>,
}

fn row(label: &'static str, value: impl Into<String>, tone: Tone) -> Row {
    Row { label, value: value.into(), tone }
}

impl Meta {
    /// the facts of a file that is open, or is being opened: a file that lists no frames is read through to count
    /// them, if this run has not counted it yet.
    pub fn read(path: &str) -> Result<Meta, String> {
        Meta::read_with(path, &Count::Read { counted: &AtomicUsize::new(0), stop: &AtomicBool::new(false) })
    }

    /// the facts of a file that is only looked at, in the browser: one that would have to be counted is refused.
    pub fn read_known(path: &str) -> Result<Meta, String> {
        Meta::read_with(path, &Count::Known)
    }

    fn read_with(path: &str, count: &Count) -> Result<Meta, String> {
        if still::is_still(path) {
            return still_facts(path);
        }
        let file = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
        let mut input = Input::open(path)?;
        let video = input.first_video().ok_or_else(|| format!("{path}: no picture in this file"))?;
        let tag = ff::fourcc(video.tag);
        // mkv, a fragmented mp4 and frames that vary are counted by reading the whole file (`clip::stored`)
        let (stored, shown) = clip::stored(path, &input, &video, count)?;
        let (count, hidden, all_key) = (stored.len(), stored.iter().filter(|s| s.hidden).count(), stored.iter().all(|s| s.key));
        let (samples, timeline, _) = clip::frames(&video, stored, shown);
        // before `probe`, which mixes in what the decoder finds or assumes
        let container = input.color(video.index);
        let audio = input.audio();

        let name = Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
        let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let modified = file.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok());
        let place = vec![
            row("file", format!("{name} · {}", size(file.len())), Tone::Plain),
            row("modified", modified.map_or_else(|| "not known".to_string(), |d| utc(d.as_secs() as i64)), Tone::Plain),
            row("path", std::path::absolute(path).map_or_else(|_| path.to_string(), |p| p.display().to_string()), Tone::Plain),
            row("container", container_name(&input.format_name(), &ext), Tone::Plain),
        ];

        let n = timeline.frames();
        let mut frames = match n {
            0 => "not listed up front in this kind of file".to_string(),
            _ => format!("{} · numbered 0 to {}", grouped(n as u64), grouped(n as u64 - 1)),
        };
        if hidden > 0 {
            frames += &format!(" · {} stored, {} hidden by an edit list", grouped(count as u64), grouped(hidden as u64));
        }
        if n > 0 && all_key {
            frames += " · all key frames";
        }
        // up to `max` bytes of the first frame
        let first = |max: usize| samples.first().ok_or_else(|| "no frames listed".to_string()).and_then(|s| bytes(path, &Sample { size: s.size.min(max), ..*s }));
        let (codec, depth, alpha, color) = if let Some(variant) = hap_variant(&tag) {
            let (codec, depth) = hap_rows(variant, &tag, first(usize::MAX));
            let alpha = match tag.as_str() {
                "Hap5" | "HapM" => row("alpha", "yes", Tone::Plain),
                "HapA" => row("alpha", "alpha only", Tone::Plain),
                "Hap7" => row("alpha", "hap r does not say. shown as opaque", Tone::Assumed),
                _ => row("alpha", "none", Tone::Plain),
            };
            (codec, depth, alpha, vec![row("color", "hap cannot say. shown as srgb, full range", Tone::Assumed)])
        } else if video.codec_name == "prores" {
            let header = first(PRORES_HEADER).and_then(|raw| prores_header(&raw));
            let profile = prores_profile(&tag);
            let codec = row("codec", format!("{} · {tag}", profile.map_or("prores".to_string(), |p| format!("prores {}", p.0))), Tone::Plain);
            let depth = match profile {
                Some((_, bits, chroma)) => row("bit depth", format!("{bits}-bit · {chroma} · nominal for this profile"), Tone::Plain),
                None => row("bit depth", "prores stores none, and this profile's nominal depth is not known", Tone::Assumed),
            };
            let alpha = match &header {
                Ok((_, 0)) => row("alpha", "none", Tone::Plain),
                Ok((_, 1)) => row("alpha", "yes · 8-bit in file", Tone::Plain),
                Ok((_, 2)) => row("alpha", "yes · 16-bit in file", Tone::Plain),
                Ok((_, other)) => row("alpha", format!("unknown alpha type {other} in the frame header"), Tone::Fault),
                Err(why) => row("alpha", why.clone(), Tone::Fault),
            };
            // the decoder takes a tag from the frame header when it names a color (0, 2 and 3 do not: rdd 36), and the
            // container's otherwise. the range it never reads: it is always video range
            let tags: [i32; 3] = std::array::from_fn(|i| match &header {
                Ok((h, _)) if !matches!(h[i], 0 | 2 | 3) => h[i] as i32,
                _ => container[i],
            });
            (codec, depth, alpha, vec![color_row(tags, video.height), row("range", "not stated in file. shown as video range", Tone::Assumed)])
        } else {
            let probed = match video.codec_name.as_str() {
                "h264" | "hevc" => input.probe(),
                _ => Err("not read for this codec yet".to_string()),
            };
            let codec = clip::codec_name(&video).map_or_else(|| video.codec_name.clone(), str::to_string);
            let depth = match probed.and_then(|()| input.depth(video.index).ok_or_else(|| "not read: the decoder named no pixel format".to_string())) {
                Ok((bits, shift)) => row("bit depth", format!("{bits}-bit · {}", chroma(shift)), Tone::Plain),
                Err(why) => row("bit depth", why, Tone::Assumed),
            };
            // after the probe: the container's tags, overridden by the stream's where it has them
            let stream = input.color(video.index);
            let range = match stream[3] {
                2 => row("range", "full", Tone::Plain),
                // hevc's decoder says video range for a stream that does not say, so only the container makes that a fact
                1 if container[3] == 1 || video.codec_name != "hevc" => row("range", "video", Tone::Plain),
                _ => row("range", "not stated in file. shown as video range", Tone::Assumed),
            };
            let tags = [stream[0], stream[1], stream[2]];
            // mkv names a codec by an id of its own, not a four-letter tag
            let codec = if video.tag == 0 { codec } else { format!("{codec} · {tag}") };
            (row("codec", codec, Tone::Plain), depth, row("alpha", "not read for this codec yet. shown as opaque", Tone::Assumed), vec![color_row(tags, video.height), range])
        };
        let mut picture = vec![codec, row("size", format!("{} × {}", video.width, video.height), Tone::Plain), rate_row(&timeline), row("frames", frames, Tone::Plain), duration_row(&timeline), depth, alpha];
        picture.extend(color);

        let length = timeline.step().map(|step| {
            let frame = step as f64 * timeline.time_base.0 as f64 / timeline.time_base.1 as f64;
            (n as f64 * frame, frame)
        });
        let mut sound: Vec<Row> = audio
            .iter()
            .map(|a| {
                let base = format!("{} · {} kHz · {} ch", a.codec_name, decimal(a.sample_rate as f64 / 1000.0, 3), a.channels);
                match (a.seconds, length) {
                    // the margin keeps float error from hiding a difference of exactly one frame
                    (Some(s), Some((picture, frame))) if (s - picture).abs() >= frame - 1e-9 => {
                        let off = decimal((s - picture).abs() / frame, 1);
                        row("audio", format!("{base} · {s:.3} s, {off} frames {} than the picture", if s < picture { "shorter" } else { "longer" }), Tone::Fault)
                    }
                    _ => row("audio", base, Tone::Plain),
                }
            })
            .collect();
        if sound.is_empty() {
            sound.push(row("audio", "none", Tone::Plain));
        }
        // ffmpeg's own read of a quicktime timecode track, for now: the design wants ours, of the tmcd track in both forms
        let timecode = timecode_tag(&input, video.index);
        sound.push(match timecode {
            Some(tc) if tc.contains(';') => row("timecode", format!("{tc} · drop-frame"), Tone::Plain),
            Some(tc) => row("timecode", tc, Tone::Plain),
            None => row("timecode", "none in file: counted from 00:00:00:00", Tone::Assumed),
        });
        Ok(Meta { groups: vec![place, picture, sound] })
    }

    /// for "copy all": one "label: value" line per row, a blank line between groups
    pub fn text(&self) -> String {
        self.groups.iter().map(|g| g.iter().map(|r| format!("{}: {}\n", r.label, r.value)).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    pub fn faults(&self) -> usize {
        self.groups.iter().flatten().filter(|r| r.tone == Tone::Fault).count()
    }
}

/// the stored bytes of one sample.
/// a still, or the numbered run it is a frame of: what its own file says, and what the folder's listing says of the run.
fn still_facts(path: &str) -> Result<Meta, String> {
    let file = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?;
    let image = still::decode(path)?;
    let name = Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    let modified = file.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok());
    let place = vec![
        row("file", format!("{name} · {}", size(file.len())), Tone::Plain),
        row("modified", modified.map_or_else(|| "not known".to_string(), |d| utc(d.as_secs() as i64)), Tone::Plain),
        row("path", std::path::absolute(path).map_or_else(|_| path.to_string(), |p| p.display().to_string()), Tone::Plain),
    ];
    let mut picture = vec![row("codec", image.codec, Tone::Plain), row("size", format!("{} × {}", image.width, image.height), Tone::Plain)];
    if let Some(run) = clip::run_of(Path::new(path)) {
        picture.push(row("sequence", format!("{} · {} frames · this file is frame {}", run.name, grouped(run.frames as u64), run.at), Tone::Plain));
        if !run.missing.is_empty() {
            let first: Vec<String> = run.missing.iter().take(8).map(|f| f.to_string()).collect();
            let more = if run.missing.len() > 8 { " and more" } else { "" };
            picture.push(row("missing", format!("{} frames have no file: {}{more}", run.missing.len(), first.join(", ")), Tone::Fault));
        }
        picture.push(match image.rate {
            Some((n, d)) => row("frame rate", format!("{n}/{d} · {}", decimal(n as f64 / d as f64, 3)), Tone::Plain),
            None => row("frame rate", "none in file. shown at 24", Tone::Assumed),
        });
    }
    picture.push(row(
        "bit depth",
        match image.samples {
            Samples::U8 => "8-bit".to_string(),
            Samples::U16 => format!("{}-bit", image.bits),
            Samples::F32 => format!("{}-bit float", image.bits),
        },
        Tone::Plain,
    ));
    picture.push(row("alpha", if image.has_alpha { "yes" } else { "none" }, Tone::Plain));
    picture.push(match image.samples {
        Samples::F32 => row("color", "linear light. shown through the srgb curve", Tone::Assumed),
        _ => row("color", "not read from this kind of file yet. shown as stored", Tone::Assumed),
    });
    Ok(Meta { groups: vec![place, picture] })
}

fn bytes(path: &str, s: &Sample) -> Result<Vec<u8>, String> {
    let mut raw = vec![0u8; s.size];
    ff::read_at(&ff::open_shared(path)?, &mut raw, s.pos).map_err(|e| format!("{path}: {e}"))?;
    Ok(raw)
}

/// the codec and bit depth rows of a hap clip, from its first frame's sections.
fn hap_rows(variant: &str, tag: &str, first: Result<Vec<u8>, String>) -> (Row, Row) {
    let layers = first.and_then(|raw| hap::parse(&raw).map(|layers| layers.iter().map(|l| (l.tex, l.chunks.len())).collect::<Vec<_>>()));
    let layers = match layers {
        Ok(layers) => layers,
        Err(why) => return (row("codec", format!("{variant} · {tag} · first frame unreadable: {why}"), Tone::Fault), row("bit depth", "not read: first frame unreadable", Tone::Plain)),
    };
    let chunks = layers.iter().map(|l| l.1.to_string()).collect::<Vec<_>>().join(" + ");
    let chunks = if layers.iter().all(|l| l.1 == 1) { format!("{chunks} chunk: decodes on one core") } else { format!("{chunks} chunks") };
    let bits = if layers.iter().any(|l| matches!(l.0, Tex::Bc6u | Tex::Bc6s)) { "16-bit half float" } else { "8-bit" };
    let textures = layers.iter().map(|l| texture(l.0)).collect::<Vec<_>>().join(" + ");
    (row("codec", format!("{variant} · {tag} · {chunks}"), Tone::Plain), row("bit depth", format!("{bits} · {textures}"), Tone::Plain))
}

fn texture(tex: Tex) -> &'static str {
    match tex {
        Tex::Bc1 => "bc1",
        Tex::Bc3 => "bc3",
        Tex::YCoCgBc3 => "bc3 scaled ycocg",
        Tex::Bc7 => "bc7",
        Tex::Bc4 => "bc4",
        Tex::Bc6u => "bc6h unsigned",
        Tex::Bc6s => "bc6h signed",
    }
}

/// prores profiles by tag: the name, and the depth and chroma each is made for. prores stores no depth of its own.
fn prores_profile(tag: &str) -> Option<(&'static str, u32, &'static str)> {
    Some(match tag {
        "apco" => ("422 proxy", 10, "4:2:2"),
        "apcs" => ("422 lt", 10, "4:2:2"),
        "apcn" => ("422", 10, "4:2:2"),
        "apch" => ("422 hq", 10, "4:2:2"),
        "ap4h" => ("4444", 12, "4:4:4"),
        "ap4x" => ("4444 xq", 12, "4:4:4"),
        _ => return None,
    })
}

/// the bytes of a prores frame that hold what `prores_header` reads.
const PRORES_HEADER: usize = 28;

/// a prores frame header's color tags (primaries, transfer, matrix) and its alpha type (0 none, 1 8-bit, 2 16-bit).
/// the frame starts with its size and "icpf"; the header follows, laid out as ffmpeg's proresdec.c reads it.
fn prores_header(raw: &[u8]) -> Result<([u8; 3], u8), String> {
    if raw.len() < PRORES_HEADER || &raw[4..8] != b"icpf" {
        return Err("no prores frame header in the first frame".into());
    }
    Ok(([raw[22], raw[23], raw[24]], raw[25] & 0x0f))
}

fn chroma(shift: (u32, u32)) -> &'static str {
    match shift {
        (1, 1) => "4:2:0",
        (1, 0) => "4:2:2",
        (0, 0) => "4:4:4",
        _ => "other chroma",
    }
}

/// the matrix the player converts with for a tagged matrix, mapped as `dec::Picture::layout` maps it.
/// None: the player uses the usual assumption instead.
fn converts_with(matrix: i32) -> Option<Matrix> {
    match matrix {
        1 => Some(Matrix::Bt709),
        5 | 6 => Some(Matrix::Bt601),
        9 => Some(Matrix::Bt2020),
        _ => None,
    }
}

fn matrix_name(m: Matrix) -> &'static str {
    match m {
        Matrix::Bt601 => "bt.601",
        Matrix::Bt709 => "bt.709",
        Matrix::Bt2020 => "bt.2020",
    }
}

/// the color row from h.273 codes: primaries, transfer, matrix.
fn color_row([primaries, transfer, matrix]: [i32; 3], height: usize) -> Row {
    let named = |code: i32, names: &[(i32, &str)]| match names.iter().find(|n| n.0 == code) {
        Some(n) => n.1.to_string(),
        None if matches!(code, 2 | 3) => "not stated".to_string(),
        None => format!("code {code}"),
    };
    let rest = format!(
        "primaries {} · transfer {}",
        named(primaries, &[(0, "not stated"), (1, "bt.709"), (5, "bt.470bg"), (6, "smpte 170m"), (9, "bt.2020"), (11, "dci-p3"), (12, "display p3")]),
        named(transfer, &[(0, "not stated"), (1, "bt.709"), (6, "smpte 170m"), (8, "linear"), (13, "srgb"), (14, "bt.2020 10-bit"), (15, "bt.2020 12-bit"), (16, "pq"), (18, "hlg")]),
    );
    let assumed = matrix_name(Matrix::assumed(height));
    match converts_with(matrix) {
        Some(_) => row("color", format!("matrix {} · {rest}", named(matrix, &[(1, "bt.709"), (5, "bt.470bg"), (6, "smpte 170m"), (9, "bt.2020 nc")])), Tone::Plain),
        None if matches!(matrix, 2 | 3) => row("color", format!("matrix not stated in file. shown as {assumed} · {rest}"), Tone::Assumed),
        None => row("color", format!("matrix {}, not converted yet: shown as {assumed} · {rest}", named(matrix, &[(0, "rgb"), (10, "bt.2020 c"), (14, "ictcp")])), Tone::Plain),
    }
}

fn rate_row(t: &Timeline) -> Row {
    match (t.rate(), t.derived()) {
        (Some((n, d)), _) => row("frame rate", format!("{n}/{d} · {}", decimal(n as f64 / d as f64, 3)), Tone::Plain),
        (None, Some(fps)) => row("frame rate", format!("{} · derived: the file rounds its frames' times to its clock", decimal(fps, 3)), Tone::Assumed),
        (None, None) if t.frames() == 0 => row("frame rate", "not listed up front in this kind of file", Tone::Plain),
        (None, None) if t.frames() == 1 => row("frame rate", "still: one frame", Tone::Plain),
        (None, None) => row("frame rate", "variable: its frames do not all last the same. each is shown for its own time", Tone::Fault),
    }
}

/// shown frames × the frame duration, or the rate derived.
fn duration_row(t: &Timeline) -> Row {
    let n = t.frames();
    let length = |ms: i128| format!("{}:{:02}:{:02}.{:03} · {} frames", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000, grouped(n as u64));
    let (value, tone) = match (t.step(), t.derived()) {
        (Some(step), _) => {
            let units = n as i128 * step as i128 * t.time_base.0 as i128;
            let den = t.time_base.1 as i128;
            (length((units * 2000 + den) / (2 * den)), Tone::Plain)
        }
        (None, Some(fps)) => (format!("{} · at the rate derived", length((n as f64 * 1000.0 / fps).round() as i128)), Tone::Assumed),
        (None, None) if n == 0 => ("not listed up front in this kind of file".to_string(), Tone::Plain),
        (None, None) if n == 1 => ("1 frame".to_string(), Tone::Plain),
        (None, None) => (format!("{} frames · their lengths vary", grouped(n as u64)), Tone::Plain),
    };
    row("duration", value, tone)
}

/// a person's name for the container: ffmpeg's name, with the extension to tell quicktime from mp4.
fn container_name(format: &str, ext: &str) -> String {
    let kind = match format {
        f if f.starts_with("mov,") => {
            if matches!(ext, "mov" | "qt") {
                "quicktime"
            } else {
                "mpeg-4"
            }
        }
        "matroska,webm" if ext == "webm" => "webm",
        "matroska,webm" => "matroska",
        other => other,
    };
    format!("{kind} · .{ext}")
}

/// `x` to at most `places` decimals, without trailing zeros.
pub(super) fn decimal(x: f64, places: usize) -> String {
    let s = format!("{x:.places$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// 1234567 as "1,234,567".
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// a file size in decimal units to three figures, and in bytes.
fn size(bytes: u64) -> String {
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < 4 {
        v /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} bytes");
    }
    let places = if v < 10.0 { 2 } else if v < 100.0 { 1 } else { 0 };
    format!("{v:.places$} {} · {} bytes", ["", "KB", "MB", "GB", "TB"][unit], grouped(bytes))
}

/// seconds since 1970 as a utc date and time. the os's time zone is not read: that needs a system call per os.
fn utc(secs: i64) -> String {
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // days to a civil date (howard hinnant's algorithm), with years starting in march so leap days fall last
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!("{year}-{month:02}-{day:02} {:02}:{:02}:{:02} utc", rest / 3600, rest / 60 % 60, rest % 60)
}

/// the file's own timecode as written, in ffmpeg's read of it: the picture's stream's, else any stream's, else the
/// container's.
pub fn timecode_tag(input: &Input, video: usize) -> Option<String> {
    std::iter::once(Some(video)).chain((0..input.streams()).map(Some)).chain([None]).find_map(|s| input.tag(s, "timecode"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(name: &str) -> Meta {
        Meta::read(&format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))).expect(name)
    }

    /// the value and tone of the first row with this label.
    fn get<'a>(m: &'a Meta, label: &str) -> (&'a str, Tone) {
        let r = m.groups.iter().flatten().find(|r| r.label == label).unwrap_or_else(|| panic!("no {label} row"));
        (r.value.as_str(), r.tone)
    }

    #[test]
    fn hap_says_its_variant_chunks_and_texture() {
        let m = read("hap_64.mov");
        assert_eq!(get(&m, "codec"), ("hap · Hap1 · 2 chunks", Tone::Plain));
        assert_eq!(get(&m, "size"), ("64 × 64", Tone::Plain));
        assert_eq!(get(&m, "frames"), ("3 · numbered 0 to 2 · all key frames", Tone::Plain));
        assert_eq!(get(&m, "bit depth"), ("8-bit · bc1", Tone::Plain));
        assert_eq!(get(&m, "alpha"), ("none", Tone::Plain));
        assert_eq!(get(&m, "color").1, Tone::Assumed);
        assert!(m.groups.iter().flatten().all(|r| r.label != "range"));
        assert_eq!(get(&m, "container"), ("quicktime · .mov", Tone::Plain));

        let m = read("hapq_64.mov");
        // the file's own ratio, unreduced, then its decimal
        assert_eq!(get(&m, "frame rate"), ("60000/1001 · 59.94", Tone::Plain));
        assert_eq!(get(&m, "duration"), ("0:00:00.050 · 3 frames", Tone::Plain));
        assert_eq!(get(&m, "codec"), ("hap q · HapY · 4 chunks", Tone::Plain));
        assert_eq!(get(&m, "bit depth"), ("8-bit · bc3 scaled ycocg", Tone::Plain));

        let m = read("hapalpha_64.mov");
        assert_eq!(get(&m, "codec"), ("hap alpha · Hap5 · 1 chunk: decodes on one core", Tone::Plain));
        assert_eq!(get(&m, "alpha"), ("yes", Tone::Plain));
        assert_eq!(m.faults(), 0);
    }

    #[test]
    fn frames_an_edit_list_hides_are_counted_apart() {
        let m = read("h264_cut_320.mp4");
        assert_eq!(get(&m, "frames"), ("18 · numbered 0 to 17 · 24 stored, 6 hidden by an edit list", Tone::Plain));
        assert_eq!(get(&m, "frame rate"), ("15360/512 · 30", Tone::Plain));
        assert_eq!(get(&m, "duration"), ("0:00:00.600 · 18 frames", Tone::Plain));
        assert_eq!(get(&m, "codec"), ("h.264 · avc1", Tone::Plain));
        assert_eq!(get(&m, "bit depth"), ("8-bit · 4:2:0", Tone::Plain));
        assert_eq!(get(&m, "container"), ("mpeg-4 · .mp4", Tone::Plain));
        // nothing reads alpha from h.264 yet, and it says so
        assert_eq!(get(&m, "alpha").1, Tone::Assumed);
    }

    #[test]
    fn prores_depth_is_nominal_and_alpha_comes_from_the_frame_header() {
        let raw = {
            let path = format!("{}/tests/media/prores_64.mov", env!("CARGO_MANIFEST_DIR"));
            let input = Input::open(&path).unwrap();
            let first = input.samples(input.first_video().unwrap().index)[0];
            bytes(&path, &first).unwrap()
        };
        // the header is where proresdec.c says: it holds the picture's size at these offsets
        assert_eq!((u16::from_be_bytes([raw[16], raw[17]]), u16::from_be_bytes([raw[18], raw[19]])), (64, 64));
        assert_eq!(prores_header(&raw).unwrap().1, 0);
        assert!(prores_header(&raw[..20]).is_err());

        let m = read("prores_64.mov");
        assert_eq!(get(&m, "codec"), ("prores 422 hq · apch", Tone::Plain));
        assert_eq!(get(&m, "bit depth"), ("10-bit · 4:2:2 · nominal for this profile", Tone::Plain));
        assert_eq!(get(&m, "alpha"), ("none", Tone::Plain));
        // prores has no range field: what ffmpeg reports is a constant in its decoder
        assert_eq!(get(&m, "range"), ("not stated in file. shown as video range", Tone::Assumed));
        assert_eq!(get(&m, "color"), ("matrix not stated in file. shown as bt.601 · primaries not stated · transfer not stated", Tone::Assumed));

        let m = read("prores4444_64.mov");
        assert_eq!(get(&m, "codec"), ("prores 4444 · ap4h", Tone::Plain));
        assert_eq!(get(&m, "bit depth"), ("12-bit · 4:4:4 · nominal for this profile", Tone::Plain));
        assert_eq!(get(&m, "alpha"), ("yes · 16-bit in file", Tone::Plain));
        assert_eq!(get(&m, "color"), ("matrix bt.709 · primaries bt.709 · transfer bt.709", Tone::Plain));
    }

    #[test]
    fn color_is_what_the_file_tags_or_the_assumption_in_use() {
        // tagged in the stream alone, which only the probe reads
        let path = format!("{}/tests/media/h264_vui709_64.mp4", env!("CARGO_MANIFEST_DIR"));
        let input = Input::open(&path).unwrap();
        assert_eq!(input.color(input.first_video().unwrap().index), [2, 2, 2, 0], "the container should carry no tags");
        let m = read("h264_vui709_64.mp4");
        assert_eq!(get(&m, "color"), ("matrix bt.709 · primaries bt.709 · transfer bt.709", Tone::Plain));
        assert_eq!(get(&m, "range"), ("video", Tone::Plain));

        let m = read("h264_709_64.mp4");
        assert_eq!(get(&m, "color"), ("matrix bt.709 · primaries bt.709 · transfer bt.709", Tone::Plain));
        assert_eq!(get(&m, "range"), ("video", Tone::Plain));
        // untagged: bt.601 below 720 lines, bt.709 from 720 up
        let m = read("h264_64.mp4");
        assert_eq!(get(&m, "color"), ("matrix not stated in file. shown as bt.601 · primaries not stated · transfer not stated", Tone::Assumed));
        assert_eq!(get(&m, "range").1, Tone::Assumed);
        let m = read("h264_720.mp4");
        assert_eq!(get(&m, "color"), ("matrix not stated in file. shown as bt.709 · primaries not stated · transfer not stated", Tone::Assumed));
        // hevc's decoder reports video range for a stream that does not say: not a fact from the file
        let m = read("hevc10_64.mp4");
        assert_eq!(get(&m, "range"), ("not stated in file. shown as video range", Tone::Assumed));
        assert_eq!(get(&m, "bit depth"), ("10-bit · 4:2:0", Tone::Plain));
        assert_eq!(get(&m, "codec"), ("hevc · hvc1", Tone::Plain));
    }

    #[test]
    fn audio_off_by_a_frame_or_more_is_a_fault_and_timecode_is_read() {
        let m = read("av_tc_64.mov");
        assert_eq!(get(&m, "frame rate"), ("30000/1001 · 29.97", Tone::Plain));
        assert_eq!(get(&m, "timecode"), ("01:00:00;00 · drop-frame", Tone::Plain));
        let audio: Vec<_> = m.groups.iter().flatten().filter(|r| r.label == "audio").map(|r| (r.value.as_str(), r.tone)).collect();
        // the first track is 0.4 ms shorter than the picture; the second 0.1 s, three frames
        assert_eq!(audio, [("pcm_s16le · 8 kHz · 2 ch", Tone::Plain), ("pcm_s16le · 8 kHz · 1 ch · 0.300 s, 3 frames shorter than the picture", Tone::Fault)]);
        assert_eq!(m.faults(), 1);

        let m = read("hap_64.mov");
        assert_eq!(get(&m, "audio"), ("none", Tone::Plain));
        assert_eq!(get(&m, "timecode"), ("none in file: counted from 00:00:00:00", Tone::Assumed));
    }

    #[test]
    fn a_file_without_a_frame_table_is_counted() {
        let m = read("h264_64.mkv");
        assert_eq!(get(&m, "container"), ("matroska · .mkv", Tone::Plain));
        assert_eq!(get(&m, "frames"), ("12 · numbered 0 to 11", Tone::Plain));
        // milliseconds cannot hold a 30th of a second: the rate is derived from the times, and says so
        assert_eq!(get(&m, "frame rate"), ("30 · derived: the file rounds its frames' times to its clock", Tone::Assumed));
        assert_eq!(get(&m, "duration"), ("0:00:00.400 · 12 frames · at the rate derived", Tone::Assumed));
        assert_eq!(get(&m, "codec"), ("h.264", Tone::Plain));
        let m = read("vp9_64.webm");
        assert_eq!(get(&m, "container"), ("webm · .webm", Tone::Plain));
        assert_eq!((get(&m, "codec").0, get(&m, "frames").0), ("vp9", "12 · numbered 0 to 11"));
        let m = read("h264_frag_64.mp4");
        assert_eq!((get(&m, "frames").0, get(&m, "frame rate").0), ("12 · numbered 0 to 11", "15360/512 · 30"));
    }

    #[test]
    fn frames_that_do_not_all_last_the_same_are_a_fault() {
        let m = read("h264_vfr_64.mp4");
        assert_eq!(get(&m, "frames"), ("12 · numbered 0 to 11", Tone::Plain));
        assert_eq!(get(&m, "frame rate"), ("variable: its frames do not all last the same. each is shown for its own time", Tone::Fault));
        let t = Timeline { time_base: (1, 1000), times: vec![0, 40, 90, 120] };
        assert_eq!(rate_row(&t).tone, Tone::Fault);
        assert_eq!(duration_row(&t).value, "4 frames · their lengths vary");
        let t = Timeline { time_base: (1, 2997), times: (0..3).map(|i| i * 100).collect() };
        assert_eq!(rate_row(&t).value, "2997/100 · 29.97");
        // 216,000 frames at 60000/1001 last 3603.6 s; 215,784 last 3599.99640 s, which rounds to the nearest millisecond
        let t = Timeline::even((1, 60000), 216_000, 1001);
        assert_eq!(duration_row(&t).value, "1:00:03.600 · 216,000 frames");
        let t = Timeline::even((1, 60000), 215_784, 1001);
        assert_eq!(duration_row(&t).value, "0:59:59.996 · 215,784 frames");
    }

    #[test]
    fn copy_all_is_one_line_a_row_and_a_blank_line_between_groups() {
        let m = Meta { groups: vec![vec![row("a", "1", Tone::Plain), row("b", "2", Tone::Fault)], vec![row("c", "3", Tone::Assumed)]] };
        assert_eq!(m.text(), "a: 1\nb: 2\n\nc: 3\n");
        assert_eq!(m.faults(), 1);
    }

    #[test]
    fn numbers_read_as_a_person_writes_them() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 utc");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00 utc");
        assert_eq!(utc(1_700_000_000), "2023-11-14 22:13:20 utc");
        assert_eq!(grouped(2_412_345_678), "2,412,345,678");
        assert_eq!(grouped(999), "999");
        assert_eq!(size(2_412_345_678), "2.41 GB · 2,412,345,678 bytes");
        assert_eq!(size(33_895), "33.9 KB · 33,895 bytes");
        assert_eq!(size(512), "512 bytes");
        assert_eq!(decimal(44.1, 3), "44.1");
        assert_eq!(decimal(30.0, 3), "30");
    }
}
