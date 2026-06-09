use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

/// Byte count wrapper used by shared domain models.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct ByteCount(u64);

impl ByteCount {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Duration wrapper stored as nanoseconds.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct DurationNanos(u128);

impl DurationNanos {
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    pub const fn as_u128(self) -> u128 {
        self.0
    }
}

/// Duration wrapper stored as nanoseconds with `u64` precision.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Nanoseconds(u64);

impl Nanoseconds {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<Nanoseconds> for Duration {
    fn from(value: Nanoseconds) -> Self {
        Duration::from_nanos(value.0)
    }
}

impl TryFrom<Duration> for Nanoseconds {
    type Error = UnitConversionError;

    fn try_from(value: Duration) -> Result<Self, Self::Error> {
        let nanos = value.as_nanos();
        if nanos > u128::from(u64::MAX) {
            return Err(UnitConversionError::TooLarge {
                unit: "nanoseconds",
            });
        }

        Ok(Self::new(nanos as u64))
    }
}

/// Unix timestamp represented as nanoseconds.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct UnixNanoseconds(u128);

impl UnixNanoseconds {
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    pub const fn as_u128(self) -> u128 {
        self.0
    }
}

/// Duration wrapper stored as milliseconds.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Milliseconds(u64);

impl Milliseconds {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<Milliseconds> for Duration {
    fn from(value: Milliseconds) -> Self {
        Duration::from_millis(value.0)
    }
}

impl TryFrom<Duration> for Milliseconds {
    type Error = UnitConversionError;

    fn try_from(value: Duration) -> Result<Self, Self::Error> {
        if !value.subsec_nanos().is_multiple_of(1_000_000) {
            return Err(UnitConversionError::NotWholeMilliseconds);
        }

        let millis = value.as_millis();
        if millis > u128::from(u64::MAX) {
            return Err(UnitConversionError::TooLarge {
                unit: "milliseconds",
            });
        }

        Ok(Self::new(millis as u64))
    }
}

/// Duration wrapper stored as seconds.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Seconds(u64);

impl Seconds {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<Seconds> for Duration {
    fn from(value: Seconds) -> Self {
        Duration::from_secs(value.0)
    }
}

impl TryFrom<Duration> for Seconds {
    type Error = UnitConversionError;

    fn try_from(value: Duration) -> Result<Self, Self::Error> {
        if value.subsec_nanos() != 0 {
            return Err(UnitConversionError::NotWholeSeconds);
        }

        Ok(Self::new(value.as_secs()))
    }
}

/// Confidence score in the inclusive range `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Confidence(f32);

impl Confidence {
    pub fn new(value: f32) -> Result<Self, ConfidenceError> {
        if !value.is_finite() {
            return Err(ConfidenceError::NotFinite);
        }

        if !(0.0..=1.0).contains(&value) {
            return Err(ConfidenceError::OutOfRange { value });
        }

        Ok(Self(value))
    }

