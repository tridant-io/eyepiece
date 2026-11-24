//! one open clip: what the container says about it, its exact frame times, and where its frames come from.

use super::dec::Hardware;
use super::ff::{self, Input, Sample, Video};
use super::hap_source::HapLane;
use super::seq_source::SeqLane;
use super::still;
use super::timeline::Timeline;
use super::video_source::VideoLane;
use crate::browse::{self, Kind};
use std::path::Path;
use std::sync::Arc;

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
    /// true when the file states no rate and the timeline runs at the assumed one (a sequence: 24)
    pub rate_assumed: bool,
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
struct Run {
    name: String,
    frames: usize,
    /// the frame of the file the run was found from
    at: usize,
    missing: Vec<usize>,
    path_of: Box<dyn Fn(usize) -> String + Send>,
}

/// the file's name without its folder.
fn file_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

/// the numbered run in the file's folder that the file is a frame of, from the folder's listing alone: what the
/// listing lacks is missing, and no other file is opened.
fn run_of(file: &Path) -> Option<Run> {
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
    step: i64,
    size: (usize, usize),
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
        _ => None,
    })
}

/// a clip's frames, from the container's own sample table: the samples in the order they are read, the times of the
/// frames shown, and how long every frame lasts when they all last the same.
pub(super) fn frames(video: &Video, mut samples: Vec<Sample>) -> (Vec<Sample>, Timeline, Option<i64>) {
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
    /// reads the header. a hap clip or stills start reading at once; any other waits for `start`.
    pub fn open(path: &str) -> Result<Clip, String> {
        if still::is_still(path) {
            return Clip::open_stills(path);
        }
        let input = Input::open(path)?;
        let video = input.first_video().ok_or_else(|| format!("{path}: no picture in this file"))?;
        let tag = ff::fourcc(video.tag);
        let name = file_name(path);
        let codec = codec_name(&video).ok_or_else(|| format!("{path}: {} ({tag}) does not play yet. this build plays hap, prores, h.264 and hevc.", video.codec_name))?;
        let (samples, timeline, step) = frames(&video, input.samples(video.index));
        let lane = if hap_variant(&tag).is_some() {
            Lane::Hap(HapLane::open(path, samples, video.width, video.height)?)
        } else {
            let step = match (step, samples.len()) {
                (Some(step), _) => step,
                // mkv and fragmented mp4 carry no table of frames: they have to be counted by reading the file, which is not built
                (None, 0) => return Err(format!("{path}: this kind of file does not list its frames up front. it does not play yet.")),
                (None, 1) => 1,
                (None, _) => return Err(format!("{path}: its frames do not all last the same. variable frame rates do not play yet.")),
            };
            let size = (video.width, video.height);
            if codec == "prores" {
                // prores decodes in software here, so it need not wait for the graphics device: its first frame is
                // being decoded while the device is made, which takes longer than anything else in a start
                Lane::Video(VideoLane::open(path, video.index, samples, step, size, None)?)
            } else {
                Lane::Waiting(Waiting { path: path.to_string(), stream: video.index, samples, step, size })
            }
        };
        Ok(Clip { name, width: video.width, height: video.height, codec, timeline, lane, first_frame: 0, missing: Vec::new(), rate_assumed: false })
    }

    /// a still, or the numbered run of stills it is a frame of, opened on its frame. the file's own picture gives the
    /// size and codec, and is the first frame shown.
    fn open_stills(path: &str) -> Result<Clip, String> {
        let image = Arc::new(still::decode(path)?);
        let (width, height, codec, rate) = (image.width, image.height, image.codec, image.rate);
        let alone = path.to_string();
        let run = run_of(Path::new(path)).unwrap_or_else(|| Run { name: file_name(path), frames: 1, at: 0, missing: Vec::new(), path_of: Box::new(move |_| alone.clone()) });
        // the file's own rate when it states one (exr), else the design's assumption until the user types another
        let rate_assumed = rate.is_none() && run.frames > 1;
        let (num, den) = rate.unwrap_or((24, 1));
        let timeline = Timeline::even((den, num), run.frames, 1);
        let lane = SeqLane::open(run.missing.clone(), (run.at, image), run.path_of)?;
        Ok(Clip { name: run.name, width, height, codec, timeline, lane: Lane::Stills(lane), first_frame: run.at, missing: run.missing, rate_assumed })
    }

    /// starts decoding, for a clip that was waiting. `hardware` is the renderer's device, when there is one.
    pub fn start(&mut self, hardware: Option<Hardware>) -> Result<(), String> {
        if let Lane::Waiting(w) = &mut self.lane {
            let lane = VideoLane::open(&w.path, w.stream, std::mem::take(&mut w.samples), w.step, w.size, hardware)?;
            self.lane = Lane::Video(lane);
        }
        Ok(())
    }

    /// the frame rate as text: the file's own ratio, the one assumed, or that it has none.
    pub fn rate_text(&self) -> String {
        let text = match self.timeline.rate() {
            Some((n, 1)) => format!("{n} fps"),
            Some((n, d)) => format!("{n}/{d} fps"),
            None if self.timeline.frames() < 2 => "still".to_string(),
            None => "variable rate".to_string(),
        };
        if self.rate_assumed { format!("{text} assumed") } else { text }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::dec::Picture;
    use crate::media::source::Frame;
    use crate::media::still::Samples;
    use std::time::Duration;

    fn media(name: &str) -> String {
        format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn a_clip_reports_what_the_file_says() {
        let c = Clip::open(&media("hapq_64.mov")).expect("open");
        assert_eq!((c.name.as_str(), c.width, c.height, c.codec, c.timeline.frames()), ("hapq_64.mov", 64, 64, "hap q", 3));
        // the rate is the file's own ratio, not a rounded 59.94
        assert_eq!(c.rate_text(), "60000/1001 fps");
        let c = Clip::open(&media("hapalpha_64.mov")).expect("open");
        assert_eq!((c.codec, c.timeline.frames()), ("hap alpha", 2));
        let c = Clip::open(&media("h264_64.mp4")).expect("open");
        assert_eq!((c.codec, c.timeline.frames(), c.rate_text().as_str()), ("h.264", 12, "15360/512 fps"));
        let c = Clip::open(&media("prores_64.mov")).expect("open");
        assert_eq!((c.codec, c.timeline.frames()), ("prores", 4));
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
        c.start(None).expect("start");
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

    /// any frame, asked for in any order, is the same picture as when the clip is decoded straight through: the proof
    /// that going back to a key frame and decoding forward lands on the right frame. the h.264 clip has frames stored
    /// out of display order; the hevc clip has key frames with pictures that are shown before them.
    #[test]
    fn any_frame_in_any_order_is_the_frame_asked_for() {
        for (name, count) in [("h264_64.mp4", 12), ("hevc10_64.mp4", 12), ("prores_64.mov", 4)] {
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
        assert_eq!(cut.rate_text(), "15360/512 fps");
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
        assert_eq!((stills.samples, stills.bits, stills.has_alpha), (Samples::U8, 8, false));
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
        let c = Clip::open(files[1].to_str().unwrap()).expect("open");
        assert_eq!((c.name.as_str(), c.timeline.frames(), c.first_frame, c.rate_text().as_str()), ("shot.[0001-0002].exr", 2, 1, "30000/1001 fps"));
        drop(c);
        for file in &files {
            std::fs::remove_file(file).unwrap();
        }
        std::fs::remove_dir(&dir).unwrap();
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
}
