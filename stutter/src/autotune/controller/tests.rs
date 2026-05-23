use super::*;
use crate::{
    actions::{ActionId, SafetyClass},
    affinity::CpuMask,
    autotune::{
        decision::{AutotuneDecision, CandidateAction, ExperimentId},
        observation::AutotuneObservation,
        quality::OnlineDataQuality,
        state::{AutotuneMode, ControllerPhase, SituationKind},
    },
    focus::FocusGroupKind,
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
    scorer::StutterScore,
};

fn gaming_cpu_affinity_candidate() -> CandidateAction {
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

fn candidate_with_safety_class(safety_class: SafetyClass) -> CandidateAction {
    CandidateAction::fake(ActionId::new("test".to_owned()), safety_class)
}

fn game_focus_cpu_affinity_candidate_with_name(profile_name: &str) -> CandidateAction {
    CandidateAction::cpu_affinity_profile(
        Profile {
            name: profile_name.to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: Vec::new(),
                match_comm: Vec::new(),
            }],
        },
        1234,
    )
}

fn high_quality_observation_with_score(
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

fn high_quality_observation(diagnostic_score_total: u64) -> AutotuneObservation {
    high_quality_observation_with_score(diagnostic_score_total, 100, 5)
}

fn active_state_with_baseline_score(
    diagnostic_baseline_raw_score_total: u64,
) -> ControllerRuntimeState {
    active_state_with_baseline_window(diagnostic_baseline_raw_score_total, 100, 5)
}

fn active_state_with_baseline_window(
    diagnostic_baseline_raw_score_total: u64,
    baseline_scored_samples: u64,
    baseline_interval_count: usize,
) -> ControllerRuntimeState {
    ControllerRuntimeState {
        phase: ControllerPhase::Measuring,
        active_experiment: Some(ActiveExperiment {
            experiment_id: ExperimentId("experiment-1".to_owned()),
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
fn active_experiment_does_not_revert_when_candidate_raw_score_is_higher_but_rate_is_equal() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_window(1_000, 100, 5); // 10.0 / sample
    let observation = high_quality_observation_with_score(3_000, 300, 15); // 10.0 / sample

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(
                reason.contains("active experiment is still inconclusive"),
                "expected Noop due to inconclusive normalized rates, got: {reason}"
            );
        }
        other => panic!("expected Noop, got {other:?}"),
    }
}

#[test]
fn active_experiment_reverts_when_candidate_raw_score_is_lower_but_rate_is_worse() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_window(1_000, 1_000, 50); // 1.0 / sample
    let observation = high_quality_observation_with_score(900, 100, 5); // 9.0 / sample

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(
                reason.contains("regressed normalized score"),
                "expected regression, got: {reason}"
            );
        }
        other => {
            panic!("expected Revert for worse rate despite lower raw score, got {other:?}")
        }
    }
}

#[test]
fn active_experiment_keeps_when_candidate_raw_score_is_higher_but_rate_is_better() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_window(1_000, 100, 5); // 10.0 / sample
    let observation = high_quality_observation_with_score(1_500, 300, 15); // 5.0 / sample

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::KeepCurrent {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(
                reason.contains("improved normalized score"),
                "expected improvement, got: {reason}"
            );
        }
        other => {
            panic!("expected KeepCurrent for better rate despite higher raw score, got {other:?}")
        }
    }
}

#[test]
fn active_experiment_reverts_when_score_comparison_is_invalid() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_window(1_000, 100, 5);
    let mut observation = high_quality_observation_with_score(1_000, 0, 0);
    observation.scored_task_count = 0;

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(
                reason.contains("invalid active experiment score comparison"),
                "expected invalid reason, got: {reason}"
            );
        }
        other => panic!("expected Revert for invalid score comparison, got {other:?}"),
    }
}

#[test]
fn controller_policy_derives_permissions_from_daemon_policy() {
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
fn controller_policy_uses_shared_score_comparison_thresholds() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let config = crate::autotune::comparison::DEFAULT_SCORE_COMPARISON_CONFIG;

    assert_eq!(
        policy.min_improvement_percent,
        config.min_improvement_percent
    );
    assert_eq!(policy.max_regression_percent, config.max_regression_percent);
}

#[test]
fn controller_policy_uses_named_focus_confidence_threshold() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);

    assert_eq!(DEFAULT_MIN_FOCUS_CONFIDENCE, 0.70);
    assert_eq!(policy.min_focus_confidence, DEFAULT_MIN_FOCUS_CONFIDENCE);
}

