//! Windows for the action-latency fixtures, counted through the real resident observer, so the sealed record a test
//! needs is built by the same collectors and the same cut as production: the attempt's own epochs, over the stretch the
//! receipt says the action was held.

use crate::{
    autotune::research::{
        action_child::ChildTarget,
        resident_observer::{AttemptResident, ResidentObserver},
    },
    metrics::{IntervalRecord, LatencyHistogram},
    process_tree::TaskClass,
    recorder::{CpuFreqRecord, FrameEvent, GpuSample},
};

thread_local! {
    static TICKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// A clock for observers fed by a test loop that has no times of its own. Each attempt starts its own session at unix
/// 5000 and walks it in 20-second steps (to 6190, the end of the synthetic session), so the epochs of every attempt fall
/// across any treated interval a fixture's receipts name.
pub(super) fn clock() -> crate::autotune::research::resident_observer::TestClock {
    crate::autotune::research::resident_observer::TestClock {
        now: || {
            5_000
                + TICKS.with(|ticks| {
                    let next = ticks.get() + 1;
                    ticks.set(next);
                    (next * 20).min(1_190)
                })
        },
        arm: || TICKS.with(|ticks| ticks.set(0)),
    }
}

/// One attempt's finished window as the collectors would count it: an interval event every 20 s of the session, each of
/// 500 runnable events (450 at 0.5 ms and 50 at `tail_ns`), with the frame, GPU and CPU frequency events the guards need.
pub(super) fn fixture_window(
    target: ChildTarget,
    attempt: u32,
    started: u64,
    ended: u64,
    tail_ns: u64,
) -> AttemptResident {
    let mut observer = ResidentObserver::default();
    observer.arm(target, Some(attempt), started + 10);
    let mut histogram = LatencyHistogram::new();
    for _ in 0..450 {
        histogram.record(500_000);
    }
    for _ in 0..50 {
        histogram.record(tail_ns);
    }
    let (mut elapsed, mut at) = (0, started + 20);
    while at < ended {
        elapsed += 20_000;
        observer.observe_intervals_at(
            &[IntervalRecord {
                elapsed_ms: elapsed,
                process_pid: Some(target.pid.as_u32()),
                class: TaskClass::Game,
                samples: 500,
                max_ns: tail_ns,
                histogram: histogram.snapshot(),
                ..Default::default()
            }],
            0,
            at,
        );
        for frame in 0..100 {
            observer.observe_frame_at(
                &FrameEvent {
                    elapsed_ms: elapsed,
                    frametime_ms: 12.0 + f64::from(frame % 3),
                },
                at,
            );
        }
        observer.observe_gpu_at(
            &GpuSample {
                elapsed_ms: elapsed,
                gpu_busy_percent: Some(60),
                gpu_clock_mhz: Some(2_000),
                temp_millidegrees: Some(60_000),
                ..Default::default()
            },
            at,
        );
        observer.observe_cpu_freq_at(
            &CpuFreqRecord {
                elapsed_ms: elapsed,
                cpu: 0,
                freq_khz: 4_000_000,
                timestamp_ns: 0,
            },
            at,
        );
        at += 20;
    }
    observer.end_attempt(ended - 10);
    observer.freeze(ended - 10).remove(0)
}

/// One attempt's raw window as the resident runtime keeps it, for a study in which the planner evaluated the game 600 times
/// and never offered the action: the counters the sealed planner observation is checked against.
pub(super) fn raw_planner_window(
    process: (stutter_core::ids::Pid, u64, u32),
    attempt: u32,
    started: u64,
    ended: u64,
) -> serde_json::Value {
    serde_json::json!({
        "attempt": attempt, "cgroup": "", "pid": process.0, "start_ticks": process.1, "uid": process.2,
        "started_unix": started, "ended_unix": ended, "histogram": [], "max_ns": 0,
        "lost_events": 0, "invalid_intervals": 0, "target_changed": false, "epochs": [],
        "epochs_truncated": false, "cpu_samples": 600, "gpu_samples": 600, "cpu_freq_samples": 600,
        "psi_samples": 600, "unattributed_intervals": 0, "evaluations": 600, "denied_evaluations": 0,
        "affinity_candidates": 0, "cgroup_candidates": 0,
    })
}

/// The raw guard events of `windows`, beside a study, as the resident runtime keeps them.
pub(super) fn write_guard_events(dir: &std::path::Path, windows: &[AttemptResident]) {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(crate::autotune::research::resident_observer::guard_events::FILE);
    let bytes = serde_json::to_vec(
        &crate::autotune::research::resident_observer::guard_events::record(windows),
    )
    .unwrap_or_default();
    // A write that fails leaves no events file, which the replay under test refuses loudly.
    let _ = std::fs::write(&file, bytes);
    let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
}
