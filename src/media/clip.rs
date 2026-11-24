//! one open clip: what the container says about it, its exact frame times, and where its frames come from.

use super::dec::Hardware;
use super::ff::{self, Input, Sample};
use super::hap_source::HapLane;
use super::timeline::Timeline;
use super::video_source::VideoLane;

pub struct Clip {
    /// the file's name without its folder
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// what to call the codec, as a person would
    pub codec: &'static str,
    pub timeline: Timeline,
    pub lane: Lane,
}

/// where a clip's frames come from.
pub enum Lane {
    /// our own hap code: blocks as stored
    Hap(HapLane),
    /// a clip ffmpeg decodes, not started: it waits for the renderer's device, which a hardware decoder writes into
    Waiting(Waiting),
    /// a clip ffmpeg decodes, running
    Video(VideoLane),
}

pub struct Waiting {
    path: String,
    stream: usize,
    samples: Vec<Sample>,
    step: i64,
}

/// hap's variants by their container tag. ffmpeg knows only the first three; the rest it reports as no codec at all.
fn hap_variant(tag: &str) -> Option<&'static str> {
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

impl Clip {
    /// reads the header. a hap clip starts reading at once; any other waits for `start`.
    pub fn open(path: &str) -> Result<Clip, String> {
        let input = Input::open(path)?;
        let video = input.first_video().ok_or_else(|| format!("{path}: no picture in this file"))?;
        let tag = ff::fourcc(video.tag);
        let mut samples = input.samples(video.index);
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
        let (codec, timeline, lane) = if let Some(codec) = hap_variant(&tag) {
            // every hap frame stands alone, so file order is free to differ from time order: time order is what plays
            samples.sort_by_key(|s| s.time);
            let timeline = Timeline::new(video.time_base, &samples);
            (codec, timeline, Lane::Hap(HapLane::open(path, samples, video.width, video.height)?))
        } else {
            let codec = match video.codec_name.as_str() {
                "h264" => "h.264",
                "hevc" => "hevc",
                "prores" => "prores",
                other => return Err(format!("{path}: {other} ({tag}) does not play yet. this build plays hap, prores, h.264 and hevc.")),
            };
            // the samples stay in decode order. their decode times are evenly spaced exactly when the frames are
            let timeline = Timeline::new(video.time_base, &samples);
            let step = match (timeline.step(), samples.len()) {
                (Some(step), _) => step,
                (None, 0) => return Err(format!("{path}: no frames")),
                (None, 1) => 1,
                (None, _) => return Err(format!("{path}: its frames do not all last the same. variable frame rates do not play yet.")),
            };
            (codec, timeline, Lane::Waiting(Waiting { path: path.to_string(), stream: video.index, samples, step }))
        };
        Ok(Clip { name, width: video.width, height: video.height, codec, timeline, lane })
    }

    /// starts decoding, for a clip that was waiting. `hardware` is the renderer's device, when there is one.
    pub fn start(&mut self, hardware: Option<Hardware>) -> Result<(), String> {
        if let Lane::Waiting(w) = &mut self.lane {
            let lane = VideoLane::open(&w.path, w.stream, std::mem::take(&mut w.samples), w.step, hardware)?;
            self.lane = Lane::Video(lane);
        }
        Ok(())
    }

    /// the frame rate as text: the file's own ratio, or that it has none.
    pub fn rate_text(&self) -> String {
        match self.timeline.rate() {
            Some((n, 1)) => format!("{n} fps"),
            Some((n, d)) => format!("{n}/{d} fps"),
            None if self.timeline.frames() < 2 => "still".to_string(),
            None => "variable rate".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::dec::Picture;
    use crate::media::source::Frame;
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
            hap.source.want(frame, false);
            match hap.source.get(frame, Duration::from_secs(5)) {
                Frame::Ready(blocks) => assert_eq!(blocks.len(), 64 / 4 * (64 / 4) * 8),
                Frame::Pending => panic!("frame {frame} never arrived"),
                Frame::Failed(why) => panic!("frame {frame}: {why}"),
            }
        }
    }

    /// a number that changes when a picture's luma changes.
    fn luma(picture: &Picture) -> u64 {
        let layout = picture.layout().expect("layout");
        let (data, stride, rows) = picture.plane(0, layout.chroma_shift);
        let row_bytes = 64 * (layout.stored.texel_bits as usize / 8);
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
                video.source.want(frame, false);
                match video.source.get(frame, Duration::from_secs(10)) {
                    Frame::Ready(picture) => luma(&picture),
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

    #[test]
    fn something_that_is_not_media_is_an_error() {
        assert!(Clip::open(&media("hap_64.frame0.rgb")).is_err());
        assert!(Clip::open(&media("no such file.mov")).is_err());
    }

    /// gate 5, against the player's own way of holding a file: a render must be able to overwrite it in place, rename
    /// a new file over it, and delete and recreate it, all while the clip is open. hap reads through our own handle
    /// and the others through ffmpeg on our handle, so both are held here.
    #[test]
    fn a_render_can_replace_a_clip_that_is_open() {
        for name in ["hap_64.mov", "h264_64.mp4"] {
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
