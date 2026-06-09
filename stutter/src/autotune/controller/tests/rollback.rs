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
