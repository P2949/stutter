//! Tests for autotune runtime decision-stream and event handling behavior.
//!
//! Owns runtime regression tests and test-only fixtures. Does not own production runtime config,
//! daemon-state mapping, worker/session helpers, or controller orchestration.

use std::fs;

use super::*;
use crate::{
    actions::{RollbackToken, SafetyClass},
    autotune::{
        candidate_memory::CandidateMemoryResult,
        experiment::WindowScore,
        live_experiment::LiveExperiment,
        objective::ObjectiveSignals,
        quality::{OnlineDataQuality, OnlineDataQualityPolicy},
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

#[test]
fn runtime_starts_with_default_observation() {
    let runtime = runtime();
    let observation = runtime.observation();

    assert!(!observation.target_present);
    assert_eq!(observation.target_root_pid, None);
    assert_eq!(observation.score.total, 0);
    assert!(observation.data_quality.blocks_action());
}

#[test]
fn apply_medium_runtime_can_start_reversible_medium_experiment_in_simulation() {
    let daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::ApplyMediumRisk,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    let mut daemon_config = daemon_config;
    daemon_config.autotune.allow_medium_risk_apply = true;
    let mut config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None)
        .with_simulated_action_effects();
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-medium".to_owned()),
        SafetyClass::ReversibleMediumRisk,
    );

    runtime
        .apply_decision_side_effects(
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate,
                reason: "test medium start".to_owned(),
            },
            "test medium start",
        )
        .unwrap();

    assert!(runtime.has_active_experiment());
    assert_eq!(runtime.controller.state.phase, ControllerPhase::Measuring);
    assert_eq!(
        runtime
            .pending_history_context
            .as_ref()
            .map(|context| context.action_kind.as_str()),
        Some("fake")
    );
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
        tid,
        process_pid,
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

#[test]
fn daemon_phase_from_controller_phase_maps_all_controller_phases() {
    let cases = [
        (ControllerPhase::Disabled, DaemonPhase::Disabled),
        (ControllerPhase::Observing, DaemonPhase::Observe),
        (ControllerPhase::Planning, DaemonPhase::Decide),
        (ControllerPhase::Applying, DaemonPhase::Apply),
        (ControllerPhase::Measuring, DaemonPhase::Measure),
        (ControllerPhase::Keeping, DaemonPhase::Keep),
        (ControllerPhase::Reverting, DaemonPhase::Rollback),
        (ControllerPhase::Cooldown, DaemonPhase::Cooldown),
        (ControllerPhase::Faulted, DaemonPhase::Faulted),
    ];

    for (controller_phase, expected_daemon_phase) in cases {
        assert_eq!(
            daemon_phase_from_controller_phase(controller_phase),
            expected_daemon_phase
        );
    }
}

#[test]
fn top_denied_reason_for_plan_prefers_deny_reason_enum() {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-noop".to_owned()),
        SafetyClass::ObserveOnly,
    );
    let descriptor = candidate.descriptor();
    let evaluation = crate::autotune::planner::CandidateEvaluation {
        candidate_name: "fake-noop".to_owned(),
        action_kind: "fake".to_owned(),
        descriptor,
        provider: "test".to_owned(),
        confidence: 1.0,
        eligible: false,
        deny_reasons: vec![crate::autotune::planner::CandidateDenyReason::NoEffectiveChange],
        deny_messages: vec!["candidate would not change active configuration".to_owned()],
        evidence: Vec::new(),
        objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
        rank: Some(1),
        dry_run: None,
        candidate,
    };
    let plan = PlanResult {
        selected: None,
        evaluations: vec![evaluation],
        no_action_reason: None,
    };

    assert_eq!(
        top_denied_reason_for_plan(&plan).as_deref(),
        Some("NoEffectiveChange")
    );
}

