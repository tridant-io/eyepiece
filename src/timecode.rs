//! what a clip's frames are called by the clock: timecode, drop-frame too. platform-neutral.

/// a clip's clock: what its first frame is called, and how the frames after it are counted.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Timecode {
    /// the first frame's place on the clock, in frame numbers from 00:00:00:00
    pub start: u64,
    /// frame numbers a second
    pub base: u32,
    /// drop-frame: the clock passes over two frame numbers a minute (four at 60), but not every tenth minute, to
    /// keep to the time of day at 29.97 and 59.94 frames a second
    pub drop: bool,
    /// the clip's own frames a second, rounded: 59.94 pictures are often counted at 30, two to a frame number
    pub rate: u32,
}

impl Timecode {
    /// a clock counted from 00:00:00:00, for a clip that carries none.
    pub fn counted(rate: u32) -> Timecode {
        Timecode { start: 0, base: rate, drop: false, rate }
    }

    /// a clip's own, as a file writes it: "01:00:00:00", or with a semicolon before the frames for drop-frame.
    /// none when it is not one, or its frames are not under `base`.
    pub fn parse(text: &str, base: u32, rate: u32) -> Option<Timecode> {
        // only a clock of 30 or 60 frame numbers a second can drop any: on another the mark is the file's mistake
        let drop = text.contains(';') && base % 30 == 0;
        let mut parts = text.split([':', ';']).map(|part| part.parse::<u64>().ok());
        let (hours, minutes, seconds, frames) = (parts.next()??, parts.next()??, parts.next()??, parts.next()??);
        if parts.next().is_some() || minutes > 59 || seconds > 59 || frames >= base as u64 || rate == 0 {
            return None;
        }
        let minutes = hours * 60 + minutes;
        let passed = if drop { Self::dropped(base) * (minutes - minutes / 10) } else { 0 };
        Some(Timecode { start: ((minutes * 60 + seconds) * base as u64 + frames).checked_sub(passed)?, base, drop, rate })
    }

    /// how many frame numbers a drop-frame clock passes over each minute: two at 30, four at 60.
    fn dropped(base: u32) -> u64 {
        (base as f64 / 15.0).round() as u64
    }

    /// what the clock says at a frame of the clip.
    pub fn at(&self, frame: usize) -> String {
        let base = self.base as u64;
        let mut n = self.start + frame as u64 * base / self.rate as u64;
        if self.drop {
            let passed = Self::dropped(self.base);
            let (ten_minutes, minute) = (base * 600 - passed * 9, base * 60 - passed);
            n += passed * 9 * (n / ten_minutes) + passed * ((n % ten_minutes).saturating_sub(passed) / minute);
        }
        let mark = if self.drop { ';' } else { ':' };
        format!("{:02}:{:02}:{:02}{mark}{:02}", n / base / 3600 % 24, n / base / 60 % 60, n / base % 60, n % base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counted_from_nothing() {
        let clock = Timecode::counted(24);
        assert_eq!((clock.at(0), clock.at(742), clock.at(24 * 3600)), ("00:00:00:00".to_string(), "00:00:30:22".to_string(), "01:00:00:00".to_string()));
    }

    #[test]
    fn a_files_own_start() {
        let clock = Timecode::parse("10:00:00:00", 25, 25).unwrap();
        assert_eq!((clock.start, clock.drop), (900_000, false));
        assert_eq!((clock.at(0), clock.at(26)), ("10:00:00:00".to_string(), "10:00:01:01".to_string()));
        // past midnight the clock starts again
        assert_eq!(Timecode::parse("23:59:59:24", 25, 25).unwrap().at(1), "00:00:00:00");
    }

    /// 29.97 frames a second on a clock that counts 30: the clock passes over ;00 and ;01 at each minute but the
    /// tenth, and an hour of frames is an hour on the clock.
    #[test]
    fn drop_frame_passes_over_two_numbers_a_minute_but_not_every_tenth() {
        let clock = Timecode { start: 0, base: 30, drop: true, rate: 30 };
        assert_eq!((clock.at(1799), clock.at(1800)), ("00:00:59;29".to_string(), "00:01:00;02".to_string()));
        assert_eq!((clock.at(17_981), clock.at(17_982)), ("00:09:59;29".to_string(), "00:10:00;00".to_string()));
        assert_eq!(clock.at(107_892), "01:00:00;00");
        // as written and back, for every frame of two hours
        for frame in 0..216_000usize {
            let written = clock.at(frame);
            assert_eq!(Timecode::parse(&written, 30, 30).map(|c| c.start), Some(frame as u64), "{written}");
        }
        let own = Timecode::parse("01:00:00;00", 30, 30).unwrap();
        assert_eq!((own.start, own.at(1800)), (107_892, "01:01:00;02".to_string()));
        // at 59.94, four
        let fast = Timecode { start: 0, base: 60, drop: true, rate: 60 };
        assert_eq!((fast.at(3599), fast.at(3600), fast.at(215_784)), ("00:00:59;59".to_string(), "00:01:00;04".to_string(), "01:00:00;00".to_string()));
    }

    /// 59.94 pictures on a clock that counts 30: two frames to a frame number.
    #[test]
    fn a_clock_slower_than_the_pictures_names_two_frames_alike() {
        let clock = Timecode::parse("01:00:00:00", 30, 60).unwrap();
        assert_eq!((clock.at(0), clock.at(1), clock.at(2), clock.at(60)), ("01:00:00:00".to_string(), "01:00:00:00".to_string(), "01:00:00:01".to_string(), "01:00:01:00".to_string()));
    }

    /// only a clock of 30 or 60 frame numbers a second can drop: on any other the mark is the file's mistake, and
    /// its frames are counted plainly from where it says.
    #[test]
    fn a_drop_frame_mark_on_a_clock_that_cannot_drop_counts_plainly() {
        let clock = Timecode::parse("00:01:00;02", 24, 24).unwrap();
        assert_eq!((clock.drop, clock.start), (false, 1442));
        assert_eq!((clock.at(0), clock.at(22), clock.at(1438)), ("00:01:00:02".to_string(), "00:01:01:00".to_string(), "00:02:00:00".to_string()));
    }

    #[test]
    fn what_is_not_a_timecode_is_none() {
        for text in ["", "01:00:00", "01:00:00:30", "01:60:00:00", "01:00:61:00", "a:b:c:d", "01:00:00:00:00"] {
            assert_eq!(Timecode::parse(text, 30, 30), None, "{text}");
        }
    }
}