#[test]
fn policy_observe_never_applies() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::Observe);
    let state = ControllerRuntimeState::default();
    let observation = high_quality_observation(100);
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(
                reason.contains("observe mode never applies"),
                "unexpected observe-mode reason: {reason}"
            );
        }
        AutotuneDecision::StartExperiment { .. } => {
            panic!("observe mode must never start an experiment")
        }
        AutotuneDecision::Suggest { .. } => {
            panic!("observe mode must never suggest an action")
        }
        other => panic!("expected observe-mode Noop, got {other:?}"),
    }
}

#[test]
fn policy_suggest_never_applies() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::Suggest);
    let state = ControllerRuntimeState::default();
    let observation = high_quality_observation(100);
    let candidate = candidate_with_safety_class(SafetyClass::HighRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Suggest { reason, .. } => {
            assert!(
                reason.contains("suggest mode reports candidate without applying"),
                "unexpected suggest-mode reason: {reason}"
            );
        }
        AutotuneDecision::StartExperiment { .. } => {
            panic!("suggest mode must never start an experiment")
        }
        other => panic!("expected Suggest, got {other:?}"),
    }
}

#[test]
fn policy_apply_low_risk_blocks_medium_risk() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = ControllerRuntimeState::default();
    let observation = high_quality_observation(100);
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleMediumRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(
                reason.contains("candidate safety class ReversibleMediumRisk exceeds mode maximum ReversibleLowRisk"),
                "unexpected low-risk safety gate reason: {reason}"
            );
        }
        AutotuneDecision::StartExperiment { .. } => {
            panic!("apply-low-risk mode must not start a medium-risk experiment")
        }
        other => {
            panic!("expected Noop for medium-risk candidate in low-risk mode, got {other:?}")
        }
    }
}

#[test]
fn policy_low_data_quality_blocks_action() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = ControllerRuntimeState::default();
    let mut observation = high_quality_observation(100);
    observation.data_quality = OnlineDataQuality::Low {
        reasons: vec!["fewer than min_scored_samples".to_owned()],
    };
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(
                reason.contains("low data quality blocks experiment"),
                "unexpected low-data-quality reason: {reason}"
            );
            assert!(
                reason.contains("fewer than min_scored_samples"),
                "low-data-quality reason did not preserve source reason: {reason}"
            );
        }
        AutotuneDecision::StartExperiment { .. } => {
            panic!("low data quality must not start an experiment")
        }
        other => panic!("expected Noop for low data quality, got {other:?}"),
    }
}

#[test]
fn policy_target_exit_reverts() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let mut observation = high_quality_observation(90);
    observation.target_present = false;
    observation.target_root_pid = None;
    observation.active_target_count = 0;

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(
                reason.contains("target disappeared during active experiment"),
                "unexpected target-exit revert reason: {reason}"
            );
        }
        other => panic!("expected Revert after target exit, got {other:?}"),
    }
}

#[test]
fn policy_candidate_regression_reverts() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let observation = high_quality_observation(108);

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(
                reason.contains("candidate regressed normalized score by 8.0%"),
                "unexpected regression reason: {reason}"
            );
            assert!(
                reason.contains("exceeds max_regression_percent 7.5%"),
                "regression reason did not include threshold: {reason}"
            );
        }
        other => panic!("expected Revert for candidate regression, got {other:?}"),
    }
}

#[test]
fn policy_candidate_improvement_keeps() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let observation = high_quality_observation(87);

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::KeepCurrent {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(
                reason.contains("candidate improved normalized score by 13.0%"),
                "unexpected improvement reason: {reason}"
            );
            assert!(
                reason.contains("meets min_improvement_percent 12.5%"),
                "improvement reason did not include threshold: {reason}"
            );
        }
        other => {
            panic!("expected KeepCurrent for sufficient candidate improvement, got {other:?}")
        }
    }
}

#[test]
fn policy_cooldown_prevents_thrashing() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let mut observation = high_quality_observation(100);
    observation.now_unix_nanos = 1_000_000_000;
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);
    let mut state = ControllerRuntimeState::default();

    state.mark_candidate_action_attempted(&candidate, observation.now_unix_nanos);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::EnterCooldown { duration, reason } => {
            assert_eq!(duration.as_secs(), 300);
            assert!(
                reason.contains("same action 'test' is still cooling down"),
                "unexpected same-action cooldown reason: {reason}"
            );
            assert!(
                reason.contains("minimum_time_between_same_action is 300s"),
                "cooldown reason did not include anti-thrash minimum: {reason}"
            );
        }
        AutotuneDecision::StartExperiment { .. } => {
            panic!("cooldown must prevent immediately repeating the same action")
        }
        other => panic!("expected EnterCooldown for repeated action, got {other:?}"),
    }
}

