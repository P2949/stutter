use super::support::*;
use crate::autotune::{
    active_config::verify_rollback_restored_baseline, observation::ActiveConfigSnapshot,
    planning::candidate::CandidateAction,
};

#[test]
fn rollback_verification_succeeds_when_active_config_returns_to_baseline() {
    let candidate = nice_candidate_for_rollback();
    let baseline = active_nice_snapshot(42, 0);
    let post_rollback = active_nice_snapshot(42, 0);

    let verification =
        verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

    assert!(verification.verified);
    assert_eq!(verification.reason_code, "rollback_verified");
}

#[test]
fn rollback_verification_faults_when_active_config_still_differs() {
    let candidate = nice_candidate_for_rollback();
    let baseline = active_nice_snapshot(42, 0);
    let post_rollback = active_nice_snapshot(42, 5);

    let verification =
        verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

    assert!(!verification.verified);
    assert_eq!(verification.reason_code, "rollback_state_mismatch");
    assert!(verification.expected.contains("tid=42 nice=0"));
    assert!(verification.actual.contains("tid=42 nice=5"));
}

#[test]
fn rollback_verification_faults_when_target_missing_after_rollback() {
    let candidate = nice_candidate_for_rollback();
    let baseline = active_nice_snapshot(42, 0);
    let post_rollback = ActiveConfigSnapshot::default();

    let verification =
        verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

    assert!(!verification.verified);
    assert_eq!(verification.reason_code, "rollback_target_missing");
    assert!(verification.actual.contains("missing"));
}

#[test]
fn rollback_verification_faults_when_verifier_is_unavailable() {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-rollback".to_owned()),
        crate::actions::SafetyClass::ReversibleLowRisk,
    );
    let baseline = ActiveConfigSnapshot::default();
    let post_rollback = ActiveConfigSnapshot::default();

    let verification =
        verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

    assert!(!verification.verified);
    assert_eq!(
        verification.reason_code,
        "rollback_verification_unavailable"
    );
}
