use super::helpers::*;
use crate::{
    actions::SafetyClass,
    affinity::CpuMask,
    autotune::{
        controller::*,
        decision::{AutotuneDecision, CandidateAction, ExperimentId},
        quality::OnlineDataQuality,
        state::{AutotuneMode, SituationKind},
    },
    focus::FocusGroupKind,
    profiles::{Profile, ProfileRule},
};

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