    pub const fn as_f32(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Confidence {
    type Error = ConfidenceError;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Confidence> for f32 {
    fn from(value: Confidence) -> Self {
        value.0
    }
}

impl<'de> Deserialize<'de> for Confidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f32::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Latency wrapper stored as nanoseconds.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct LatencyNanoseconds(u64);

impl LatencyNanoseconds {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<LatencyNanoseconds> for Duration {
    fn from(value: LatencyNanoseconds) -> Self {
        Duration::from_nanos(value.0)
    }
}

impl TryFrom<Duration> for LatencyNanoseconds {
    type Error = UnitConversionError;

    fn try_from(value: Duration) -> Result<Self, Self::Error> {
        let nanos = value.as_nanos();
        if nanos > u128::from(u64::MAX) {
            return Err(UnitConversionError::TooLarge {
                unit: "latency nanoseconds",
            });
        }

        Ok(Self::new(nanos as u64))
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum ConfidenceError {
    #[error("confidence must be finite")]
    NotFinite,
    #[error("confidence must be in the range 0.0..=1.0, got {value}")]
    OutOfRange { value: f32 },
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum UnitConversionError {
    #[error("duration is too large to represent as {unit}")]
    TooLarge { unit: &'static str },
    #[error("duration is not an exact whole number of milliseconds")]
    NotWholeMilliseconds,
    #[error("duration is not an exact whole number of seconds")]
    NotWholeSeconds,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        Confidence, ConfidenceError, LatencyNanoseconds, Milliseconds, Nanoseconds, Seconds,
        UnitConversionError, UnixNanoseconds,
    };

    #[test]
    fn duration_units_construct_and_convert_to_duration() {
        let nanos = Nanoseconds::new(42);
        assert_eq!(nanos.as_u64(), 42);
        assert_eq!(Duration::from(nanos), Duration::from_nanos(42));

        let millis = Milliseconds::new(1_500);
        assert_eq!(millis.as_u64(), 1_500);
        assert_eq!(Duration::from(millis), Duration::from_millis(1_500));

        let seconds = Seconds::new(3);
        assert_eq!(seconds.as_u64(), 3);
        assert_eq!(Duration::from(seconds), Duration::from_secs(3));

        let latency = LatencyNanoseconds::new(99);
        assert_eq!(latency.as_u64(), 99);
        assert_eq!(Duration::from(latency), Duration::from_nanos(99));
    }

    #[test]
    fn duration_units_try_from_duration_when_lossless() {
        let nanos = match Nanoseconds::try_from(Duration::from_nanos(42)) {
            Ok(nanos) => nanos,
            Err(err) => panic!("expected valid nanosecond conversion, got {err}"),
        };
        assert_eq!(nanos.as_u64(), 42);

        let millis = match Milliseconds::try_from(Duration::from_millis(250)) {
            Ok(millis) => millis,
            Err(err) => panic!("expected valid millisecond conversion, got {err}"),
        };
        assert_eq!(millis.as_u64(), 250);

        let seconds = match Seconds::try_from(Duration::from_secs(5)) {
            Ok(seconds) => seconds,
            Err(err) => panic!("expected valid second conversion, got {err}"),
        };
        assert_eq!(seconds.as_u64(), 5);

        let latency = match LatencyNanoseconds::try_from(Duration::from_nanos(77)) {
            Ok(latency) => latency,
            Err(err) => panic!("expected valid latency conversion, got {err}"),
        };
        assert_eq!(latency.as_u64(), 77);
    }

    #[test]
    fn duration_units_reject_lossy_duration_conversions() {
        assert_eq!(
            Milliseconds::try_from(Duration::from_nanos(1)),
            Err(UnitConversionError::NotWholeMilliseconds)
        );

        assert_eq!(
            Seconds::try_from(Duration::from_millis(1_500)),
            Err(UnitConversionError::NotWholeSeconds)
        );
    }

    #[test]
    fn unix_nanoseconds_stores_u128_timestamp() {
        let timestamp = UnixNanoseconds::new(123_u128);
        assert_eq!(timestamp.as_u128(), 123);
    }

    #[test]
    fn confidence_accepts_only_finite_unit_range() {
        let zero = match Confidence::new(0.0) {
            Ok(value) => value,
            Err(err) => panic!("expected zero confidence to be valid, got {err}"),
        };
        assert_eq!(zero.as_f32(), 0.0);

        let one = match Confidence::try_from(1.0) {
            Ok(value) => value,
            Err(err) => panic!("expected one confidence to be valid, got {err}"),
        };
        assert_eq!(f32::from(one), 1.0);

        assert_eq!(
            Confidence::new(-0.1),
            Err(ConfidenceError::OutOfRange { value: -0.1 })
        );
        assert_eq!(
            Confidence::new(1.1),
            Err(ConfidenceError::OutOfRange { value: 1.1 })
        );
        assert_eq!(Confidence::new(f32::NAN), Err(ConfidenceError::NotFinite));
    }
}