#[test]
fn runtime_config_stores_intent_and_permissions_in_daemon_fields() {
    let config =
        AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), Some("Game.exe".to_owned()))
            .with_min_focus_confidence(0.81)
            .with_candidate_window_seconds(45);

    assert_eq!(config.daemon_config.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(config.daemon_config.source, ActionSource::AutotuneRuntime);
    assert_eq!(config.daemon_config.target.tree_pids, vec![1234]);
    assert_eq!(
        config.daemon_config.target.watch_process.as_deref(),
        Some("Game.exe")
    );
    assert!(config.daemon_config.target.require_explicit_target);
    assert_eq!(config.daemon_config.safety.min_confidence, 0.81);
    assert_eq!(config.daemon_config.autotune.candidate_window_seconds, 45);
    assert_eq!(config.daemon_policy.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(
        config.daemon_policy.max_safety_class,
        SafetyClass::ReversibleLowRisk
    );
    assert_eq!(config.daemon_policy.min_confidence, 0.81);
}

#[test]
fn runtime_config_resolves_workload_policy_once_from_daemon_config() {
    let mut daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::Suggest,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    daemon_config.autotune.workload_policy = DaemonWorkloadPolicyConfig {
        rules: vec![crate::autotune::workload_policy::WorkloadPolicyRule {
            situation: SituationKind::BrowserFocused,
            allowed_families: std::collections::BTreeSet::from(["nice".to_owned()]),
            allowed_objectives: std::collections::BTreeSet::from([
                crate::autotune::objective::ObjectiveKind::BrowserInteractivity,
            ]),
            autonomous_families: std::collections::BTreeSet::new(),
        }],
    };

    let runtime_config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None);
    let rule = runtime_config
        .workload_policy
        .rule_for(SituationKind::BrowserFocused);

    assert_eq!(
        rule.allowed_families,
        std::collections::BTreeSet::from(["nice".to_owned()])
    );
    assert!(runtime_config.workload_policy_error.is_none());
}

#[test]
fn runtime_config_records_invalid_workload_policy_error_once() {
    let mut daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::Suggest,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    daemon_config.autotune.workload_policy = DaemonWorkloadPolicyConfig {
        rules: vec![crate::autotune::workload_policy::WorkloadPolicyRule {
            situation: SituationKind::BrowserFocused,
            allowed_families: std::collections::BTreeSet::from(["not_real".to_owned()]),
            allowed_objectives: std::collections::BTreeSet::new(),
            autonomous_families: std::collections::BTreeSet::new(),
        }],
    };

    let runtime_config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None);

    assert!(
        runtime_config
            .workload_policy_error
            .as_deref()
            .unwrap_or_default()
            .contains("unknown workload policy action family")
    );
}

#[test]
fn dry_run_all_safe_runtime_config_requires_suggest_mode() {
    let config =
        AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None).with_dry_run_all_safe(true);

    let err = validate_runtime_config(&config).unwrap_err().to_string();

    assert!(err.contains("--dry-run-all-safe requires suggest mode"));
}

#[test]
fn dry_run_all_safe_writes_plan_files_without_starting_experiment() {
    let plan_dir = temp_runtime_plan_dir("suggest-mode");
    let current_pid = std::process::id();
    let mut config = AutotuneRuntimeConfig::suggest(None, Some(current_pid), None)
        .with_profiles(vec![low_risk_profile()])
        .with_dry_run_all_safe(true)
        .with_dry_run_plan_dir(plan_dir.clone());
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let mut observation = high_quality_game_observation_with_focus_confidence(0.95);
    observation.target_root_pid = Some(current_pid);
    observation.active_target_count = 1;
    observation.active_tasks = vec![active_task_snapshot(
        current_pid,
        current_pid,
        "game",
        TaskClass::Game,
    )];

    let candidate = runtime
        .select_candidate_for_observation(&observation)
        .unwrap();
    let decision = decide_autotune_transition(
        &runtime.controller.policy,
        &runtime.controller.state,
        &observation,
        candidate,
    );

    runtime
        .apply_decision_side_effects(&observation, &decision, "dry-run-all-safe test")
        .unwrap();

    assert!(!runtime.has_active_experiment());
    assert!(runtime.controller.state.active_experiment.is_none());
    assert!(runtime.pending_history_context.is_none());
    assert!(
        !runtime.last_dry_run_plan_files.is_empty(),
        "expected at least one candidate plan file from dry-run planner results"
    );
    for plan in &runtime.last_dry_run_plan_files {
        assert!(plan.path.starts_with(&plan_dir));
        assert!(
            plan.path.exists(),
            "missing plan file {}",
            plan.path.display()
        );
        assert_eq!(plan.safety_class, SafetyClass::ReversibleLowRisk);
    }
}

