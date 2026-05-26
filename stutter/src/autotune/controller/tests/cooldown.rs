use super::helpers::*;
use crate::{
    actions::SafetyClass,
    autotune::{
        controller::*,
        decision::AutotuneDecision,
        state::{AutotuneMode, ControllerPhase},
    },
};

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
