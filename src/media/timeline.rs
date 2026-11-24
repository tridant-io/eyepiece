//! the exact times of a clip's frames, from the container's own sample table.
//! ffmpeg's summary fields (average rate, frame count) are never used: they round and guess (adr 0003).

use super::ff::Sample;

pub struct Timeline {
    /// seconds per time unit, as a ratio
    pub time_base: (i64, i64),
    /// each frame's time, in time units, in order
    pub times: Vec<i64>,
}

impl Timeline {
    pub fn new(time_base: (i64, i64), samples: &[Sample]) -> Timeline {
        Timeline { time_base, times: samples.iter().map(|s| s.time).collect() }
    }

    /// `frames` frames that each last `step` time units.
    pub fn even(time_base: (i64, i64), frames: usize, step: i64) -> Timeline {
        Timeline { time_base, times: (0..frames as i64).map(|i| i * step).collect() }
    }

    pub fn frames(&self) -> usize {
        self.times.len()
    }

    /// how long every frame lasts, in time units, when they all last the same.
    /// None when the durations vary, or when there are fewer than two frames to measure.
    pub fn step(&self) -> Option<i64> {
        let step = self.times.get(1)? - self.times[0];
        (step > 0 && self.times.windows(2).all(|p| p[1] - p[0] == step)).then_some(step)
    }

    /// the frame rate as the file writes it: time units a second over units a frame.
    /// not reduced, so 60000/1001 stays 60000/1001 and a file written as 2997/100 shows 2997/100.
    pub fn rate(&self) -> Option<(i64, i64)> {
        self.step().map(|step| (self.time_base.1, self.time_base.0 * step))
    }

    /// frames a second, for pacing: the file's own rate, else the one derived, else the average (a clip whose frames
    /// vary is paced by `starts`, and the average only counts loop analysis's seconds in frames).
    pub fn fps(&self) -> f64 {
        if let Some((n, d)) = self.rate() {
            return n as f64 / d as f64;
        }
        if let Some(fps) = self.derived() {
            return fps;
        }
        let (Some(first), Some(last)) = (self.times.first(), self.times.last()) else { return 0.0 };
        let span = (last - first) as f64 * self.time_base.0 as f64 / self.time_base.1 as f64;
        if span > 0.0 { (self.times.len() - 1) as f64 / span } else { 0.0 }
    }

    /// the rate of frames that are evenly spaced but for the rounding of the file's clock: mkv keeps milliseconds,
    /// where 30 fps is stored as 0, 33, 67, 100. the plainest decimal that puts every frame within one tick of the
    /// clock of its stored time, measured from the first. it is not the file's own ratio, so it is called derived.
    /// None when the frames are exactly even, or vary by more than that.
    pub fn derived(&self) -> Option<f64> {
        if self.step().is_some() || self.times.len() < 3 {
            return None;
        }
        // the frame lengths, in ticks, that keep every frame within one tick
        let (mut short, mut long) = (0.0f64, f64::MAX);
        for (i, t) in self.times.iter().enumerate().skip(1) {
            let since = (t - self.times[0]) as f64;
            short = short.max((since - 1.0) / i as f64);
            long = long.min((since + 1.0) / i as f64);
        }
        if short <= 0.0 || short > long {
            return None;
        }
        let ticks = self.time_base.1 as f64 / self.time_base.0 as f64;
        let (slow, fast) = (ticks / long, ticks / short);
        let middle = (slow + fast) / 2.0;
        // 30 rather than 29.994 when either fits: a short clip pins its rate loosely
        let plain = (0..4).map(|places| 10f64.powi(places)).map(|scale| (middle * scale).round() / scale).find(|fps| (slow..=fast).contains(fps));
        Some(plain.unwrap_or(middle))
    }

    /// when each frame starts, in seconds from the first, and then when the last ends (it is taken to last as long as
    /// the one before it): for showing each frame for its own time. None unless the frames vary.
    pub fn starts(&self) -> Option<Vec<f64>> {
        let n = self.times.len();
        if n < 2 || self.step().is_some() || self.derived().is_some() || self.times[n - 1] <= self.times[0] {
            return None;
        }
        let seconds = |t: i64| (t - self.times[0]) as f64 * self.time_base.0 as f64 / self.time_base.1 as f64;
        let mut starts: Vec<f64> = self.times.iter().map(|&t| seconds(t)).collect();
        starts.push(seconds(2 * self.times[n - 1] - self.times[n - 2]));
        Some(starts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline(time_base: (i64, i64), times: &[i64]) -> Timeline {
        Timeline { time_base, times: times.to_vec() }
    }

    #[test]
    fn rate_is_the_files_own_ratio() {
        let t = timeline((1, 60000), &[0, 1001, 2002, 3003]);
        assert_eq!(t.rate(), Some((60000, 1001)));
        assert!((t.fps() - 59.94006).abs() < 1e-4);
    }

    #[test]
    fn rate_is_not_reduced() {
        let t = timeline((1, 2997), &[0, 100, 200]);
        assert_eq!(t.rate(), Some((2997, 100)));
    }

    #[test]
    fn varying_durations_have_no_rate_but_still_pace() {
        let t = timeline((1, 1000), &[0, 40, 90, 120]);
        assert_eq!(t.rate(), None);
        assert!((t.fps() - 25.0).abs() < 1e-9);
    }

    #[test]
    fn one_frame_has_no_rate() {
        let t = timeline((1, 600), &[0]);
        assert_eq!(t.rate(), None);
        assert_eq!(t.fps(), 0.0);
    }

    /// times on a millisecond clock, as mkv keeps them: rounded from an even rate
    fn milliseconds(frames: i64, fps: f64) -> Timeline {
        timeline((1, 1000), &(0..frames).map(|i| (i as f64 * 1000.0 / fps).round() as i64).collect::<Vec<_>>())
    }

    #[test]
    fn times_rounded_to_the_clock_give_a_derived_rate() {
        // 0, 33, 67, 100: no single step, and not a variable rate
        let t = milliseconds(12, 30.0);
        assert_eq!((t.rate(), t.derived(), t.fps()), (None, Some(30.0), 30.0));
        assert_eq!(t.starts(), None);
        // a minute of 60000/1001 pins it to its decimal; a short clip can only say 60
        assert_eq!(milliseconds(3600, 60000.0 / 1001.0).derived(), Some(59.94));
        assert_eq!(milliseconds(10, 60000.0 / 1001.0).derived(), Some(60.0));
        assert_eq!(milliseconds(600, 24000.0 / 1001.0).derived(), Some(23.976));
        // exactly even has its own ratio, and two frames always are
        assert_eq!(timeline((1, 1000), &[0, 40, 80]).derived(), None);
        assert_eq!(timeline((1, 1000), &[0, 33]).rate(), Some((1000, 33)));
    }

    #[test]
    fn frames_that_vary_each_keep_their_own_time() {
        // two ticks of 1/600 s apart, then four: more than the clock's rounding
        let t = timeline((1, 600), &[0, 20, 40, 80, 120]);
        assert_eq!((t.rate(), t.derived()), (None, None));
        // the last lasts as long as the one before it
        assert_eq!(t.starts(), Some(vec![0.0, 20.0 / 600.0, 40.0 / 600.0, 80.0 / 600.0, 120.0 / 600.0, 160.0 / 600.0]));
        assert!((t.fps() - 4.0 / (120.0 / 600.0)).abs() < 1e-9);
    }
}
