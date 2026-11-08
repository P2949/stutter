//! PERF_EVENT_IOC_RESET resets counts only, never enabled/running times. Keep the group
//! continuously enabled and subtract the same two atomic group snapshots for every field.
//! https://man7.org/linux/man-pages/man2/perf_event_open.2.html

pub(super) const WORDS: usize = 9;

pub(super) struct IntervalTracker {
    previous: Option<[u64; WORDS]>,
}

impl IntervalTracker {
    pub(super) fn new(events: usize) -> Self {
        let mut initial = [0; WORDS];
        initial[0] = events as u64;
        Self {
            previous: Some(initial),
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.previous = None;
    }

    pub(super) fn interval<const N: usize>(
        &mut self,
        payload: [u64; N],
        events: usize,
    ) -> Result<[u64; WORDS], String> {
        if events == 0 || events > WORDS - 3 || N > WORDS || N < 3 + events {
            self.invalidate();
            return Err("invalid perf group payload length".into());
        }
        let mut now = [0; WORDS];
        now[..N].copy_from_slice(&payload);
        if now[0] != events as u64 || now[2] > now[1] {
            self.invalidate();
            return Err("invalid perf group shape or cumulative timing".into());
        }
        // Even after a bad interval, this valid snapshot is the next baseline. A missing read
        // needs one priming sample so a later value cannot conceal the measurement gap.
        let before = self
            .previous
            .replace(now)
            .ok_or("perf group re-priming after lost read")?;
        let mut delta = [0; WORDS];
        delta[0] = events as u64;
        for index in 1..3 + events {
            delta[index] = now[index]
                .checked_sub(before[index])
                .ok_or("perf counter or timing went backwards")?;
        }
        if delta[2] > delta[1] {
            return Err("perf interval running time exceeds enabled time".into());
        }
        Ok(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_uses_interval_time_not_lifetime_time() {
        let mut tracker = IntervalTracker::new(3);
        let first = tracker.interval([3, 100, 100, 100, 200, 10, 0], 3).unwrap();
        assert_eq!(&first[..6], &[3, 100, 100, 100, 200, 10]);
        let second = tracker.interval([3, 200, 125, 125, 250, 12, 0], 3).unwrap();
        assert_eq!(&second[..6], &[3, 100, 25, 25, 50, 2]);
        assert_eq!(
            super::super::sample::scale(second[3], second[1], second[2]),
            Some(100)
        );
        // The old lifetime ratio would report 25*200/125 = 40 for this same interval.
    }

    #[test]
    fn missing_and_malformed_reads_require_a_fresh_baseline() {
        let mut tracker = IntervalTracker::new(3);
        tracker.invalidate();
        assert!(tracker.interval([3, 100, 100, 100, 200, 10, 0], 3).is_err());
        assert!(tracker.interval([3, 200, 200, 200, 400, 20, 0], 3).is_ok());
        assert!(tracker.interval([8, 300, 300, 300, 600, 30, 0], 3).is_err());
        assert!(tracker.interval([3, 400, 400, 400, 800, 40, 0], 3).is_err());
        assert!(tracker.interval([3, 500, 500, 500, 1000, 50, 0], 3).is_ok());
    }

    #[test]
    fn reset_wrap_and_impossible_running_time_cannot_be_counts() {
        let mut tracker = IntervalTracker::new(3);
        tracker.interval([3, 100, 100, 100, 200, 10, 0], 3).unwrap();
        assert!(tracker.interval([3, 200, 200, 1, 400, 20, 0], 3).is_err());
        assert!(tracker.interval([3, 300, 400, 200, 600, 30, 0], 3).is_err());
        assert!(tracker.interval([3, 400, 400, 400, 800, 40, 0], 3).is_err());
    }
}
