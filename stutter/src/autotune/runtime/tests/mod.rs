//! Tests for autotune runtime decision-stream and event handling behavior.
//!
//! Owns runtime regression tests and test-only fixtures. Does not own production runtime config,
//! daemon-state mapping, worker/session helpers, or controller orchestration.

use std::fs;

use super::*;
use crate::{
    actions::{RollbackToken, SafetyClass},
    autotune::{
        candidate_memory::CandidateMemoryResult, experiment::WindowScore,
        external_mutation::ExternalMutationPolicy, live_experiment::LiveExperiment,
        objective::ObjectiveSignals, observation::ActiveConfigSnapshot, quality::OnlineDataQuality,
        workload_policy::DaemonWorkloadPolicyConfig,
    },
    daemon::{
        policy::ActionSource,
        state::{DAEMON_STATE_SCHEMA_VERSION, DaemonPhase},
    },
    diagnosis::{Confidence, StutterCause},
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
    process_tree::TaskClass,
    recorder::IntervalRecord,
    scorer::StutterScore,
};

fn runtime() -> AutotuneRuntime {
    let mut config = AutotuneRuntimeConfig::observe(None, Some(1234), None);
    config.history_log = None;
    AutotuneRuntime::new(config)
}
fn low_risk_profile() -> crate::profiles::Profile {
    crate::profiles::Profile {
        name: "game-low-risk".to_owned(),
        rules: vec![crate::profiles::ProfileRule {
            affinity: Some(crate::affinity::CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![crate::process_tree::TaskClass::Game],
            match_comm: Vec::new(),
        }],
    }
}

fn active_task_snapshot(
    tid: u32,
    process_pid: u32,
    comm: &str,
    class: TaskClass,
) -> crate::autotune::observation::ActiveTaskSnapshot {
    crate::autotune::observation::ActiveTaskSnapshot {
        tid: tid.into(),
        process_pid: process_pid.into(),
        comm: comm.to_owned(),
        class,
        process_starttime_ticks: Some(u64::from(process_pid)),
        task_starttime_ticks: Some(u64::from(tid)),
        cgroup_path: None,
    }
}

fn temp_runtime_plan_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "stutter-runtime-dry-run-{name}-{}",
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn high_quality_game_observation_with_focus_confidence(
    focus_confidence: f32,
) -> AutotuneObservation {
    AutotuneObservation {
        now_unix_nanos: 1_000_000_000,
        elapsed_ms: 30_000,
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 1,
        scored_task_count: 1,
        interval_count: 5,
        scored_samples: 100,
        score: StutterScore {
            total: 100,
            over_1ms: 10,
            over_2ms: 5,
            over_5ms: 1,
            ..StutterScore::default()
        },
        data_quality: OnlineDataQuality::High,
        primary_situation: SituationKind::GameCpuSchedulerPressure,
        situation: Default::default(),
        focus_kind: Some(FocusGroupKind::Game),
        focus_confidence,
        focus_roots: vec![1234],
        focus_reasons: vec!["game focus selected".to_owned()],
        recent_diagnoses: Vec::new(),
        system_health: Default::default(),
        capabilities: Default::default(),
        topology_signature: None,
        workload_identity: None,
        active_tasks: Vec::new(),
        protected_tasks: Vec::new(),
        active_config_snapshot: None,
        frame_count: 100,
        frame_p99_ms: 12.0,
        frame_max_ms: 20.0,
        ..AutotuneObservation::default()
    }
}

fn fake_live_experiment(candidate: CandidateAction) -> LiveExperiment {
    LiveExperiment {
        experiment_id: ExperimentId::new("experiment-unknown-active-config"),
        safety_class: candidate.safety_class(),
        mode: DaemonMode::ApplyLowRisk,
        candidate,
        baseline_score: WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 1,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total: 500,
                over_1ms: 10,
                over_2ms: 5,
                over_5ms: 1,
                ..StutterScore::default()
            },
        },
        baseline_signals: ObjectiveSignals::default(),
        baseline_active_config: None,
        applied_unix_nanos: 1_000,
        washout_until_unix_nanos: 2_000,
        measure_until_unix_nanos: 3_000,
        rollback: RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/stutter-test-restore.json"),
            affected_tasks: 1,
        },
    }
}
mod runtime_config;

mod config;

mod daemon_state;

mod emission;

mod lifecycle;

mod planning;

mod restore;
