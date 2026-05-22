use super::{
    super::{candidate::*, dry_run::*},
    support::*,
};
use crate::actions::SafetyClass;

#[test]
fn dry_run_candidate_records_failure_as_ineligible() {
    let candidate = CandidateAction::cpu_affinity_profile(profile("bad-tree"), 0);

    let record = dry_run_candidate(&candidate);

    assert_eq!(record.candidate_name, "bad-tree");
    assert_eq!(record.affected_tasks, 0);
    assert_eq!(record.safety_class, SafetyClass::ReversibleLowRisk);
    assert!(!record.eligible);
    assert!(
        record
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("dry-run failed")
    );
    assert!(
        record
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("tree pid must be greater than zero")
    );
}

#[test]
fn dry_run_candidates_preserves_candidate_order() {
    let candidates = vec![
        CandidateAction::cpu_affinity_profile(profile("first"), 0),
        CandidateAction::cpu_affinity_profile(profile("second"), 0),
    ];

    let records = dry_run_candidates(&candidates);

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].candidate_name, "first");
    assert_eq!(records[1].candidate_name, "second");
}
