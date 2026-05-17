use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime},
};

use serde::{Deserialize, Serialize};

/// Source of monotonic and Unix wall-clock time.
pub trait Clock {
    fn now(&self) -> Instant;

    fn unix_time(&self) -> SystemTime;
}

/// Clock backed by `std::time`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_time(&self) -> SystemTime {
        SystemTime::now()
    }
}

#[derive(Clone, Debug)]
pub struct ManualClock {
    state: Arc<Mutex<ManualClockState>>,
}

#[derive(Clone, Copy, Debug)]
struct ManualClockState {
    now: Instant,
    unix_time: SystemTime,
}

impl ManualClock {
    pub fn new(now: Instant, unix_time: SystemTime) -> Self {
        Self {
            state: Arc::new(Mutex::new(ManualClockState { now, unix_time })),
        }
    }

    pub fn from_unix_time(unix_time: SystemTime) -> Self {
        Self::new(Instant::now(), unix_time)
    }

    pub fn set(&self, now: Instant, unix_time: SystemTime) {
        let mut state = self.state();
        state.now = now;
        state.unix_time = unix_time;
    }

    pub fn set_unix_time(&self, unix_time: SystemTime) {
        self.state().unix_time = unix_time;
    }

    pub fn advance(&self, duration: Duration) {
        let mut state = self.state();
        state.now += duration;
        state.unix_time += duration;
    }

    fn state(&self) -> MutexGuard<'_, ManualClockState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.state().now
    }

    fn unix_time(&self) -> SystemTime {
        self.state().unix_time
    }
}

/// Unix timestamp represented as nanoseconds.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct UnixNanos(u128);

impl UnixNanos {
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    pub const fn as_u128(self) -> u128 {
        self.0
    }
}

/// Monotonic timestamp represented as nanoseconds.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct MonotonicNanos(u128);

impl MonotonicNanos {
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    pub const fn as_u128(self) -> u128 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, UNIX_EPOCH};

    use super::{Clock, ManualClock, MonotonicNanos, SystemClock, UnixNanos};

    #[test]
    fn system_clock_returns_current_time() {
        let clock = SystemClock;
        let _now = clock.now();
        let unix_time = clock.unix_time();

        assert!(
            unix_time.duration_since(UNIX_EPOCH).is_ok(),
            "system clock should return a Unix time after the Unix epoch"
        );
    }

    #[test]
    fn manual_clock_returns_deterministic_time_and_advances() {
        let start_instant = Instant::now();
        let start_unix_time = UNIX_EPOCH + Duration::from_secs(1_000);
        let clock = ManualClock::new(start_instant, start_unix_time);

        assert_eq!(clock.now(), start_instant);
        assert_eq!(clock.unix_time(), start_unix_time);

        clock.advance(Duration::from_millis(250));

        assert_eq!(clock.now(), start_instant + Duration::from_millis(250));
        assert_eq!(
            clock.unix_time(),
            start_unix_time + Duration::from_millis(250)
        );
    }

    #[test]
    fn manual_clock_can_set_times() {
        let clock = ManualClock::from_unix_time(UNIX_EPOCH);
        let next_instant = Instant::now() + Duration::from_secs(5);
        let next_unix_time = UNIX_EPOCH + Duration::from_secs(5);

        clock.set(next_instant, next_unix_time);

        assert_eq!(clock.now(), next_instant);
        assert_eq!(clock.unix_time(), next_unix_time);

        let later_unix_time = UNIX_EPOCH + Duration::from_secs(10);
        clock.set_unix_time(later_unix_time);

        assert_eq!(clock.now(), next_instant);
        assert_eq!(clock.unix_time(), later_unix_time);
    }

    #[test]
    fn timestamp_wrappers_store_raw_values() {
        let unix = UnixNanos::new(123);
        assert_eq!(unix.as_u128(), 123);

        let monotonic = MonotonicNanos::new(456);
        assert_eq!(monotonic.as_u128(), 456);
    }
}
