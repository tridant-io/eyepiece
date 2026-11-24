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

    pub fn frames(&self) -> usize {
        self.times.len()
    }

    /// the frame rate as the file writes it, when every frame lasts the same: time units a second over units a frame.
    /// not reduced, so 60000/1001 stays 60000/1001 and a file written as 2997/100 shows 2997/100.
    /// None when the durations vary, or when there are fewer than two frames to measure.
    pub fn rate(&self) -> Option<(i64, i64)> {
        let step = self.times.get(1)? - self.times[0];
        if step <= 0 || self.times.windows(2).any(|p| p[1] - p[0] != step) {
            return None;
        }
        Some((self.time_base.1, self.time_base.0 * step))
    }

    /// frames a second, for pacing. a clip with no single rate is paced by its average.
    pub fn fps(&self) -> f64 {
        if let Some((n, d)) = self.rate() {
            return n as f64 / d as f64;
        }
        let (Some(first), Some(last)) = (self.times.first(), self.times.last()) else { return 0.0 };
        let span = (last - first) as f64 * self.time_base.0 as f64 / self.time_base.1 as f64;
        if span > 0.0 { (self.times.len() - 1) as f64 / span } else { 0.0 }
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
}
