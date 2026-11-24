//! which frame belongs on which display refresh. pure arithmetic: frames are chosen by counting refreshes, never by
//! reading a clock (gate 2). speed, shuttle, loop modes and loop analysis follow the prototype player in
//! `docs/design/plan.html`; each rule names the part of it that it comes from.

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

/// the speeds `-` and `=` step through (prototype `SL`). its ends are the slowest and fastest speed.
const SPEEDS: [f64; 9] = [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 4.0, 8.0];

/// the fastest shuttle either way (prototype `shuttle()`).
const SHUTTLE_MOST: i32 = 8;

/// loop analysis: the seconds each side of the seam to start with (prototype `seamSec`), and the least and most the
/// field takes (prototype `-ss`).
pub const SEAM_SECONDS: f64 = 3.0;
const SEAM_LEAST: f64 = 0.5;
const SEAM_MOST: f64 = 10.0;

/// the pause on black between rounds of loop analysis (prototype `hold`, 320 ms).
const PAUSE_SECONDS: f64 = 0.32;

/// what play does at the clip's ends (prototype `t.loop`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Loop {
    /// stops on the last frame going forward, or on the first going backward.
    Off,
    /// goes round.
    One,
}

/// what the next refresh shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Show {
    Frame(usize),
    /// the pause between rounds of loop analysis.
    Black,
}

/// where a clip is, whether it is running, and how: speed, shuttle, loop mode and loop analysis.
pub struct Transport {
    frames: usize,
    frame: usize,
    playing: bool,
    fps: f64,
    hz: f64,
    /// the pace at the rate in force
    cadence: Cadence,
    speed: f64,
    /// negative in reverse, 0 when not shuttling
    shuttle: i32,
    looping: Loop,
    /// loop analysis: the seconds each side of the seam while it is on, the place within its round, and the
    /// refreshes of black still to show
    seam: Option<f64>,
    place: usize,
    black: u64,
    /// where play started from (a frame, or a place in loop analysis), and the refreshes counted since
    from: usize,
    refreshes: u64,
}

impl Transport {
    pub fn new(frames: usize, fps: f64, hz: f64) -> Transport {
        Transport {
            frames: frames.max(1),
            frame: 0,
            playing: false,
            fps,
            hz,
            cadence: cadence(fps, hz),
            speed: 1.0,
            shuttle: 0,
            looping: Loop::One,
            seam: None,
            place: 0,
            black: 0,
            from: 0,
            refreshes: 0,
        }
    }

    /// during the pause on black this is the frame the round starts again on, so it can be read meanwhile.
    pub fn frame(&self) -> usize {
        self.frame
    }

