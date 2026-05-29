use crate::{
    actions::{ActionId, SafetyClass},
    affinity::CpuMask,
    autotune::{
        controller::*,
        decision::{CandidateAction, ExperimentId},
        observation::AutotuneObservation,
        quality::OnlineDataQuality,
        state::{AutotuneMode, ControllerPhase, SituationKind},
    },
    focus::FocusGroupKind,
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
    scorer::StutterScore,
};

pub(super) fn gaming_cpu_affinity_candidate() -> CandidateAction {
    CandidateAction::cpu_affinity_profile(
        Profile {
            name: "game-main-suggested".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        },
        1234,
    )
}

pub(super) fn candidate_with_safety_class(safety_class: SafetyClass) -> CandidateAction {
    CandidateAction::fake(ActionId::new("test".to_owned()), safety_class)
}

pub(super) fn high_quality_observation_with_score(
    diagnostic_score_total: u64,
    scored_samples: u64,
    interval_count: usize,
) -> AutotuneObservation {
    AutotuneObservation {
        now_unix_nanos: 1_000_000_000,
        elapsed_ms: 30_000,
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 4,
        scored_task_count: 2,
        interval_count,
        scored_samples,
        score: StutterScore {
            total: diagnostic_score_total,
            frame_p99_ms: 12.0,
            frame_max_ms: 20.0,
            ..StutterScore::default()
        },
        data_quality: OnlineDataQuality::High,
        primary_situation: SituationKind::GameCpuSchedulerPressure,
        situation: Default::default(),
        focus_kind: Some(FocusGroupKind::Game),
        focus_confidence: 0.80,
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

pub(super) fn high_quality_observation(diagnostic_score_total: u64) -> AutotuneObservation {
    high_quality_observation_with_score(diagnostic_score_total, 100, 5)
}

pub(super) fn active_state_with_baseline_score(
    diagnostic_baseline_raw_score_total: u64,
) -> ControllerRuntimeState {
    active_state_with_baseline_window(diagnostic_baseline_raw_score_total, 100, 5)
}

pub(super) fn active_state_with_baseline_window(
    diagnostic_baseline_raw_score_total: u64,
    baseline_scored_samples: u64,
    baseline_interval_count: usize,
) -> ControllerRuntimeState {
    ControllerRuntimeState {
        phase: ControllerPhase::Measuring,
        active_experiment: Some(ActiveExperiment {
            experiment_id: ExperimentId::new("experiment-1"),
            candidate: candidate_with_safety_class(SafetyClass::ReversibleLowRisk),
            baseline_score: WindowScore {
                started_unix_nanos: 0,
                finished_unix_nanos: 30_000_000_000,
                interval_count: baseline_interval_count,
                scored_samples: baseline_scored_samples,
                scored_task_count: 2,
                score: crate::scorer::StutterScore {
                    total: diagnostic_baseline_raw_score_total,
                    frame_p99_ms: 12.0,
                    frame_max_ms: 20.0,
                    ..Default::default()
                },
            },
        }),
        cooldown_until_unix_nanos: None,
        candidate_memory: CandidateMemory::default(),
    }
}

#[test]
pub(super) fn controller_policy_derives_permissions_from_daemon_policy() {
    let mut config = DaemonConfig {
        mode: DaemonMode::ApplyLowRisk,
        source: ActionSource::AutotuneRuntime,
        ..DaemonConfig::default()
    };
    config.safety.min_confidence = 0.82;
    let daemon_policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    });

    let policy = ControllerPolicy::from_daemon_policy(&daemon_policy);

    assert_eq!(policy.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(policy.max_safety_class, SafetyClass::ReversibleLowRisk);
    assert_eq!(policy.min_focus_confidence, 0.82);
    assert!(policy.can_start_experiment());
}

#[test]
pub(super) fn controller_policy_uses_shared_score_comparison_thresholds() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let config = crate::autotune::comparison::DEFAULT_SCORE_COMPARISON_CONFIG;

    assert_eq!(
        policy.min_improvement_percent,
        config.min_improvement_percent
    );
    assert_eq!(policy.max_regression_percent, config.max_regression_percent);
}

#[test]
pub(super) fn controller_policy_uses_named_focus_confidence_threshold() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);

    assert_eq!(DEFAULT_MIN_FOCUS_CONFIDENCE, 0.70);
    assert_eq!(policy.min_focus_confidence, DEFAULT_MIN_FOCUS_CONFIDENCE);
}