#[test]
fn observe_mode_never_applies() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::Observe);
    let state = ControllerRuntimeState::default();
    let observation = high_quality_observation(100);
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("observe mode never applies"));
        }
        other => panic!("expected Noop, got {other:?}"),
    }
}

#[test]
fn low_data_quality_blocks_experiment() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = ControllerRuntimeState::default();
    let mut observation = high_quality_observation(100);
    observation.data_quality = OnlineDataQuality::Low {
        reasons: vec!["fewer than min_scored_samples".to_owned()],
    };
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("low data quality blocks experiment"));
            assert!(reason.contains("fewer than min_scored_samples"));
        }
        other => panic!("expected Noop, got {other:?}"),
    }
}

#[test]
fn high_risk_action_blocked_by_low_risk_mode() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = ControllerRuntimeState::default();
    let observation = high_quality_observation(100);
    let candidate = candidate_with_safety_class(SafetyClass::HighRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("exceeds mode maximum"));
            assert!(reason.contains("HighRisk"));
            assert!(reason.contains("ReversibleLowRisk"));
        }
        other => panic!("expected Noop, got {other:?}"),
    }
}

#[test]
fn target_disappeared_reverts_active_experiment() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let mut observation = high_quality_observation(90);
    observation.target_present = false;
    observation.data_quality = OnlineDataQuality::Low {
        reasons: vec!["target disappeared".to_owned()],
    };

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(reason.contains("target disappeared"));
        }
        other => panic!("expected Revert, got {other:?}"),
    }
}

#[test]
fn regression_reverts_candidate() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let observation = high_quality_observation(120);

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(reason.contains("regressed normalized score"));
            assert!(reason.contains("exceeds max_regression_percent"));
        }
        other => panic!("expected Revert, got {other:?}"),
    }
}

#[test]
fn improvement_keeps_candidate() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let observation = high_quality_observation(80);

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::KeepCurrent {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(reason.contains("improved normalized score"));
            assert!(reason.contains("meets min_improvement_percent"));
        }
        other => panic!("expected KeepCurrent, got {other:?}"),
    }
}

#[test]
fn idle_focus_blocks_new_experiment() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = ControllerRuntimeState::default();
    let mut observation = high_quality_observation(100);
    observation.focus_kind = Some(FocusGroupKind::Idle);
    observation.primary_situation = SituationKind::Idle;
    observation.focus_reasons = vec!["idle focus selected".to_owned()];
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("idle or unknown"));
            assert!(reason.contains("observe-only"));
        }
        other => panic!("expected Noop for idle focus, got {other:?}"),
    }
}

#[test]
fn compile_focus_blocks_gaming_cpu_isolation_profile() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = ControllerRuntimeState::default();
    let mut observation = high_quality_observation(100);
    observation.focus_kind = Some(FocusGroupKind::Compile);
    observation.primary_situation = SituationKind::CompileLoad;
    observation.focus_reasons = vec!["compile focus selected".to_owned()];
    let candidate = gaming_cpu_affinity_candidate();

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("Compile focus"));
            assert!(reason.contains("game-focus-only CPU-affinity"));
        }
        other => panic!("expected Noop for compile focus gaming profile block, got {other:?}"),
    }
}

#[test]
fn compile_focus_blocks_cpu_affinity_profile_even_without_game_name_or_classes() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = ControllerRuntimeState::default();
    let mut observation = high_quality_observation(100);
    observation.focus_kind = Some(FocusGroupKind::Compile);
    observation.primary_situation = SituationKind::CompileLoad;
    observation.focus_reasons = vec!["compile focus selected".to_owned()];
    let candidate = game_focus_cpu_affinity_candidate_with_name("competitive-latency");

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("Compile focus"));
            assert!(reason.contains("game-focus-only CPU-affinity"));
        }
        other => {
            panic!("expected Noop for compile focus CPU-affinity profile block, got {other:?}")
        }
    }
}

#[test]
fn browser_focus_blocks_gaming_cpu_isolation_profile() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::Suggest);
    let state = ControllerRuntimeState::default();
    let mut observation = high_quality_observation(100);
    observation.focus_kind = Some(FocusGroupKind::Browser);
    observation.primary_situation = SituationKind::BrowserFocused;
    observation.focus_reasons = vec!["browser focus selected".to_owned()];
    let candidate = gaming_cpu_affinity_candidate();

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("Browser focus"));
            assert!(reason.contains("game-focus-only CPU-affinity"));
        }
        other => panic!("expected Noop for browser focus gaming profile block, got {other:?}"),
    }
}

