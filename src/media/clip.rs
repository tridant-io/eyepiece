//! one open clip: what the container says about it, its exact frame times, and where its frames come from.

use super::ff::{self, Input};
use super::hap_source::HapSource;
use super::timeline::Timeline;

pub struct Clip {
    /// the file's name without its folder
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// what to call the codec, as a person would
    pub codec: &'static str,
    pub timeline: Timeline,
    pub hap: HapSource,
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
    /// reads the header and starts the frame source. the first frame is read; nothing else is.
    pub fn open(path: &str) -> Result<Clip, String> {
        let input = Input::open(path)?;
        let video = input.first_video().ok_or_else(|| format!("{path}: no picture in this file"))?;
        let tag = ff::fourcc(video.tag);
        let codec = hap_variant(&tag).ok_or_else(|| format!("{path}: {} ({tag}) does not play yet. this build plays hap.", video.codec_name))?;
        let mut samples = input.samples(video.index);
        // every hap frame stands alone, so file order is free to differ from time order: time order is what plays
        samples.sort_by_key(|s| s.time);
        let timeline = Timeline::new(video.time_base, &samples);
        let hap = HapSource::open(path, samples, video.width, video.height)?;
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
        Ok(Clip { name, width: video.width, height: video.height, codec, timeline, hap })
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
    use crate::media::hap_source::Frame;
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
    }

    #[test]
    fn frames_arrive_and_the_last_is_reachable() {
        let c = Clip::open(&media("hap_64.mov")).expect("open");
        for frame in [0, 2, 1] {
            c.hap.want(frame, false);
            match c.hap.get(frame, Duration::from_secs(5)) {
                Frame::Ready(blocks) => assert_eq!(blocks.len(), 64 / 4 * (64 / 4) * 8),
                Frame::Pending => panic!("frame {frame} never arrived"),
                Frame::Failed(why) => panic!("frame {frame}: {why}"),
            }
        }
    }

    #[test]
    fn something_that_is_not_media_is_an_error() {
        assert!(Clip::open(&media("hap_64.frame0.rgb")).is_err());
        assert!(Clip::open(&media("no such file.mov")).is_err());
    }

    /// gate 5, against the player's own way of holding a file: a render must be able to overwrite it in place, rename
    /// a new file over it, and delete and recreate it, all while the clip is open.
    #[test]
    fn a_render_can_replace_a_clip_that_is_open() {
        let dir = std::env::temp_dir().join(format!("eyepiece-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (path, beside) = (dir.join("clip.mov"), dir.join("clip.tmp"));
        let original = std::fs::read(media("hap_64.mov")).unwrap();
        std::fs::write(&path, &original).unwrap();
        let clip = Clip::open(path.to_str().unwrap()).expect("open");

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