#[test]
fn runtime_reports_active_experiment_state() {
    let mut runtime = runtime();

    assert!(!runtime.has_active_experiment());

    let candidate = CandidateAction::cpu_affinity_profile(low_risk_profile(), 1234);
    let baseline_score = WindowScore {
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
    };

    runtime
        .live_experiments
        .set_current_for_tests(LiveExperiment {
            experiment_id: ExperimentId::new("experiment-active"),
            safety_class: candidate.safety_class(),
            mode: DaemonMode::ApplyLowRisk,
            candidate,
            baseline_score,
            baseline_signals: ObjectiveSignals::default(),
            baseline_active_config: None,
            applied_unix_nanos: 1_000,
            washout_until_unix_nanos: 2_000,
            measure_until_unix_nanos: 3_000,
            rollback: RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-active-restore.json"),
                affected_tasks: 1,
            },
        });

    assert!(runtime.has_active_experiment());
}

#[test]
fn runtime_rollback_on_stop_noops_without_active_experiment() {
    let mut runtime = runtime();

    let snapshot = runtime.rollback_on_stop("daemon stop").unwrap();

    assert!(snapshot.is_none());
    assert!(!runtime.has_active_experiment());
}

#[test]
fn controller_session_finish_rolls_back_active_experiment_on_clean_stop() {
    let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
        .with_simulated_action_effects();
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    runtime.last_observation = observation.clone();

    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-low-risk-stop".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );

    runtime
        .apply_decision_side_effects(
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate,
                reason: "test start".to_owned(),
            },
            "test start",
        )
        .unwrap();

    assert!(runtime.has_active_experiment());

    let exit =
        finish_autotune_controller_session(&mut runtime, Ok("stop requested".to_owned())).unwrap();

    assert_eq!(exit.reason, "stop requested");
    assert!(!runtime.has_active_experiment());
    assert_eq!(runtime.controller.state.phase, ControllerPhase::Cooldown);
    assert!(runtime.controller.state.active_experiment.is_none());
}

