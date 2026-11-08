//! A frame source belongs to the target it was launched with, and to no other. These tests drive
//! the real reconcile path with a source attached to this test process as "game A".
use std::path::Path;

use super::*;
use crate::{
    autotune::observation::WorkloadIdentity,
    game_run::LaunchRecord,
    recorder::FrameEvent,
    telemetry::{
        frame_source::{Attribution, MeasurementStage},
        launch_discovery::DiscoveredLaunch,
        managed_source::ManagedFrameSource,
        proc_view::live_process,
        quality::StreamQuality,
    },
};

fn identity(pid: u32, ticks: Option<u64>) -> WorkloadIdentity {
    WorkloadIdentity {
        root_pid: pid,
        process_starttime_ticks: ticks,
        exe_dev: None,
        exe_ino: None,
        cgroup_path: None,
        focus_kind: None,
        class_distribution: Default::default(),
        stable_hash: "h".into(),
    }
}

/// A runtime with a managed source bound to this process, some frames and an attestation.
fn runtime_with_source() -> (AutotuneRuntime, u32, u64) {
    let pid = std::process::id();
    let live = live_process(Path::new("/proc"), pid).unwrap();
    let launch = DiscoveredLaunch {
        dir: std::env::temp_dir(),
        record: LaunchRecord {
            launch_id: "g".into(),
            boot_id: "b".into(),
            uid: crate::syscall::geteuid(),
            pid,
            start_ticks: live.start_ticks,
            argv: vec!["game".into()],
            mangohud_requested: true,
        },
        live: live.clone(),
    };
    let mut runtime = runtime();
    let source = ManagedFrameSource::new(launch, crate::syscall::geteuid());
    runtime.frame_source = Some(source.shared_status());
    runtime.managed_source = Some(source);
    runtime.controller.window.push_frame(FrameEvent {
        elapsed_ms: 1_000,
        frametime_ms: 16.0,
    });
    runtime
        .controller
        .window
        .attest_frame_stream(StreamQuality {
            attribution: Attribution::ValidatedDirect,
            stage: MeasurementStage::ApplicationFrame,
            unexplained_sequence_gaps: 0,
            dropped_frames: 0,
            alignment_uncertainty_ms: 0.5,
            source_healthy: true,
        });
    (runtime, pid, live.start_ticks)
}

#[test]
fn the_source_of_game_a_is_retired_the_moment_the_target_is_game_b() {
    let (mut runtime, pid, ticks) = runtime_with_source();
    // Same target: the source stays.
    runtime.reconcile_frame_source(Some(&identity(pid, Some(ticks))));
    assert!(runtime.managed_source.is_some());
    assert_eq!(runtime.controller.window.frame_count(), 1);

    // Game B (a different pid) is now the target: A's source, frames and attestation are gone
    // before anything can be polled or attested for B.
    runtime.reconcile_frame_source(Some(&identity(pid.wrapping_add(7), Some(ticks))));
    assert!(runtime.managed_source.is_none());
    assert!(runtime.frame_source.is_none());
    assert_eq!(runtime.controller.window.frame_count(), 0);
    assert!(runtime.controller.window.frame_stream().is_none());
}

#[test]
fn a_replaced_process_or_a_lost_identity_also_retires_the_source() {
    // Same pid, different start time: the pid was reused by another process.
    let (mut reused, pid, ticks) = runtime_with_source();
    reused.reconcile_frame_source(Some(&identity(pid, Some(ticks + 1))));
    assert!(reused.managed_source.is_none());

    // No resolved identity at all: no target, so no source.
    let (mut lost, _, _) = runtime_with_source();
    lost.reconcile_frame_source(None);
    assert!(lost.managed_source.is_none());
    assert_eq!(lost.controller.window.frame_count(), 0);

    // A target without a verified start time cannot be matched to a launch.
    let (mut unverified, pid, _) = runtime_with_source();
    unverified.reconcile_frame_source(Some(&identity(pid, None)));
    assert!(unverified.managed_source.is_none());
}
