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
    use crate::clock::Clock;

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

        let nanos = units::Nanoseconds::new(42);
        assert_eq!(
            std::time::Duration::from(nanos),
            std::time::Duration::from_nanos(42)
        );

        let unix_nanos = units::UnixNanoseconds::new(123);
        assert_eq!(unix_nanos.as_u128(), 123);

        let millis = units::Milliseconds::new(250);
        assert_eq!(
            std::time::Duration::from(millis),
            std::time::Duration::from_millis(250)
        );

        let seconds = units::Seconds::new(5);
        assert_eq!(
            std::time::Duration::from(seconds),
            std::time::Duration::from_secs(5)
        );

        let confidence = match units::Confidence::new(0.95) {
            Ok(confidence) => confidence,
            Err(err) => panic!("expected valid confidence, got {err}"),
        };
        assert_eq!(confidence.as_f32(), 0.95);

        let latency = units::LatencyNanoseconds::new(99);
        assert_eq!(
            std::time::Duration::from(latency),
            std::time::Duration::from_nanos(99)
        );

        let timestamp = clock::UnixNanos::new(123);
        assert_eq!(timestamp.as_u128(), 123);

        let monotonic_timestamp = clock::MonotonicNanos::new(456);
        assert_eq!(monotonic_timestamp.as_u128(), 456);

        let manual_clock = clock::ManualClock::from_unix_time(std::time::UNIX_EPOCH);
        assert_eq!(manual_clock.unix_time(), std::time::UNIX_EPOCH);

        let path = paths::LogicalPath::new("runs/latest");
        assert_eq!(path.as_str(), "runs/latest");

        let stutter_paths = paths::StutterPaths::new(
            "/var/lib/stutter",
            "/etc/stutter",
            "/var/cache/stutter",
            "/var/lib/stutter/runs",
            "/var/log/stutter/audit.jsonl",
            "/var/lib/stutter/daemon-state.json",
            "/run/stutter/agent.sock",
        );
        assert_eq!(
            stutter_paths.runs_dir,
            std::path::PathBuf::from("/var/lib/stutter/runs")
        );
        assert_eq!(
            stutter_paths.agent_socket,
            std::path::PathBuf::from("/run/stutter/agent.sock")
        );

        let reason_code = match reason::ReasonCode::new("policy-denied") {
            Ok(reason) => reason,
            Err(err) => panic!("expected valid reason code, got {err}"),
        };
        assert_eq!(reason_code.as_str(), "policy-denied");

        let reason = match reason::Reason::from_code(reason_code, "policy denied the candidate") {
            Ok(reason) => reason,
            Err(err) => panic!("expected valid reason, got {err}"),
        };
        assert_eq!(reason.code(), "policy-denied");
        assert_eq!(reason.message(), "policy denied the candidate");

        assert_eq!(
            reason::ReasonCode::new(" "),
            Err(reason::ReasonCodeError::Empty)
        );
        assert_eq!(
            reason::Reason::new("policy-denied", " "),
            Err(reason::ReasonError::EmptyMessage)
        );
    }
}