#[test]
fn daemon_state_snapshot_serializes_live_runtime_state() {
    let mut runtime = AutotuneRuntime::new(
        AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
            .with_profiles(vec![low_risk_profile()]),
    );
    let candidate = CandidateAction::cpu_affinity_profile(low_risk_profile(), 1234);
    let baseline_score = WindowScore {
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
    };
    let rollback = RollbackToken::CpuAffinityRestoreFile {
        path: PathBuf::from("/tmp/stutter-restore.json"),
        affected_tasks: 2,
    };

    runtime.target_state = RuntimeTargetState {
        root_pid: Some(1234),
        active_targets: 2,
        target_comm: Some("game".to_owned()),
        active_tasks: BTreeMap::new(),
    };
    runtime.latest_focus = Some(AutotuneObservationFocus {
        kind: FocusGroupKind::Game,
        root_pids: vec![1234],
        member_pids: vec![1234, 1235],
        confidence: 0.95,
        situation: SituationKind::GameCpuSchedulerPressure,
        reasons: vec!["game focus selected".to_owned()],
    });
    runtime.latest_drop_counters = DropCountersSnapshot {
        ringbuf_reserve_failed: 7,
        ..DropCountersSnapshot::default()
    };
    runtime.recent_diagnoses.push_back(LiveDiagnosisEntry {
        elapsed_ms: 12_345,
        cause: StutterCause::GpuBoundCandidate,
        confidence: Confidence::High,
        anchor_class: TaskClass::Game,
        anchor_comm: "game".to_owned(),
        evidence: vec!["gpu busy".to_owned()],
    });
    runtime
        .live_experiments
        .set_current_for_tests(LiveExperiment {
            experiment_id: ExperimentId::new("experiment-1"),
            safety_class: candidate.safety_class(),
            mode: DaemonMode::ApplyLowRisk,
            candidate,
            baseline_score,
            baseline_signals: ObjectiveSignals::default(),
            baseline_active_config: None,
            applied_unix_nanos: 1_000,
            washout_until_unix_nanos: 2_000,
            measure_until_unix_nanos: 3_000,
            rollback,
        });
    runtime.controller.state.phase = ControllerPhase::Faulted;
    runtime.controller.state.cooldown_until_unix_nanos = Some(9_000);

    let mut observation = high_quality_game_observation_with_focus_confidence(0.95);
    observation.data_quality = OnlineDataQuality::Low {
        reasons: vec!["low scored samples".to_owned()],
    };
    observation.drop_counter_total = 7;
    observation.score.total = 999;
    runtime.last_observation = observation;
    runtime.last_decision = Some(AutotuneDecisionStreamEntry {
        unix_nanos: 8_000,
        phase: "Faulted".to_owned(),
        mode: "ApplyLowRisk".to_owned(),
        focus_kind: Some("Game".to_owned()),
        focus_confidence: 0.95,
        target_root_pid: Some(1234),
        active_target_count: 2,
        situation: "GameCpuSchedulerPressure".to_owned(),
        situation_confidence: 0.95,
        situation_evidence: Vec::new(),
        situation_blockers: Vec::new(),
        protected_tasks_count: 0,
        candidate_count: 0,
        top_denied_reason: None,
        planner: None,
        dry_run_plan_files: Vec::new(),
        score_total: 999,
        data_quality: "Low: low scored samples".to_owned(),
        data_quality_reason_codes: vec!["measurement_uncertain".to_owned()],
        decision: "faulted".to_owned(),
        reason: "rollback failed".to_owned(),
    });
    let kept_candidate = CandidateAction::cpu_affinity_profile(low_risk_profile(), 1234);
    runtime.controller.state.record_candidate_result(
        crate::autotune::controller::ControllerCandidateResultInput {
            candidate: &kept_candidate,
            observation: &runtime.last_observation,
            cpu_topology_signature: None,
            result: CandidateMemoryResult::Kept,
            baseline_score_total: Some(500),
            current_score_total: Some(400),
            rollback_reason: None,
            cooldown_expires_unix_nanos: None,
        },
    );

    let snapshot = runtime.daemon_state_snapshot();
    let value = serde_json::to_value(&snapshot).unwrap();

    assert_eq!(snapshot.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(snapshot.phase, DaemonPhase::Faulted);
    assert_eq!(snapshot.cooldown_until_unix_nanos, Some(9_000));
    assert_eq!(
        snapshot
            .active_target
            .as_ref()
            .and_then(|target| target.root_pid),
        Some(1234)
    );
    assert_eq!(
        snapshot
            .active_experiment
            .as_ref()
            .map(|experiment| experiment.experiment_id.as_str()),
        Some("experiment-1")
    );
    assert_eq!(
        snapshot
            .active_rollback
            .as_ref()
            .map(|rollback| rollback.rollback_available),
        Some(true)
    );
    assert_eq!(
        snapshot
            .last_decision
            .as_ref()
            .map(|decision| decision.decision.as_str()),
        Some("faulted")
    );
    assert_eq!(
        snapshot.faulted.as_ref().map(|fault| fault.reason.as_str()),
        Some("rollback failed")
    );
    assert!(
        snapshot
            .degraded
            .iter()
            .any(|status| status.category == "data_quality")
    );
    assert!(snapshot.degraded.iter().any(|status| {
        status.category == "data_quality"
            && status
                .message
                .contains("reason_codes=measurement_uncertain")
    }));
    assert!(
        snapshot
            .degraded
            .iter()
            .any(|status| status.category == "drop_counters")
    );
    assert!(!snapshot.health.ok_for_apply);
    assert_eq!(
        snapshot.health.reason_code.as_deref(),
        Some("drop_counters_high")
    );
    assert!(
        snapshot
            .degraded
            .iter()
            .any(|status| status.category == "cooldown")
    );
    assert!(
        snapshot
            .degraded
            .iter()
            .any(|status| status.category == "recent_diagnosis")
    );

    assert_eq!(value["schema_version"], DAEMON_STATE_SCHEMA_VERSION);
    assert_eq!(value["mode"], "apply-low-risk");
    assert_eq!(value["phase"], "faulted");
    assert_eq!(value["cooldown_until_unix_nanos"].as_u64(), Some(9_000));
    assert_eq!(value["active_target"]["comm"], "game");
    assert_eq!(
        value["active_experiment"]["action_id"],
        "cpu-affinity-profile:game-low-risk"
    );
    assert_eq!(
        value["active_rollback"]["token"]["kind"],
        "cpu-affinity-restore-file"
    );
    assert_eq!(value["last_decision"]["score_total"].as_u64(), Some(999));
    assert_eq!(
        value["active_rollback"]["manual_restore_command"],
        "stutter daemon emergency-restore"
    );
    assert_eq!(
        value["faulted"]["manual_restore_command"],
        "stutter daemon emergency-restore"
    );
    assert_eq!(snapshot.profile_memory.profiles.len(), 1);
    assert_eq!(
        snapshot.profile_memory.profiles[0].candidate_name,
        "game-low-risk"
    );
    assert_eq!(
        snapshot.profile_memory.profiles[0]
            .workload_label
            .as_deref(),
        Some("game")
    );
    assert_eq!(
        value["profile_memory"]["profiles"][0]["action_kind"],
        "cpu_affinity_profile"
    );
}

#[test]
fn candidate_selection_blocks_focus_confidence_below_policy_threshold() {
    let mut runtime = AutotuneRuntime::new(
        AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
            .with_profiles(vec![low_risk_profile()]),
    );
    let observation = high_quality_game_observation_with_focus_confidence(
        runtime.controller.policy.min_focus_confidence - 0.01,
    );

    let candidate = runtime
        .select_candidate_for_observation(&observation)
        .unwrap();

    assert!(candidate.is_none());
}

#[test]
fn runtime_observation_uses_configured_online_data_quality_policy() {
    let mut runtime = AutotuneRuntime::new(
        AutotuneRuntimeConfig::observe(None, Some(1234), None).with_online_data_quality_policy(
            OnlineDataQualityPolicy {
                min_scored_samples: 200,
                ..OnlineDataQualityPolicy::default()
            },
        ),
    );

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        runtime
            .controller
            .window
            .push_interval(crate::recorder::IntervalRecord {
                elapsed_ms,
                task: 42,
                samples: 20,
                over_1ms: 1,
                max_ns: 2_000_000,
                ..Default::default()
            });
    }

    let observation = runtime.build_observation();

    assert_eq!(observation.scored_samples, 100);
    assert!(observation.data_quality.is_low());
    assert!(
        observation
            .data_quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("fewer than min_scored_samples"))
    );
}

