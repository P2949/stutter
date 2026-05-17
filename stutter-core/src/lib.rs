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

        let pid = ids::Pid::new(1_234);
        assert_eq!(pid.as_u32(), 1_234);

        let tid = ids::Tid::from(1_235);
        assert_eq!(u32::from(tid), 1_235);

        let cpu = ids::CpuId::new(7);
        assert_eq!(cpu.as_u32(), 7);

        let irq = ids::IrqId::from(42);
        assert_eq!(u32::from(irq), 42);

        let process = ids::ProcessId::from(pid);
        assert_eq!(process.as_u32(), 1_234);

        let task = ids::TaskId::from(ids::Tid::new(1_236));
        assert_eq!(task.as_u32(), 1_236);

        let run = ids::RunId::from("run-001");
        assert_eq!(run.as_str(), "run-001");

        let action = ids::ActionId::new("action/cpu-affinity");
        assert_eq!(action.as_str(), "action/cpu-affinity");

        let experiment = ids::ExperimentId::new("experiment/live-001");
        assert_eq!(experiment.as_str(), "experiment/live-001");

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