#[test]
fn browser_focus_blocks_cpu_affinity_profile_even_without_game_name_or_classes() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::Suggest);
    let state = ControllerRuntimeState::default();
    let mut observation = high_quality_observation(100);
    observation.focus_kind = Some(FocusGroupKind::Browser);
    observation.primary_situation = SituationKind::BrowserFocused;
    observation.focus_reasons = vec!["browser focus selected".to_owned()];
    let candidate = game_focus_cpu_affinity_candidate_with_name("fps-boost");

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("Browser focus"));
            assert!(reason.contains("game-focus-only CPU-affinity"));
        }
        other => {
            panic!("expected Noop for browser focus CPU-affinity profile block, got {other:?}")
        }
    }
}

#[test]
fn critical_realtime_focus_warning_blocks_unsafe_candidate() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = ControllerRuntimeState::default();
    let mut observation = high_quality_observation(100);
    observation.focus_kind = Some(FocusGroupKind::Game);
    observation.focus_reasons = vec![
        "safety: critical realtime/input process present pid=55 comm='pipewire'; never lower or deprioritize this task".to_owned(),
    ];
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("critical realtime/input"));
            assert!(reason.contains("blocked"));
        }
        other => panic!("expected Noop for critical realtime warning, got {other:?}"),
    }
}

#[test]
fn game_focus_allows_existing_gaming_profile_path() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::Suggest);
    let state = ControllerRuntimeState::default();
    let observation = high_quality_observation(100);
    let candidate = gaming_cpu_affinity_candidate();

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::Suggest { reason, .. } => {
            assert!(reason.contains("suggest mode reports candidate"));
        }
        other => panic!("expected Suggest for game focus, got {other:?}"),
    }
}

#[test]
fn focus_policy_reverts_active_experiment_when_focus_becomes_idle() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let state = active_state_with_baseline_score(100);
    let mut observation = high_quality_observation(90);
    observation.focus_kind = Some(FocusGroupKind::Idle);
    observation.primary_situation = SituationKind::Idle;
    observation.focus_reasons = vec!["idle focus selected".to_owned()];

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id, ExperimentId("experiment-1".to_owned()));
            assert!(reason.contains("focus policy blocks active experiment"));
            assert!(reason.contains("idle or unknown"));
        }
        other => panic!("expected Revert for active experiment with idle focus, got {other:?}"),
    }
}

#[test]
fn cooldown_blocks_repeated_action() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let mut observation = high_quality_observation(100);
    observation.now_unix_nanos = 1_000_000_000;
    let state = ControllerRuntimeState {
        phase: ControllerPhase::Cooldown,
        active_experiment: None,
        cooldown_until_unix_nanos: Some(11_000_000_000),
        candidate_memory: CandidateMemory::default(),
    };
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::EnterCooldown { duration, reason } => {
            assert_eq!(duration.as_secs(), 10);
            assert!(reason.contains("cooldown blocks repeated action"));
        }
        other => panic!("expected EnterCooldown, got {other:?}"),
    }
}

#[test]
fn policy_defaults_use_required_cooldown_values() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);

    assert_eq!(policy.cooldown_after_keep, Duration::from_secs(60));
    assert_eq!(policy.cooldown_after_revert, Duration::from_secs(120));
    assert_eq!(policy.cooldown_after_fault, Duration::from_secs(300));
    assert_eq!(
        policy.minimum_time_between_same_action,
        Duration::from_secs(300)
    );
}

#[test]
fn same_action_hysteresis_blocks_repeated_candidate() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let observation = high_quality_observation(100);
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);
    let mut state = ControllerRuntimeState::default();

    state.mark_candidate_action_attempted(&candidate, observation.now_unix_nanos);

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::EnterCooldown { duration, reason } => {
            assert_eq!(duration.as_secs(), 300);
            assert!(reason.contains("same action 'test' is still cooling down"));
            assert!(reason.contains("minimum_time_between_same_action is 300s"));
        }
        other => panic!("expected same-action EnterCooldown, got {other:?}"),
    }
}

#[test]
fn same_action_hysteresis_allows_candidate_after_minimum_elapsed() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let mut observation = high_quality_observation(100);
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);
    let mut state = ControllerRuntimeState::default();

    state.mark_candidate_action_attempted(&candidate, 1_000_000_000);
    observation.now_unix_nanos = 301_000_000_000;

    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::StartExperiment { reason, .. } => {
            assert!(reason.contains("candidate passed data-quality and safety gates"));
        }
        other => panic!("expected StartExperiment after same-action cooldown, got {other:?}"),
    }
}