#[test]
fn runtime_config_defaults_to_default_online_data_quality_policy() {
    let config = AutotuneRuntimeConfig::observe(None, Some(1234), None);
    let default_policy = OnlineDataQualityPolicy::default();

    assert_eq!(
        config.online_data_quality_policy.min_scored_intervals,
        default_policy.min_scored_intervals
    );
    assert_eq!(
        config.online_data_quality_policy.min_scored_samples,
        default_policy.min_scored_samples
    );
    assert_eq!(
        config.online_data_quality_policy.max_drop_counter_total,
        default_policy.max_drop_counter_total
    );
    assert_eq!(
        config.online_data_quality_policy.frame_data_policy,
        default_policy.frame_data_policy
    );
}

#[test]
fn runtime_config_defaults_and_overrides_washout_policy() {
    let default_config = AutotuneRuntimeConfig::observe(None, Some(1234), None);

    assert_eq!(
        default_config.washout.washout_seconds,
        crate::autotune::washout::DEFAULT_WASHOUT_SECONDS
    );
    assert_eq!(
        default_config.washout.verify_interval_ms,
        crate::autotune::washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS
    );

    let custom_config =
        AutotuneRuntimeConfig::observe(None, Some(1234), None).with_washout(30, 2_000);

    assert_eq!(custom_config.washout.washout_seconds, 30);
    assert_eq!(custom_config.washout.verify_interval_ms, 2_000);

    let clamped_config = AutotuneRuntimeConfig::observe(None, Some(1234), None).with_washout(0, 50);

    assert_eq!(
        clamped_config.washout.washout_seconds,
        crate::autotune::washout::MIN_WASHOUT_SECONDS
    );
    assert_eq!(
        clamped_config.washout.verify_interval_ms,
        crate::autotune::washout::MIN_WASHOUT_VERIFY_INTERVAL_MS
    );
}

#[test]
fn runtime_config_default_min_focus_confidence_matches_controller_default() {
    let config = AutotuneRuntimeConfig::suggest(None, None, None);
    let runtime = AutotuneRuntime::new(config.clone());

    assert_eq!(
        config.daemon_config.safety.min_confidence,
        crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
    );
    assert_eq!(
        runtime.controller.policy.min_focus_confidence,
        crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
    );
}

#[test]
fn runtime_config_custom_min_focus_confidence_updates_controller_policy() {
    let config = AutotuneRuntimeConfig::suggest(None, None, None).with_min_focus_confidence(0.42);
    let runtime = AutotuneRuntime::new(config.clone());

    assert_eq!(config.daemon_config.safety.min_confidence, 0.42);
    assert_eq!(runtime.controller.policy.min_focus_confidence, 0.42);
}

