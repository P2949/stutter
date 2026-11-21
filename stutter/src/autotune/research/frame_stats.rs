//! Tail statistics of one native benchmark run, computed from its verified per-frame times. The
//! 99th-percentile frame time is the preferred primary metric because it sees the stutter an average
//! hides; it is only reported when the run holds enough frames for the percentile to mean something,
//! and a run that does not is explicitly left to the average-FPS fallback, never given a guess.

use serde::{Deserialize, Serialize};

/// Fewer frames than this and the worst 1% is under ten frames: not a distribution tail.
pub const MIN_FRAMES_FOR_TAIL: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameStats {
    pub frames: u64,
    /// Nearest-rank 99th-percentile frame time.
    pub p99_frame_ms: f64,
    /// FPS of the mean of the slowest 1% of frames.
    pub one_percent_low_fps: f64,
}

impl FrameStats {
    /// `None` when the population is too small for a tail, or any time is not finite and positive.
    pub fn of(times_ms: &[f64]) -> Option<Self> {
        if times_ms.len() < MIN_FRAMES_FOR_TAIL
            || times_ms.iter().any(|t| !(t.is_finite() && *t > 0.0))
        {
            return None;
        }
        let mut sorted = times_ms.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let n = sorted.len();
        let rank = (n * 99).div_ceil(100).clamp(1, n);
        let tail = n - (n * 99) / 100;
        let worst = &sorted[n - tail..];
        let worst_mean = worst.iter().sum::<f64>() / worst.len() as f64;
        Some(Self {
            frames: n as u64,
            p99_frame_ms: sorted[rank - 1],
            one_percent_low_fps: 1000.0 / worst_mean,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_is_read_from_the_sorted_frames_by_nearest_rank() {
        // 990 frames at 10 ms and 10 at 40 ms: the 99th percentile is the 990th frame (10 ms) and the
        // slowest 1% (the ten 40 ms frames) averages 40 ms, i.e. 25 fps.
        let mut times = vec![10.0; 990];
        times.extend([40.0; 10]);
        let stats = FrameStats::of(&times).unwrap();
        assert_eq!(stats.frames, 1000);
        assert_eq!(stats.p99_frame_ms, 10.0);
        assert_eq!(stats.one_percent_low_fps, 25.0);
        // One more slow frame moves the 99th percentile into the tail.
        times.push(40.0);
        times.sort_by(|a, b| a.total_cmp(b));
        assert_eq!(FrameStats::of(&times).unwrap().p99_frame_ms, 40.0);
    }

    #[test]
    fn a_small_or_corrupt_population_has_no_tail_rather_than_a_guess() {
        assert_eq!(FrameStats::of(&vec![10.0; MIN_FRAMES_FOR_TAIL - 1]), None);
        let mut times = vec![10.0; 1500];
        times[7] = f64::NAN;
        assert_eq!(FrameStats::of(&times), None);
        times[7] = -1.0;
        assert_eq!(FrameStats::of(&times), None);
    }
}