#[test]
fn runtime_state_records_keep_revert_and_fault_cooldowns() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let now = 1_000_000_000;
    let mut state = ControllerRuntimeState::default();

    state.enter_cooldown_after_keep(&policy, now);
    assert_eq!(state.phase, ControllerPhase::Cooldown);
    assert_eq!(state.cooldown_until_unix_nanos, Some(61_000_000_000));

    state.enter_cooldown_after_revert(&policy, now);
    assert_eq!(state.phase, ControllerPhase::Cooldown);
    assert_eq!(state.cooldown_until_unix_nanos, Some(121_000_000_000));

    state.enter_cooldown_after_fault(&policy, now);
    assert_eq!(state.phase, ControllerPhase::Faulted);
    assert_eq!(state.cooldown_until_unix_nanos, Some(301_000_000_000));
}

#[test]
fn faulted_controller_enters_fault_cooldown_then_reports_fault() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let mut observation = high_quality_observation(100);
    let mut state = ControllerRuntimeState::default();

    state.enter_cooldown_after_fault(&policy, 1_000_000_000);
    observation.now_unix_nanos = 2_000_000_000;

    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::EnterCooldown { duration, reason } => {
            assert_eq!(duration.as_secs(), 299);
            assert!(reason.contains("fault cooldown blocks repeated action"));
        }
        other => panic!("expected EnterCooldown during fault cooldown, got {other:?}"),
    }

    observation.now_unix_nanos = 301_000_000_000;
    let decision = decide_autotune_transition(&policy, &state, &observation, None);

    match decision {
        AutotuneDecision::Fault { reason } => {
            assert!(reason.contains("controller is faulted"));
        }
        other => panic!("expected Fault after fault cooldown expires, got {other:?}"),
    }
}

#[test]
fn controller_candidate_memory_records_result_fields() {
    let mut state = ControllerRuntimeState::default();
    let mut observation = high_quality_observation(1_125);
    observation.now_unix_nanos = 9_000;
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);

    let record = state.record_candidate_result(ControllerCandidateResultInput {
        candidate: &candidate,
        observation: &observation,
        cpu_topology_signature: Some("cpu0-7:smt:on"),
        result: CandidateMemoryResult::Reverted,
        diagnostic_baseline_raw_score_total: Some(1_000),
        diagnostic_current_raw_score_total: Some(1_125),
        rollback_reason: Some("candidate regressed normalized score".to_owned()),
        cooldown_expires_unix_nanos: Some(309_000_000_000),
    });

    assert_eq!(record.candidate_name, "fake-profile");
    assert_eq!(record.result, CandidateMemoryResult::Reverted);
    assert_eq!(record.score_delta, 125);
    assert_eq!(
        record.rollback_reason.as_deref(),
        Some("candidate regressed normalized score")
    );
    assert_eq!(record.cooldown_expires_unix_nanos, Some(309_000_000_000));
    assert_eq!(state.candidate_memory.latest(), Some(&record));
}

#[test]
fn explicit_candidate_memory_cooldown_blocks_same_action_after_minimum_elapsed() {
    let policy = ControllerPolicy::for_mode(AutotuneMode::ApplyLowRisk);
    let mut state = ControllerRuntimeState::default();
    let candidate = candidate_with_safety_class(SafetyClass::ReversibleLowRisk);
    let mut observation = high_quality_observation(100);
    observation.now_unix_nanos = 1_000_000_000;

    state.record_candidate_result(ControllerCandidateResultInput {
        candidate: &candidate,
        observation: &observation,
        cpu_topology_signature: Some("cpu0-7:smt:on"),
        result: CandidateMemoryResult::Reverted,
        diagnostic_baseline_raw_score_total: Some(1_000),
        diagnostic_current_raw_score_total: Some(1_200),
        rollback_reason: Some("candidate regressed normalized score".to_owned()),
        cooldown_expires_unix_nanos: Some(401_000_000_000),
    });

    observation.now_unix_nanos = 350_000_000_000;
    let decision = decide_autotune_transition(&policy, &state, &observation, Some(candidate));

    match decision {
        AutotuneDecision::EnterCooldown { duration, reason } => {
            assert_eq!(duration.as_secs(), 51);
            assert!(reason.contains("same action 'test' is still cooling down"));
        }
        other => {
            panic!("expected same-action EnterCooldown from candidate memory, got {other:?}")
        }
    }
}