#[test]
fn runtime_config_min_focus_confidence_is_clamped() {
    let low = AutotuneRuntimeConfig::suggest(None, None, None).with_min_focus_confidence(-1.0);
    let high = AutotuneRuntimeConfig::suggest(None, None, None).with_min_focus_confidence(2.0);

    assert_eq!(low.daemon_config.safety.min_confidence, 0.0);
    assert_eq!(high.daemon_config.safety.min_confidence, 1.0);
}

#[test]
fn runtime_washout_policy_delays_live_measurement_deadline() {
    let config = AutotuneRuntimeConfig::observe(None, Some(1234), None)
        .with_candidate_window_seconds(30)
        .with_washout(20, 2_000);
    let applied_unix_nanos = 1_000_000_000_u128;

    let (washout_until_unix_nanos, measure_until_unix_nanos) =
        LiveExperimentManager::deadlines_from_now(
            config.simulate_action_effects,
            &config.washout,
            config.candidate_window_seconds,
            applied_unix_nanos,
        );

    let expected_washout_until =
        applied_unix_nanos.saturating_add(Duration::from_secs(20).as_nanos());
    let expected_measure_until =
        expected_washout_until.saturating_add(Duration::from_secs(30).as_nanos());

    assert_eq!(washout_until_unix_nanos, expected_washout_until);
    assert_eq!(measure_until_unix_nanos, expected_measure_until);
}

#[test]
fn interval_event_updates_window_and_emits_noop_decision() {
    let mut runtime = runtime();
    let record = IntervalRecord {
        elapsed_ms: 1_000,
        task: 42,
        samples: 100,
        over_1ms: 3,
        over_2ms: 2,
        over_5ms: 1,
        max_ns: 7_000_000,
        ..IntervalRecord::default()
    };

    let emitted = runtime
        .on_event(MonitorEvent::Interval {
            elapsed_ms: 1_000,
            records: vec![record],
            drop_counters: DropCountersSnapshot::default(),
        })
        .unwrap()
        .unwrap();

    assert_eq!(emitted.decision, "observed");
    assert_eq!(emitted.score_total, 143);
    assert_eq!(runtime.observation().score.total, 143);
    assert_eq!(runtime.observation().scored_task_count, 1);
}

#[test]
fn focus_change_resets_window_and_sets_focus_context() {
    let mut runtime = runtime();

    let emitted = runtime
        .on_event(MonitorEvent::FocusChanged {
            elapsed_ms: 1_000,
            old_kind: None,
            new_kind: FocusGroupKind::Game,
            root_pids: vec![2222],
            member_pids: vec![2222, 2223],
            confidence: 0.90,
            score: 0.95,
            situation: SituationKind::GameFocused,
            reasons: vec!["test focus".to_owned()],
        })
        .unwrap()
        .unwrap();

    assert_eq!(emitted.focus_kind.as_deref(), Some("Game"));
    assert_eq!(emitted.target_root_pid, Some(2222));
    assert_eq!(runtime.observation().focus_kind, Some(FocusGroupKind::Game));
    assert_eq!(
        runtime.observation().primary_situation,
        SituationKind::GameFocused
    );
}

#[test]
fn low_quality_is_reported_in_decision_stream() {
    let mut runtime = runtime();

    let emitted = runtime
        .on_event(MonitorEvent::DataQualityWarning {
            message: "synthetic warning".to_owned(),
        })
        .unwrap()
        .unwrap();

    assert_eq!(emitted.decision, "observed");
    assert!(emitted.data_quality.starts_with("Low"));
    assert_eq!(
        emitted.data_quality_reason_codes,
        vec![
            "insufficient_samples".to_owned(),
            "target_missing".to_owned()
        ]
    );
}

#[test]
fn data_quality_label_names_high_medium_low() {
    assert_eq!(data_quality_label(&OnlineDataQuality::High), "High");
    assert!(
        data_quality_label(&OnlineDataQuality::Medium {
            reasons: vec!["reason".to_owned()]
        })
        .starts_with("Medium")
    );
    assert!(
        data_quality_label(&OnlineDataQuality::Low {
            reasons: vec!["reason".to_owned()]
        })
        .starts_with("Low")
    );
}
