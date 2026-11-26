//! which frame belongs on which display refresh. pure arithmetic: frames are chosen by counting refreshes, never by
//! reading a clock (gate 2). speed, shuttle, loop modes and loop analysis follow the prototype player in
//! `docs/design/plan.html`; each rule names the part of it that it comes from.

use std::sync::Arc;

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

/// loop analysis: the least and most seconds each side of the seam (prototype `-ss`). the seconds to start with
/// are a setting.
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

/// frames a reader may keep ready past the one on screen: a reader takes as many as its workers use, four for each,
/// at most 32 (`media::source`)
const AHEAD: usize = 32;
/// how many refreshes ahead to look for them: enough for the slowest speed on the fastest display
const AHEAD_REFRESHES: usize = 480;

/// where a clip is, whether it is running, and how: speed, shuttle, loop mode and loop analysis.
#[derive(Clone)]
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
    /// counts the runs of play: a new one wherever play starts, stops or is sent somewhere. a clip's sound starts
    /// again with each
    run: u32,
    /// a clip whose frames do not all last the same: when each starts, in seconds from the first, then when the last
    /// ends. play then shows each frame for its own time, and the cadence is not used
    starts: Option<Arc<[f64]>>,
    /// the trim: its in frame and its out frame, both kept, each set or not. one alone trims to the clip's end or
    /// start. loop one plays only the trim, and loop analysis plays its out into its in
    trim: (Option<usize>, Option<usize>),
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
            run: 0,
            starts: None,
            trim: (None, None),
        }
    }

    /// for a clip whose frames do not all last the same (`Timeline::starts`): each is shown for its own time. `fps`
    /// is then their average, which counts loop analysis's seconds in frames.
    pub fn with_starts(mut self, starts: Option<Vec<f64>>) -> Transport {
        self.starts = starts.map(Into::into);
        self
    }

    /// the trim's in frame and its out frame, each set or not.
    pub fn trim(&self) -> (Option<usize>, Option<usize>) {
        self.trim
    }

    /// sets the trim, or with neither point clears it. a point the clip has no frame for is not kept. play goes on
    /// in a new run, which takes a clip's sound round the new trim; play that is outside it then goes to its in
    /// frame. loop analysis starts its round again, as it does for new seconds.
    pub fn set_trim(&mut self, mark_in: Option<usize>, mark_out: Option<usize>) {
        let has = |frame: &usize| *frame < self.frames;
        self.trim = (mark_in.filter(has), mark_out.filter(has));
        match self.seam {
            Some(_) => self.set_seam(self.seam),
            None => self.anchor(),
        }
    }

    /// the in point, at the frame on screen. an out point before it is dropped: a trim is never backwards.
    pub fn set_in(&mut self) {
        self.set_trim(Some(self.frame), self.trim.1.filter(|&out| out >= self.frame));
    }

    /// the out point, at the frame on screen, which is kept. an in point after it is dropped.
    pub fn set_out(&mut self) {
        self.set_trim(self.trim.0.filter(|&start| start <= self.frame), Some(self.frame));
    }

    /// what play goes round, as its first frame and how many: the trim in loop one and in loop analysis, the whole
    /// clip with no trim. with looping off the trim has no say.
    fn round(&self) -> (usize, usize) {
        if self.looping == Loop::Off && self.seam.is_none() {
            return (0, self.frames);
        }
        let first = self.trim.0.unwrap_or(0);
        (first, self.trim.1.unwrap_or(self.frames - 1) + 1 - first)
    }

    /// what a looping clip's sound goes round with it: where `round` starts and how long it is, in seconds.
    pub fn round_seconds(&self) -> (f64, f64) {
        let (first, frames) = self.round();
        match &self.starts {
            Some(starts) => (starts[first], starts[first + frames] - starts[first]),
            None => (first as f64 / self.fps, frames as f64 / self.fps),
        }
    }

    /// where the frame on screen starts, in seconds into the clip: where its sound plays from.
    pub fn seconds(&self) -> f64 {
        match &self.starts {
            Some(starts) => starts[self.frame],
            None => self.frame as f64 / self.fps,
        }
    }

    /// a time in seconds into the clip as a place in frames, as `follow` takes it.
    pub fn frame_at(&self, seconds: f64) -> f64 {
        let Some(starts) = &self.starts else { return seconds * self.fps };
        let frame = starts.partition_point(|&s| s <= seconds).clamp(1, self.frames) - 1;
        let length = starts[frame + 1] - starts[frame];
        frame as f64 + if length > 0.0 { (seconds - starts[frame]) / length } else { 0.0 }
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

    /// the frames a reader should keep ready, most urgent first: the one on screen, then the next ones play will
    /// show (whatever the speed, the direction, the loop mode or loop analysis make them). standing still, the ones
    /// a single step either way would show come first, and then what play would show from here: play then starts
    /// on frames that are already there.
    pub fn ahead(&self) -> Vec<usize> {
        let mut frames = vec![self.frame];
        let mut then = self.clone();
        if !self.playing {
            for by in [1, -1] {
                let mut stepped = self.clone();
                stepped.step(by);
                if !frames.contains(&stepped.frame) {
                    frames.push(stepped.frame);
                }
            }
            then.set_playing(true);
            if !frames.contains(&then.frame) {
                frames.push(then.frame);
            }
        }
        for _ in 0..AHEAD_REFRESHES {
            then.tick();
            if !then.playing || frames.len() > AHEAD {
                break;
            }
            if !frames.contains(&then.frame) {
                frames.push(then.frame);
            }
        }
        frames
    }

    pub fn run(&self) -> u32 {
        self.run
    }

    /// a clip with sound takes its time from the sound: `at` is where the sound is, in frames. the two clocks part
    /// slowly, and going by the sound at every refresh would make the cadence stumble, so the display's count is
    /// kept while the sound is within a frame and a half's time of the middle of the frame on screen (more at a
    /// speed that passes over frames). a little past that, one frame is held again or passed over: on a 60 Hz
    /// display a 59.94 clip repeats one frame about every 17 s. far off, play goes on from the frame the sound is in.
    /// true when the picture was moved. standing still, in reverse, shuttling faster and in loop analysis, which
    /// are all silent, the sound has no say.
    pub fn follow(&mut self, at: f64) -> bool {
        // a clip that loops takes its sound round with it, round the trim when there is one: the two are compared
        // round the seam, and a frame held or passed over there is the one across it
        let (first, count) = self.round();
        let (start, frames, round) = (first as f64, count as f64, self.looping == Loop::One);
        let at = if round { start + (at - start).rem_euclid(frames) } else { at.min(frames - 0.5) };
        let rate = if self.shuttle != 0 { 1.0 } else { self.speed };
        let leeway = (self.fps * rate / self.hz + 0.5).max(1.5);
        let apart = self.frame as f64 + 0.5 - at;
        let off = if round { apart - frames * (apart / frames).round() } else { apart };
        if !self.playing || !matches!(self.shuttle, 0 | 1) || self.seam.is_some() || off.abs() <= leeway {
            return false;
        }
        let (first, frame, count) = (first as i64, self.frame as i64, count as i64);
        let step = |by: i64| if round { (first + (frame - first + by).rem_euclid(count)) as usize } else { (frame + by).clamp(0, count - 1) as usize };
        self.frame = match off {
            far if far.abs() > leeway + 1.0 => at.max(0.0) as usize,
            ahead if ahead > 0.0 => step(-1),
            _ => step(1),
        };
        (self.from, self.refreshes) = (self.frame, 0);
        true
    }

    /// plays at the speed, or stops. either ends a shuttle (prototype: space sets `S.sh = 0`).
    pub fn set_playing(&mut self, playing: bool) {
        self.shuttle = 0;
        self.play(playing);
    }

    /// prototype `play()`: stopping ends a shuttle and the pause on black. starting forward from the last frame
    /// of what play goes round starts from its first, except in loop analysis.
    fn play(&mut self, on: bool) {
        let starting = on && !self.playing;
        self.playing = on && self.frames > 1;
        let (first, frames) = self.round();
        if !self.playing {
            self.shuttle = 0;
            self.black = 0;
        } else if starting && self.seam.is_none() && self.shuttle >= 0 && self.frame == first + frames - 1 {
            self.frame = first;
        }
        self.anchor();
    }

    /// counts again from where play is, at the rate in force: a speed paces play as a clip of `speed × fps` would
    /// be, so it keeps a locked cadence locked (0.5× of 59.94 on 60 holds every frame two refreshes). play that is
    /// outside the trim it goes round is put on its in frame first, or backward on its out frame.
    fn anchor(&mut self) {
        let (first, frames) = self.round();
        if self.playing && self.seam.is_none() && !(first..first + frames).contains(&self.frame) {
            self.frame = if self.shuttle < 0 { first + frames - 1 } else { first };
        }
        self.run = self.run.wrapping_add(1);
        self.from = if self.seam.is_some() { self.place } else { self.frame };
        self.refreshes = 0;
        let rate = if self.shuttle != 0 { self.shuttle.unsigned_abs() as f64 } else { self.speed };
        self.cadence = cadence(self.fps * rate, self.hz);
    }

    /// goes to a frame and stops there. out of range lands on the nearest end. in loop analysis a frame inside the
    /// round goes to its place there and any other frame to the round's start, so the first and last frames are the
    /// seam's two sides (prototype `setF`, fig. 13). with a trim they are its in and out frames.
    pub fn go(&mut self, frame: i64) {
        let frame = frame.clamp(0, self.frames as i64 - 1) as usize;
        let (first, frames) = self.round();
        let end = first + frames;
        match self.seam_frames() {
            Some(k) if (end - k..end).contains(&frame) => self.go_place(frame + k - end),
            Some(k) if (first..first + k).contains(&frame) => self.go_place(k + frame - first),
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
    /// at the ends `Loop::One` goes round, the trim when there is one, and `Loop::Off` stops on the end of the clip
    /// it reached (prototype `step()`). loop analysis plays its round and, past the end going forward, pauses on
    /// black for the trip back, then starts the round again from its first place, held for its full count. going
    /// backward it wraps with no black, as the prototype does (it only pauses when `d > 0`).
    pub fn tick(&mut self) {
        if !self.playing {
            return;
        }
        if self.black > 0 {
            self.black -= 1;
            return;
        }
        self.refreshes += 1;
        let moved = self.starts.as_deref().and_then(|starts| self.timed(starts)).unwrap_or_else(|| self.cadence.frames_after(self.refreshes)) as i64;
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
            None if self.looping == Loop::One => {
                // (round the end is no new run: a looping clip's sound goes round by itself)
                let (first, frames) = self.round();
                self.frame = first + (to - first as i64).rem_euclid(frames as i64) as usize;
            }
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

    /// loop analysis: the frames each side of the seam (prototype `seamK`), at least one and at most half the clip,
    /// or half the trim.
    fn seam_frames(&self) -> Option<usize> {
        self.seam.map(|s| ((s * self.fps).round() as usize).min(self.round().1 / 2).max(1))
    }

    /// loop analysis: goes to a place in the round of `2k`. the first `k` places are the clip's last `k` frames and
    /// the next `k` its first `k` (prototype `seamF`): with a trim, the `k` up to its out frame and the `k` from
    /// its in frame.
    fn put(&mut self, place: usize, k: usize) {
        let (first, frames) = self.round();
        self.place = place;
        self.frame = if place < k { first + frames - k + place } else { first + place - k };
    }

    /// what `Cadence::frames_after` is to a clip of one rate, for one whose frames vary: how many frames have gone by
    /// after `refreshes`, along the round play goes (the clip, its trim, or loop analysis's round), each shown from
    /// the refresh nearest its start for its own time. still counted in refreshes, not read from a clock. None for a
    /// round that takes no time.
    fn timed(&self, starts: &[f64]) -> Option<u64> {
        let rate = if self.shuttle != 0 { self.shuttle.unsigned_abs() as f64 } else { self.speed };
        // the round as runs of the clip's frames, (first, how many): all of them, or the last k and then the first k
        let (first, frames) = self.round();
        let runs = match self.seam_frames() {
            Some(k) => [(first + frames - k, k), (first, k)],
            None => [(first, frames), (0, 0)],
        };
        let len = runs[0].1 + runs[1].1;
        // seconds from the round's start to the start of its place `p`
        let at = |p: usize| {
            let first = p.min(runs[0].1);
            starts[runs[0].0 + first] - starts[runs[0].0] + starts[runs[1].0 + p - first] - starts[runs[1].0]
        };
        // how many places, from the first, start where `before` holds
        let count = |before: &dyn Fn(f64) -> bool| {
            let (mut lo, mut hi) = (0, len);
            while lo < hi {
                let mid = (lo + hi) / 2;
                if before(at(mid)) { lo = mid + 1 } else { hi = mid }
            }
            lo
        };
        let round = at(len);
        if round <= 0.0 {
            return None;
        }
        // half a refresh more: a frame comes up on the refresh nearest its start
        let gone = (self.refreshes as f64 + 0.5) * rate / self.hz;
        // where play started from, as a place in the round: outside loop analysis `from` is a frame of the clip
        let started = if self.seam.is_some() { self.from } else { self.from - first };
        let (from, len) = (started as i64, len as i64);
        Some(if self.shuttle >= 0 {
            let to = at(started) + gone;
            let laps = (to / round).floor();
            let place = count(&|t| t <= to - laps * round).saturating_sub(1) as i64;
            laps as i64 * len + place - from
        } else {
            // backward, each frame is shown from its end back to its start
            let to = at(started + 1) - gone;
            let laps = if to > 0.0 { 0.0 } else { (-to / round).floor() + 1.0 };
            let place = count(&|t| t < to + laps * round).saturating_sub(1) as i64;
            laps as i64 * len + from - place
        } as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the seconds each side of the seam the design starts with
    const SEAM_SECONDS: f64 = 3.0;

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

    #[test]
    fn a_clip_with_sound_keeps_to_the_sound() {
        // 60 on 60: a frame a refresh
        let mut t = Transport::new(100, 60.0, 60.0);
        t.set_playing(true);
        let run = t.run();
        // the sound within a frame and a half's time of the middle of the frame on screen: the display's count stands
        assert!(!t.follow(0.0) && !t.follow(1.9));
        (0..5).for_each(|_| t.tick());
        assert_eq!(t.frame(), 5);
        // the sound has fallen behind, as it does at 59.94 on a 60 Hz display: one frame repeats, and play counts on
        // from it in the same run
        assert!(t.follow(3.9));
        assert_eq!(t.frame(), 4);
        t.tick();
        assert_eq!((t.frame(), t.run()), (5, run));
        // far off, play goes on from where the sound is. with looping off, sound that runs past the clip's end
        // moves nothing (a clip that loops cuts its sound at its own length)
        assert!(t.follow(50.2));
        assert_eq!(t.frame(), 50);
        t.set_loop(Loop::Off);
        t.go(98);
        t.shuttle(1);
        t.tick();
        assert_eq!(t.frame(), 99);
        assert!(!t.follow(250.0));
        // standing still the sound has no say
        t.set_playing(false);
        assert!(!t.follow(50.0));
        assert_eq!(t.frame(), 99);
    }

    /// a clip that loops takes its sound round with it: across the seam the two are as near as anywhere else
    #[test]
    fn across_the_seam_of_a_loop_the_sound_is_as_near_as_anywhere() {
        let mut t = Transport::new(100, 60.0, 60.0);
        t.go(99);
        t.set_playing(true);
        // (starting from the last frame starts from the first)
        (0..99).for_each(|_| t.tick());
        assert_eq!(t.frame(), 99);
        let run = t.run();
        // the picture on its last frame, the sound just round the seam; and the other way about
        assert!(!t.follow(0.3));
        t.tick();
        assert_eq!((t.frame(), t.run()), (0, run), "round the end is no new run: the sound goes round by itself");
        assert!(!t.follow(99.6));
        // a frame and a half apart across the seam is still a frame held or passed over, not a jump round the clip
        assert!(t.follow(98.2));
        assert_eq!(t.frame(), 99);
        // with looping off the end is the end
        t.set_loop(Loop::Off);
        t.go(99);
        assert!(!t.follow(0.3) || t.frame() == 99);
        assert_eq!(t.round_seconds(), (0.0, 100.0 / 60.0));
        // a clip whose frames do not all last the same is as long as the end of its last
        assert_eq!(Transport::new(4, 4.0, 60.0).with_starts(Some(vec![0.0, 0.1, 0.5, 0.6, 1.0])).round_seconds(), (0.0, 1.0));
    }

    #[test]
    fn at_double_speed_the_sound_may_be_a_refresh_of_frames_away() {
        let mut t = Transport::new(100, 60.0, 60.0);
        t.set_speed(2.0);
        t.set_playing(true);
        t.tick();
        assert_eq!(t.frame(), 2);
        assert!(!t.follow(0.6) && !t.follow(4.9));
        // a little further off, one frame more is passed over; far off, play goes on from where the sound is
        assert!(t.follow(5.2));
        assert_eq!(t.frame(), 3);
        assert!(t.follow(9.0));
        assert_eq!(t.frame(), 9);
    }

    #[test]
    fn a_run_is_new_wherever_a_clips_sound_must_start_again() {
        let mut t = Transport::new(10, 60.0, 60.0);
        let mut last = t.run();
        let mut new = |t: &Transport| t.run() != std::mem::replace(&mut last, t.run());
        t.set_playing(true);
        assert!(new(&t));
        (0..9).for_each(|_| t.tick());
        assert_eq!(t.frame(), 9);
        assert!(!new(&t));
        // round the end is no new run: a looping clip's sound goes round by itself
        t.tick();
        assert_eq!(t.frame(), 0);
        assert!(!new(&t));
        t.set_speed(2.0);
        assert!(new(&t));
        t.go(3);
        assert!(new(&t));
    }

    /// five frames: three of a 30th of a second, then two of a 15th, so 24 a second on average
    fn varying() -> Transport {
        let starts = [0.0, 1.0, 2.0, 3.0, 5.0, 7.0].map(|s| s / 30.0);
        Transport::new(5, 24.0, 60.0).with_starts(Some(starts.to_vec()))
    }

    #[test]
    fn frames_that_vary_are_each_shown_for_their_own_time() {
        // on 60 Hz: two refreshes each, then four, and round the end to the first
        let mut t = varying();
        t.set_playing(true);
        assert_eq!(seen(&mut t, 17), frames(&[0, 0, 1, 1, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 0, 0, 1]));
        // backward, the same lengths from the other end, and round to the last
        t.go(4);
        t.shuttle(-1);
        assert_eq!(seen(&mut t, 16), frames(&[4, 4, 4, 4, 3, 3, 3, 3, 2, 2, 1, 1, 0, 0, 4, 4]));
        // twice as fast, half as long
        t.go(0);
        t.set_speed(2.0);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 8), frames(&[0, 1, 2, 3, 3, 4, 4, 0]));
    }

    #[test]
    fn loop_analysis_shows_each_frame_of_its_round_for_its_own_time() {
        // half a second at the average rate is more than half the clip: the round is the last two into the first two
        let mut t = varying();
        t.set_seam(Some(0.5));
        assert_eq!(t.seam_place(), Some((0, 4)));
        t.set_playing(true);
        assert_eq!(seen(&mut t, 13), [frames(&[3, 3, 3, 3, 4, 4, 4, 4, 0, 0, 1, 1]), vec![Show::Black]].concat());
    }

    #[test]
    fn the_sound_of_frames_that_vary_is_found_by_their_times() {
        let mut t = varying();
        t.go(4);
        assert!((t.seconds() - 5.0 / 30.0).abs() < 1e-12);
        // halfway through frame 3, which lasts a 15th
        assert!((t.frame_at(4.0 / 30.0) - 3.5).abs() < 1e-9);
        // playing, the sound where the frame on screen is moves nothing
        t.go(0);
        t.set_playing(true);
        (0..6).for_each(|_| t.tick());
        assert_eq!(t.frame(), 3);
        assert!(!t.follow(t.frame_at(0.1)));
        // a clip of one rate counts frames at that rate
        let mut t = Transport::new(100, 60.0, 60.0);
        t.go(30);
        assert_eq!((t.seconds(), t.frame_at(0.25)), (0.5, 15.0));
    }

    #[test]
    fn a_reader_is_told_the_frames_that_come_next() {
        let mut t = Transport::new(10, 60.0, 60.0);
        // standing still: the frame on screen, one step either way, then what play would show from here, so that
        // play starts on frames that are already there
        t.go(5);
        assert_eq!(t.ahead(), [5, 6, 4, 7, 8, 9, 0, 1, 2, 3]);
        t.go(0);
        assert_eq!(t.ahead(), [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        // on the last frame, play starts again from the first
        t.go(9);
        assert_eq!(t.ahead(), [9, 8, 0, 1, 2, 3, 4, 5, 6, 7]);
        // playing: what comes next, round the end
        t.go(8);
        t.set_playing(true);
        assert_eq!(t.ahead(), [8, 9, 0, 1, 2, 3, 4, 5, 6, 7]);
        // backward
        t.go(1);
        t.shuttle(-1);
        assert_eq!(t.ahead(), [1, 0, 9, 8, 7, 6, 5, 4, 3, 2]);
        // twice as fast: only the frames that will be shown
        t.go(0);
        t.set_speed(2.0);
        t.set_playing(true);
        assert_eq!(t.ahead(), [0, 2, 4, 6, 8]);
        // looping off: nothing past the end
        t.set_speed(1.0);
        t.set_loop(Loop::Off);
        t.go(8);
        t.set_playing(true);
        assert_eq!(t.ahead(), [8, 9]);
        // loop analysis: across the seam, from the clip's last frame to its first
        let mut t = Transport::new(100, 10.0, 60.0);
        t.set_seam(Some(0.5));
        t.step(4);
        assert_eq!((t.frame(), t.ahead()), (99, vec![99, 0, 98, 1, 2, 3, 4, 95, 96, 97]));
        t.set_playing(true);
        assert_eq!(t.ahead(), [99, 0, 1, 2, 3, 4, 95, 96, 97, 98]);
        // a single frame has nothing next
        assert_eq!(Transport::new(1, 60.0, 60.0).ahead(), [0]);
        // a trim in loop one: standing in it, what play would show is the trim round; standing outside it, play
        // would start on its in frame
        let mut t = trimmed();
        t.go(5);
        assert_eq!(t.ahead(), [5, 6, 4, 3]);
        t.go(8);
        assert_eq!(t.ahead(), [8, 9, 7, 3, 4, 5, 6]);
    }

    /// ten frames with a trim of frames 3 to 6, standing on its out frame
    fn trimmed() -> Transport {
        let mut t = Transport::new(10, 60.0, 60.0);
        t.go(3);
        t.set_in();
        t.go(6);
        t.set_out();
        t
    }

    #[test]
    fn a_trim_keeps_both_its_frames_and_is_never_backwards() {
        let mut t = trimmed();
        assert_eq!(t.trim(), (Some(3), Some(6)));
        // in set past the out point drops the out point, and out set before the in point drops that
        t.go(8);
        t.set_in();
        assert_eq!(t.trim(), (Some(8), None));
        t.go(2);
        t.set_out();
        assert_eq!(t.trim(), (None, Some(2)));
        // one frame can be both
        t.set_in();
        assert_eq!(t.trim(), (Some(2), Some(2)));
        t.set_trim(None, None);
        assert_eq!(t.trim(), (None, None));
        // a point the clip has no frame for is not kept
        t.set_trim(Some(4), Some(10));
        assert_eq!(t.trim(), (Some(4), None));
        // setting a point stops nothing
        t.go(5);
        t.set_playing(true);
        t.set_out();
        assert!(t.playing());
    }

    #[test]
    fn loop_one_plays_only_the_trim() {
        let mut t = trimmed();
        // on the out frame play starts from the in frame, as it starts from a clip's first frame on its last
        t.set_playing(true);
        assert_eq!(seen(&mut t, 9), frames(&[3, 4, 5, 6, 3, 4, 5, 6, 3]));
        // from outside the trim play starts on its in frame, and backward on its out frame
        t.go(8);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 3), frames(&[3, 4, 5]));
        t.go(0);
        t.shuttle(-1);
        assert_eq!(seen(&mut t, 6), frames(&[6, 5, 4, 3, 6, 5]));
        // faster than the display, what runs past the out frame comes round from the in frame
        t.go(3);
        t.set_speed(2.0);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 5), frames(&[3, 5, 3, 5, 3]));
        // a trim of one frame stays on it
        t.set_speed(1.0);
        t.set_trim(Some(5), Some(5));
        assert_eq!(seen(&mut t, 3), frames(&[5, 5, 5]));
    }

    #[test]
    fn one_point_alone_trims_to_the_clips_end_or_start() {
        let mut t = Transport::new(6, 60.0, 60.0);
        t.go(4);
        t.set_in();
        t.set_playing(true);
        assert_eq!(seen(&mut t, 5), frames(&[4, 5, 4, 5, 4]));
        // the trim set anew while play is outside it takes play to its in frame
        t.set_trim(None, Some(1));
        assert_eq!(seen(&mut t, 4), frames(&[0, 1, 0, 1]));
    }

    #[test]
    fn steps_and_seeks_go_anywhere_and_only_loop_one_plays_the_trim() {
        let mut t = trimmed();
        // a step or a seek stops play, as ever, and lands outside the trim as readily as in it
        t.set_playing(true);
        t.step(-2);
        assert_eq!((t.frame(), t.playing()), (1, false));
        t.go(9);
        assert_eq!(t.frame(), 9);
        // with looping off the trim has no say: play runs to the clip's end, stops, and starts again from its first
        t.set_loop(Loop::Off);
        t.go(5);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 6), frames(&[5, 6, 7, 8, 9, 9]));
        assert!(!t.playing());
        t.set_playing(true);
        assert_eq!(seen(&mut t, 2), frames(&[0, 1]));
        // loop one chosen while play is outside the trim takes it to the in frame
        t.go(8);
        t.set_playing(true);
        t.set_loop(Loop::One);
        assert_eq!(seen(&mut t, 3), frames(&[3, 4, 5]));
    }

    #[test]
    fn loop_analysis_plays_the_trims_out_into_its_in() {
        // a trim of frames 4 to 9 of twenty has room for three each side of the seam
        let mut t = Transport::new(20, 60.0, 60.0);
        t.set_trim(Some(4), Some(9));
        t.set_seam(Some(SEAM_SECONDS));
        assert_eq!((t.frame(), t.seam_place()), (7, Some((0, 6))));
        t.set_playing(true);
        assert_eq!(seen(&mut t, 7), [frames(&[7, 8, 9, 4, 5, 6]), vec![Show::Black]].concat());
        // the out frame and the in frame are the seam's two sides, and a step crosses it
        t.go(9);
        assert_eq!((t.frame(), t.seam_place()), (9, Some((2, 6))));
        t.step(1);
        assert_eq!((t.frame(), t.seam_place()), (4, Some((3, 6))));
        // a frame outside the trim goes to the round's start
        t.go(12);
        assert_eq!((t.frame(), t.seam_place()), (7, Some((0, 6))));
        // a point set starts the round again, round the new trim: frames 5 to 9 have room for two each side
        t.go(5);
        t.set_in();
        assert_eq!((t.trim(), t.frame(), t.seam_place()), ((Some(5), Some(9)), 8, Some((0, 4))));
        // with looping off it is still the trim's seam
        t.set_loop(Loop::Off);
        t.set_seam(Some(SEAM_SECONDS));
        assert_eq!((t.frame(), t.seam_place()), (8, Some((0, 4))));
        // a trim of one frame is both sides of its seam
        t.set_trim(Some(5), Some(5));
        assert_eq!((t.frame(), t.seam_place()), (5, Some((0, 2))));
        t.step(1);
        assert_eq!(t.frame(), 5);
    }

    #[test]
    fn a_clips_sound_goes_round_the_trim_with_it() {
        let mut t = Transport::new(100, 60.0, 60.0);
        t.set_trim(Some(40), Some(59));
        // what its sound goes round: from the start of the in frame to the end of the out frame
        assert_eq!(t.round_seconds(), (40.0 / 60.0, 20.0 / 60.0));
        t.go(40);
        t.set_playing(true);
        let run = t.run();
        (0..19).for_each(|_| t.tick());
        assert_eq!(t.frame(), 59);
        // the picture on the out frame, the sound just round to the in frame; and the other way about
        assert!(!t.follow(40.3));
        t.tick();
        assert_eq!((t.frame(), t.run()), (40, run), "round the trim is no new run: the sound goes round by itself");
        assert!(!t.follow(59.6));
        // a frame and a half apart across the out and the in is a frame held, not a jump out of the trim
        assert!(t.follow(58.2));
        assert_eq!(t.frame(), 59);
        // far off, play goes on from where the sound is
        assert!(t.follow(50.2));
        assert_eq!(t.frame(), 50);
        // a point set is a new run: the sound starts again, round the new trim
        t.set_in();
        assert_ne!(t.run(), run);
        assert_eq!(t.round_seconds(), (50.0 / 60.0, 10.0 / 60.0));
        // with looping off the sound runs to the clip's end
        t.set_loop(Loop::Off);
        assert_eq!(t.round_seconds(), (0.0, 100.0 / 60.0));
    }

    #[test]
    fn a_trim_of_frames_that_vary_shows_each_for_its_own_time() {
        // frame 2 lasts a 30th of a second and frame 3 a 15th: on 60 Hz two refreshes, then four
        let mut t = varying();
        t.set_trim(Some(2), Some(3));
        assert!((t.round_seconds().0 - 2.0 / 30.0).abs() < 1e-12 && (t.round_seconds().1 - 0.1).abs() < 1e-12);
        t.set_playing(true);
        assert_eq!(seen(&mut t, 13), frames(&[2, 2, 3, 3, 3, 3, 2, 2, 3, 3, 3, 3, 2]));
        t.go(3);
        t.shuttle(-1);
        assert_eq!(seen(&mut t, 12), frames(&[3, 3, 3, 3, 2, 2, 3, 3, 3, 3, 2, 2]));
        // loop analysis: the trim's last frame into its first
        t.set_trim(Some(1), Some(3));
        t.set_seam(Some(0.5));
        assert_eq!(t.seam_place(), Some((0, 2)));
        t.set_playing(true);
        assert_eq!(seen(&mut t, 7), [frames(&[3, 3, 3, 3, 1, 1]), vec![Show::Black]].concat());
    }
}
