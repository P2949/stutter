#![forbid(unsafe_code)]

//! Small domain primitives shared by future `stutter` crates.
//!
//! This crate must remain independent from the main `stutter` application crate.

pub mod clock;
pub mod ids;
pub mod paths;
pub mod reason;
pub mod units;

#[cfg(test)]
mod tests {
    use super::{clock, ids, paths, reason, units};

    #[test]
    fn skeleton_modules_expose_minimal_primitives() {
        let id = ids::StableId::new("candidate/cpu-affinity");
        assert_eq!(id.as_str(), "candidate/cpu-affinity");

        let bytes = units::ByteCount::new(4096);
        assert_eq!(bytes.as_u64(), 4096);

        let duration = units::DurationNanos::new(42);
        assert_eq!(duration.as_u128(), 42);

        let timestamp = clock::UnixNanos::new(123);
        assert_eq!(timestamp.as_u128(), 123);

        let path = paths::LogicalPath::new("runs/latest");
        assert_eq!(path.as_str(), "runs/latest");

        let reason = match reason::ReasonCode::new("policy-denied") {
            Ok(reason) => reason,
            Err(err) => panic!("expected valid reason code, got {err}"),
        };
        assert_eq!(reason.as_str(), "policy-denied");

        assert_eq!(
            reason::ReasonCode::new(" "),
            Err(reason::ReasonCodeError::Empty)
        );
    }
}
