//! which frame belongs on which display refresh. pure arithmetic: frames are chosen by counting refreshes, never by
//! reading a clock (gate 2).

/// how a silent clip's frames fall on the display's refreshes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cadence {
    /// the display rate is a whole multiple of the clip's (60 on 60, 30 on 60, and 59.94 on 60, which then runs a
    /// thousandth fast): every frame is held for exactly this many refreshes and none is repeated or skipped.
    Locked(u64),
    /// the rates do not divide (24 on 60): frames are held for uneven runs of refreshes. the value is frames a refresh.
    Uneven(f64),
}

/// rates within this fraction of a whole multiple are treated as that multiple.
const LOCK_WITHIN: f64 = 0.005;

pub fn cadence(fps: f64, hz: f64) -> Cadence {
    let per_frame = hz / fps;
    let whole = per_frame.round();
    if whole >= 1.0 && ((per_frame - whole) / whole).abs() < LOCK_WITHIN {
        Cadence::Locked(whole as u64)
    } else {
        Cadence::Uneven(fps / hz)
    }
}

impl Cadence {
    /// how many frames have gone by after `refreshes` refreshes.
    pub fn frames_after(self, refreshes: u64) -> u64 {
        match self {
            Cadence::Locked(n) => refreshes / n,
            Cadence::Uneven(per_refresh) => (refreshes as f64 * per_refresh).floor() as u64,
        }
    }
}

/// where a clip is and whether it is running. the clip goes round at its end.
pub struct Transport {
    frames: usize,
    frame: usize,
    playing: bool,
    cadence: Cadence,
    /// the frame play started from, and the refreshes counted since
    from: usize,
    refreshes: u64,
}

impl Transport {
    pub fn new(frames: usize, fps: f64, hz: f64) -> Transport {
        Transport { frames: frames.max(1), frame: 0, playing: false, cadence: cadence(fps, hz), from: 0, refreshes: 0 }
    }

    pub fn frame(&self) -> usize {
        self.frame
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing && self.frames > 1;
        self.from = self.frame;
        self.refreshes = 0;
    }

    /// goes to a frame and stops there. out of range lands on the nearest end.
    pub fn go(&mut self, frame: i64) {
        self.frame = frame.clamp(0, self.frames as i64 - 1) as usize;
        self.set_playing(false);
    }

    /// moves by a number of frames and stops there.
    pub fn step(&mut self, by: i64) {
        self.go(self.frame as i64 + by);
    }

    /// one more refresh has shown a frame of this clip. a silent clip is counted in frames shown, not in time: a
    /// refresh the display missed makes the clip late and is never made up by skipping frames.
    /// (a clip with sound will take its time from the audio instead.)
    pub fn tick(&mut self) {
        if self.playing {
            self.refreshes += 1;
            self.frame = (self.from + self.cadence.frames_after(self.refreshes) as usize) % self.frames;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_rates_lock_one_to_one() {
        assert_eq!(cadence(60.0, 60.0), Cadence::Locked(1));
        // a display that measures a hair off 60 still shows a 60 fps clip frame for frame
        assert_eq!(cadence(60.0, 59.9964), Cadence::Locked(1));
        // and 59.94 on 60 is locked too: every frame once, a thousandth fast, nothing repeated
        assert_eq!(cadence(60000.0 / 1001.0, 60.0), Cadence::Locked(1));
    }

    #[test]
    fn whole_multiples_lock() {
        assert_eq!(cadence(30.0, 60.0), Cadence::Locked(2));
        assert_eq!(cadence(24.0, 120.0), Cadence::Locked(5));
    }

    #[test]
    fn rates_that_do_not_divide_are_uneven() {
        let c = cadence(24.0, 60.0);
        assert_eq!(c, Cadence::Uneven(0.4));
        // three refreshes, then two, then three: the usual 3:2
        let shown: Vec<u64> = (0..10).map(|r| c.frames_after(r)).collect();
        assert_eq!(shown, [0, 0, 0, 1, 1, 2, 2, 2, 3, 3]);
    }

    #[test]
    fn a_clip_faster_than_the_display_is_uneven() {
        assert_eq!(cadence(120.0, 60.0), Cadence::Uneven(2.0));
    }

    #[test]
    fn playing_advances_and_goes_round() {
        let mut t = Transport::new(4, 60.0, 60.0);
        t.set_playing(true);
        t.tick();
        assert_eq!(t.frame(), 1);
        t.tick();
        t.tick();
        assert_eq!(t.frame(), 3);
        t.tick();
        assert_eq!(t.frame(), 0);
    }

    #[test]
    fn a_slower_clip_holds_each_frame() {
        let mut t = Transport::new(100, 30.0, 60.0);
        t.set_playing(true);
        let shown: Vec<usize> = (0..5).map(|_| { t.tick(); t.frame() }).collect();
        assert_eq!(shown, [0, 1, 1, 2, 2]);
    }

    #[test]
    fn stepping_stops_and_stays_in_range() {
        let mut t = Transport::new(10, 60.0, 60.0);
        t.set_playing(true);
        t.step(-1);
        assert!(!t.playing());
        assert_eq!(t.frame(), 0);
        t.step(25);
        assert_eq!(t.frame(), 9);
        t.tick();
        assert_eq!(t.frame(), 9);
    }

    #[test]
    fn a_still_does_not_play() {
        let mut t = Transport::new(1, 0.0, 60.0);
        t.set_playing(true);
        assert!(!t.playing());
    }
}