    pub fn show(&self) -> Show {
        if self.black > 0 {
            Show::Black
        } else {
            Show::Frame(self.frame)
        }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// plays at the speed, or stops. either ends a shuttle (prototype: space sets `S.sh = 0`).
    pub fn set_playing(&mut self, playing: bool) {
        self.shuttle = 0;
        self.play(playing);
    }

    /// prototype `play()`: stopping ends a shuttle and the pause on black. starting forward from the last frame
    /// starts from the first, except in loop analysis.
    fn play(&mut self, on: bool) {
        let starting = on && !self.playing;
        self.playing = on && self.frames > 1;
        if !self.playing {
            self.shuttle = 0;
            self.black = 0;
        } else if starting && self.seam.is_none() && self.shuttle >= 0 && self.frame == self.frames - 1 {
            self.frame = 0;
        }
        self.anchor();
    }

    /// counts again from where play is, at the rate in force: a speed paces play as a clip of `speed × fps` would
    /// be, so it keeps a locked cadence locked (0.5× of 59.94 on 60 holds every frame two refreshes).
    fn anchor(&mut self) {
        self.from = if self.seam.is_some() { self.place } else { self.frame };
        self.refreshes = 0;
        let rate = if self.shuttle != 0 { self.shuttle.unsigned_abs() as f64 } else { self.speed };
        self.cadence = cadence(self.fps * rate, self.hz);
    }

    /// goes to a frame and stops there. out of range lands on the nearest end. in loop analysis a frame inside the
    /// round goes to its place there and any other frame to the round's start, so the first and last frames are the
    /// seam's two sides (prototype `setF`, fig. 13).
    pub fn go(&mut self, frame: i64) {
        let frame = frame.clamp(0, self.frames as i64 - 1) as usize;
        match self.seam_frames() {
            Some(k) if frame >= self.frames - k => self.go_place(frame + k - self.frames),
            Some(k) if frame < k => self.go_place(k + frame),
            Some(_) => self.go_place(0),
            None => {
                self.frame = frame;
                self.set_playing(false);
            }
        }
    }

    /// loop analysis: goes to a place in the round, as the timebar does (prototype `seekFrac`), and stops there.
    /// past the round lands on its last place. outside loop analysis it only stops.
    pub fn go_place(&mut self, place: usize) {
        if let Some(k) = self.seam_frames() {
            self.put(place.min(2 * k - 1), k);
        }
        self.set_playing(false);
    }

    /// moves by a number of frames and stops there. in loop analysis it moves through the round and wraps at its
    /// ends, so stepping crosses the seam (prototype `step()`).
    pub fn step(&mut self, by: i64) {
        match self.seam_frames() {
            Some(k) => {
                self.put((self.place as i64 + by).rem_euclid(2 * k as i64) as usize, k);
                self.set_playing(false);
            }
            None => self.go(self.frame as i64 + by),
        }
    }

    /// one more refresh has shown what was due: a frame of this clip, or the black. a silent clip is counted in
    /// frames shown, not in time: a refresh the display missed makes the clip late and is never made up by skipping
    /// frames. (a clip with sound will take its time from the audio instead.)
    /// at the ends `Loop::One` goes round and `Loop::Off` stops on the end it reached (prototype `step()`). loop
    /// analysis plays its round and, past the end going forward, pauses on black for the trip back, then starts the
    /// round again from its first place, held for its full count. going backward it wraps with no black, as the
    /// prototype does (it only pauses when `d > 0`).
    pub fn tick(&mut self) {
        if !self.playing {
            return;
        }
        if self.black > 0 {
            self.black -= 1;
            return;
        }
        self.refreshes += 1;
        let moved = self.cadence.frames_after(self.refreshes) as i64;
        let to = self.from as i64 + if self.shuttle < 0 { -moved } else { moved };
        let last = self.frames as i64 - 1;
        match self.seam_frames() {
            // whatever ran past the end is dropped: the round starts on its first place (prototype `t.p = 0`)
            Some(k) if to >= 2 * k as i64 => {
                self.put(0, k);
                self.black = ((PAUSE_SECONDS * self.hz).round() as u64).max(1);
                self.anchor();
            }
            Some(k) => self.put(to.rem_euclid(2 * k as i64) as usize, k),
            None if self.looping == Loop::One => self.frame = to.rem_euclid(self.frames as i64) as usize,
            None if to < 0 || to > last => {
                self.frame = to.clamp(0, last) as usize;
                self.play(false);
            }
            None => self.frame = to as usize,
        }
    }

    /// the speed play runs at, as a multiple of the clip's rate, held to 0.1× to 8× (prototype `SL` and `-sf`).
    /// frames go by at `speed × fps / hz` a refresh, counted from the frame play was on when the speed changed.
    /// while `speed × fps` is no more than the display's rate, every frame is shown, each for more refreshes the
    /// slower it goes. past that, a refresh shows the latest frame due by its count and the frames due before it are
    /// never shown: on 60 Hz a 60 fps clip at 2× shows every other frame, at 1.5× two of every three (0 1 3 4 6),
    /// at 4× one in four.
    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.clamp(SPEEDS[0], SPEEDS[SPEEDS.len() - 1]);
        self.anchor();
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// one stop up the ladder (1) or down it (-1) from wherever the speed is (prototype `-` and `=`).
    pub fn speed_step(&mut self, direction: i32) {
        let s = self.speed;
        let next = if direction > 0 {
            SPEEDS.iter().copied().find(|&x| x > s + 1e-6).unwrap_or(SPEEDS[SPEEDS.len() - 1])
        } else {
            SPEEDS.iter().copied().rev().find(|&x| x < s - 1e-6).unwrap_or(SPEEDS[0])
        };
        self.set_speed(next);
    }

    /// prototype backspace.
    pub fn speed_reset(&mut self) {
        self.set_speed(1.0);
    }

    /// J (-1) and L (1), prototype `shuttle()` and fig. 12: from a stop, plays at 1× that way. each further press the
    /// same way doubles the rate, up to 8×; a press the other way starts again at 1× that way. play at the speed
    /// counts as 1× forward, so L while playing goes to 2×. the shuttle's rate rules over the speed while it lasts.
    pub fn shuttle(&mut self, direction: i32) {
        let dir = direction.signum();
        let now = if !self.playing { 0 } else if self.shuttle == 0 { 1 } else { self.shuttle };
        self.shuttle = if now == 0 || now.signum() != dir { dir } else { (now * 2).clamp(-SHUTTLE_MOST, SHUTTLE_MOST) };
        self.play(true);
    }

    /// K: stops (prototype `play(false)`).
    pub fn shuttle_stop(&mut self) {
        self.set_playing(false);
    }

    /// a multiple of the clip's rate: negative in reverse, 0 when not shuttling.
    pub fn shuttle_rate(&self) -> i32 {
        self.shuttle
    }

    /// choosing a loop mode leaves loop analysis (prototype: the loop menu and `R` set `t.seam = false`).
    pub fn set_loop(&mut self, mode: Loop) {
        self.looping = mode;
        self.set_seam(None);
    }

    pub fn loop_mode(&self) -> Loop {
        self.looping
    }

    /// loop analysis with this many seconds each side of the seam, held to 0.5 to 10, or off. turning it on or
    /// changing the seconds starts the round again from its first place (prototype `seam()`); turning it off leaves
    /// the frame where it is. playing or not is left alone.
    pub fn set_seam(&mut self, seconds: Option<f64>) {
        self.seam = seconds.map(|s| s.clamp(SEAM_LEAST, SEAM_MOST));
        self.black = 0;
        if let Some(k) = self.seam_frames() {
            self.put(0, k);
        }
        self.anchor();
    }

    pub fn seam(&self) -> Option<f64> {
        self.seam
    }

    /// loop analysis: the place within the round and the round's length, in frames, for the timebar.
    pub fn seam_place(&self) -> Option<(usize, usize)> {
        self.seam_frames().map(|k| (self.place, 2 * k))
    }

    /// loop analysis: the frames each side of the seam (prototype `seamK`), at least one and at most half the clip.
    fn seam_frames(&self) -> Option<usize> {
        self.seam.map(|s| ((s * self.fps).round() as usize).min(self.frames / 2).max(1))
    }

    /// loop analysis: goes to a place in the round of `2k`. the first `k` places are the clip's last `k` frames and
    /// the next `k` its first `k` (prototype `seamF`).
    fn put(&mut self, place: usize, k: usize) {
        self.place = place;
        self.frame = if place < k { self.frames - k + place } else { place - k };
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

    /// what the next `n` refreshes show, as the render loop sees them: what is due, then that refresh goes by.
    fn seen(t: &mut Transport, n: usize) -> Vec<Show> {
        (0..n).map(|_| { let s = t.show(); t.tick(); s }).collect()
    }

    fn frames(list: &[usize]) -> Vec<Show> {
        list.iter().map(|&f| Show::Frame(f)).collect()
    }

    #[test]
    fn half_speed_shows_every_frame_twice() {
        // 59.94 on 60 stays locked at half speed too: two refreshes a frame, none held for three
        for fps in [60.0, 60000.0 / 1001.0] {
            let mut t = Transport::new(100, fps, 60.0);
            t.set_speed(0.5);
            t.set_playing(true);
            assert_eq!(seen(&mut t, 8), frames(&[0, 0, 1, 1, 2, 2, 3, 3]));
        }
    }

    #[test]
    fn faster_than_the_display_skips_frames() {
        let mut t = Transport::new(100, 60.0, 60.0);
        t.set_speed(2.0);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 5), frames(&[0, 2, 4, 6, 8]));
        t.go(0);
        t.set_speed(1.5);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 6), frames(&[0, 1, 3, 4, 6, 7]));
        // a 30 fps clip at 2× on 60 is no faster than the display: nothing is skipped
        let mut t = Transport::new(100, 30.0, 60.0);
        t.set_speed(2.0);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 4), frames(&[0, 1, 2, 3]));
    }

    #[test]
    fn a_new_speed_counts_from_the_frame_play_is_on() {
        let mut t = Transport::new(100, 60.0, 60.0);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 3), frames(&[0, 1, 2]));
        t.set_speed(0.5);
        assert_eq!(seen(&mut t, 4), frames(&[3, 3, 4, 4]));
    }

    #[test]
    fn speed_steps_along_the_ladder_and_stays_on_it() {
        let mut t = Transport::new(100, 60.0, 60.0);
        let mut up = vec![];
        for _ in 0..5 {
            t.speed_step(1);
            up.push(t.speed());
        }
        assert_eq!(up, [1.5, 2.0, 4.0, 8.0, 8.0]);
        t.speed_reset();
        let mut down = vec![];
        for _ in 0..6 {
            t.speed_step(-1);
            down.push(t.speed());
        }
        assert_eq!(down, [0.75, 0.5, 0.25, 0.1, 0.1, 0.1]);
        // a speed set by hand steps to the stop either side of it
        t.set_speed(0.3);
        t.speed_step(1);
        assert_eq!(t.speed(), 0.5);
        t.set_speed(0.3);
        t.speed_step(-1);
        assert_eq!(t.speed(), 0.25);
        t.set_speed(20.0);
        assert_eq!(t.speed(), 8.0);
        t.set_speed(0.0);
        assert_eq!(t.speed(), 0.1);
    }

    #[test]
    fn shuttle_doubles_up_to_eight() {
        let mut t = Transport::new(1000, 60.0, 60.0);
        t.shuttle(1);
        assert!(t.playing());
        assert_eq!(seen(&mut t, 3), frames(&[0, 1, 2]));
        t.shuttle(1);
        assert_eq!(seen(&mut t, 3), frames(&[3, 5, 7]));
        let mut rates = vec![t.shuttle_rate()];
        for _ in 0..3 {
            t.shuttle(1);
            rates.push(t.shuttle_rate());
        }
        assert_eq!(rates, [2, 4, 8, 8]);
        // the other way starts again at 1×, not half
        t.shuttle(-1);
        assert_eq!(t.shuttle_rate(), -1);
        t.shuttle(-1);
        assert_eq!(t.shuttle_rate(), -2);
        t.shuttle_stop();
        assert!(!t.playing());
        assert_eq!(t.shuttle_rate(), 0);
    }

    #[test]
    fn shuttle_from_play_at_the_speed_goes_to_double() {
        let mut t = Transport::new(100, 60.0, 60.0);
        t.set_playing(true);
        t.shuttle(1);
        assert_eq!(t.shuttle_rate(), 2);
    }

    #[test]
    fn shuttle_rules_over_the_speed_while_it_lasts() {
        let mut t = Transport::new(100, 60.0, 60.0);
        t.set_speed(0.5);
        t.shuttle(1);
        assert_eq!(seen(&mut t, 3), frames(&[0, 1, 2]));
        // play at the speed ends the shuttle and goes back to the speed
        t.set_playing(true);
        assert_eq!(t.shuttle_rate(), 0);
        assert_eq!(seen(&mut t, 4), frames(&[3, 3, 4, 4]));
        assert_eq!(t.speed(), 0.5);
    }

    #[test]
    fn reverse_goes_round_with_loop_one() {
        let mut t = Transport::new(4, 60.0, 60.0);
        t.go(1);
        t.shuttle(-1);
        assert_eq!(seen(&mut t, 5), frames(&[1, 0, 3, 2, 1]));
        assert!(t.playing());
    }

    #[test]
    fn reverse_stops_on_the_first_frame_with_loop_off() {
        let mut t = Transport::new(4, 60.0, 60.0);
        t.set_loop(Loop::Off);
        t.go(1);
        t.shuttle(-1);
        assert_eq!(seen(&mut t, 3), frames(&[1, 0, 0]));
        assert!(!t.playing());
        assert_eq!(t.shuttle_rate(), 0);
    }

    #[test]
    fn loop_off_stops_on_the_last_frame_and_plays_again_from_the_first() {
        let mut t = Transport::new(3, 60.0, 60.0);
        t.set_loop(Loop::Off);
        assert_eq!(t.loop_mode(), Loop::Off);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 4), frames(&[0, 1, 2, 2]));
        assert!(!t.playing());
        t.set_playing(true);
        assert_eq!(seen(&mut t, 2), frames(&[0, 1]));
    }

    #[test]
    fn play_from_the_last_frame_starts_from_the_first() {
        let mut t = Transport::new(4, 60.0, 60.0);
        assert_eq!(t.loop_mode(), Loop::One);
        t.go(3);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 2), frames(&[0, 1]));
        // backward from the first frame does not jump
        t.go(0);
        t.shuttle(-1);
        assert_eq!(seen(&mut t, 2), frames(&[0, 3]));
    }

    #[test]
    fn a_still_does_not_shuttle_or_break_loop_analysis() {
        let mut t = Transport::new(1, 0.0, 60.0);
        t.shuttle(1);
        assert!(!t.playing());
        assert_eq!(t.shuttle_rate(), 0);
        t.set_seam(Some(SEAM_SECONDS));
        assert_eq!(t.seam_place(), Some((0, 2)));
        assert_eq!(t.show(), Show::Frame(0));
    }

    #[test]
    fn loop_analysis_plays_the_end_into_the_start_then_pauses_on_black() {
        // eight frames is shorter than three seconds each side, so the round is the last four into the first four
        let mut t = Transport::new(8, 60.0, 60.0);
        t.set_seam(Some(SEAM_SECONDS));
        assert_eq!(t.seam_place(), Some((0, 8)));
        t.set_playing(true);
        // the seam, 7 to 0, is crossed with no black
        assert_eq!(seen(&mut t, 8), frames(&[4, 5, 6, 7, 0, 1, 2, 3]));
        // 320 ms of black at 60 Hz, while the frame the round starts on can be read
        assert_eq!(t.frame(), 4);
        assert_eq!(t.seam_place(), Some((0, 8)));
        assert_eq!(seen(&mut t, 19), vec![Show::Black; 19]);
        // then the round again from its first place
        assert_eq!(seen(&mut t, 3), frames(&[4, 5, 6]));
    }

    #[test]
    fn loop_analysis_counts_its_seconds_in_the_clips_frames() {
        let mut t = Transport::new(100, 60.0, 60.0);
        t.set_seam(Some(0.5));
        assert_eq!(t.seam_place(), Some((0, 60)));
        assert_eq!(t.frame(), 70);
        t.set_seam(Some(0.1));
        assert_eq!(t.seam(), Some(0.5));
        t.set_seam(Some(30.0));
        assert_eq!(t.seam(), Some(10.0));
        assert_eq!(t.seam_place(), Some((0, 100)));
    }

    #[test]
    fn a_fast_round_drops_what_ran_past_its_end() {
        let mut t = Transport::new(8, 60.0, 60.0);
        t.set_seam(Some(SEAM_SECONDS));
        t.set_speed(2.0);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 4), frames(&[4, 6, 0, 2]));
        assert_eq!(t.show(), Show::Black);
        assert_eq!(t.frame(), 4);
    }

    #[test]
    fn a_pause_during_the_black_ends_it() {
        let mut t = Transport::new(8, 60.0, 60.0);
        t.set_seam(Some(SEAM_SECONDS));
        t.set_playing(true);
        seen(&mut t, 8);
        assert_eq!(t.show(), Show::Black);
        t.set_playing(false);
        assert_eq!(t.show(), Show::Frame(4));
        t.set_playing(true);
        assert_eq!(seen(&mut t, 2), frames(&[4, 5]));
    }

    #[test]
    fn loop_analysis_backward_wraps_with_no_black() {
        let mut t = Transport::new(8, 60.0, 60.0);
        t.set_seam(Some(SEAM_SECONDS));
        t.go(0);
        t.shuttle(-1);
        assert_eq!(seen(&mut t, 6), frames(&[0, 7, 6, 5, 4, 3]));
    }

    #[test]
    fn stepping_and_seeking_in_loop_analysis() {
        // 0.5 s at 8 fps: four frames each side of the seam, of ten
        let mut t = Transport::new(10, 8.0, 60.0);
        t.set_seam(Some(0.5));
        assert_eq!(t.seam_place(), Some((0, 8)));
        // the last frame and the first are the seam's two sides, and a step crosses it
        t.go(9);
        assert_eq!((t.frame(), t.seam_place()), (9, Some((3, 8))));
        t.step(1);
        assert_eq!((t.frame(), t.seam_place()), (0, Some((4, 8))));
        t.go(0);
        t.step(-1);
        assert_eq!(t.frame(), 9);
        // a frame outside the round goes to its start, and a step back from there wraps to its end
        t.go(5);
        assert_eq!((t.frame(), t.seam_place()), (6, Some((0, 8))));
        t.step(-1);
        assert_eq!((t.frame(), t.seam_place()), (3, Some((7, 8))));
        // the timebar's places, past the end landing on the last
        t.go_place(5);
        assert_eq!(t.frame(), 1);
        t.go_place(100);
        assert_eq!((t.frame(), t.seam_place()), (3, Some((7, 8))));
        // a step stops play
        t.set_playing(true);
        t.step(1);
        assert!(!t.playing());
    }

    #[test]
    fn choosing_a_loop_mode_leaves_loop_analysis() {
        let mut t = Transport::new(100, 60.0, 60.0);
        t.set_seam(Some(SEAM_SECONDS));
        t.set_loop(Loop::Off);
        assert_eq!((t.seam(), t.seam_place()), (None, None));
        // the frame stays where loop analysis left it
        assert_eq!(t.frame(), 50);
    }
}
