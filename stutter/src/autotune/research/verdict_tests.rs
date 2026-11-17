use super::*;
use crate::autotune::research::child_study::{Profile, Stage};

fn raw(completed: u32, verified: u32, problems: usize) -> RawVerification {
    RawVerification {
        completed,
        verified,
        failed_attempts: 0,
        problems: (0..problems)
            .map(
                |slot_index| crate::autotune::research::raw_verify::RawProblem {
                    slot_index: slot_index as u32,
                    reason: "x".into(),
                },
            )
            .collect(),
    }
}

fn study(result: Option<StudyResult>) -> Disposition {
    Disposition {
        study_sha256: "a".repeat(64),
        profile: Profile::Cpu,
        stage: Stage::Screen,
        result,
        reason: String::new(),
    }
}

fn all_true<'a>(
    raw: &'a RawVerification,
    studies: &'a [Disposition],
    evidence: &'a [AttemptEvidenceV1],
) -> VerdictInputs<'a> {
    VerdictInputs {
        reconstruction_passed: true,
        raw: Some(raw),
        evidence,
        child_studies: studies,
        child_reports_verified: true,
        funnel: false,
        child_runtime_qualified: false,
        child_telemetry: None,
        child_replication: None,
        deployment_attested: false,
        lease: None,
        host: None,
        restoration_verified: true,
        health_blocked: false,
    }
}

#[test]
fn each_status_stands_on_its_own_evidence_and_the_final_gate_needs_all() {
    let good = raw(2, 2, 0);
    let studies = [study(Some(StudyResult::Qualified))];
    // No evidence with a matched runtime: the runtime status (and so the gate) is false.
    let verdict = judge(&all_true(&good, &studies, &[]));
    assert!(verdict.reconstruction_passed && verdict.raw_files_verified);
    assert!(!verdict.loaded_runtime_qualified);
    assert!(!verdict.final_gate_qualified);
    assert_eq!(verdict.disposition, "PartiallyQualified");
}

#[test]
fn a_blocked_study_or_unproven_restoration_or_failed_replay_denies_the_gate() {
    let good = raw(2, 2, 0);
    let bad_raw = raw(2, 1, 1);
    let qualified = [study(Some(StudyResult::Qualified))];
    let blocked = [study(Some(StudyResult::Blocked))];
    let undecided = [study(None)];
    let unrestored = VerdictInputs {
        restoration_verified: false,
        ..all_true(&good, &qualified, &[])
    };
    assert!(!judge(&unrestored).restoration_verified);
    assert!(!judge(&unrestored).final_gate_qualified);
    assert!(!judge(&all_true(&good, &blocked, &[])).child_studies_qualified);
    assert!(!judge(&all_true(&good, &undecided, &[])).child_studies_qualified);
    assert!(!judge(&all_true(&good, &[], &[])).child_studies_qualified);
    let failed = judge(&all_true(&bad_raw, &qualified, &[]));
    assert!(!failed.raw_files_verified);
    assert_eq!(failed.disposition, "Unverified");
    let unreconstructed = VerdictInputs {
        reconstruction_passed: false,
        ..all_true(&good, &qualified, &[])
    };
    assert_eq!(judge(&unreconstructed).disposition, "Unverified");
    let no_replay = VerdictInputs {
        raw: None,
        ..all_true(&good, &qualified, &[])
    };
    assert!(!judge(&no_replay).raw_files_verified);
}

#[test]
fn a_health_incident_is_aborted_health_whatever_else_holds() {
    let good = raw(2, 2, 0);
    let qualified = [study(Some(StudyResult::Qualified))];
    let verdict = judge(&VerdictInputs {
        health_blocked: true,
        ..all_true(&good, &qualified, &[])
    });
    assert_eq!(verdict.disposition, "AbortedHealth");
    assert!(!verdict.final_gate_qualified);
}

#[test]
fn unverified_child_reports_deny_the_final_gate_even_when_every_study_looks_qualified() {
    let good = raw(2, 2, 0);
    let qualified = [study(Some(StudyResult::Qualified))];
    let verdict = judge(&VerdictInputs {
        child_reports_verified: false,
        ..all_true(&good, &qualified, &[])
    });
    assert!(!verdict.child_reports_verified);
    assert!(!verdict.final_gate_qualified);
}

#[test]
fn a_funnel_campaign_is_judged_from_its_children_not_its_empty_flat_schedule() {
    let studies = [study(Some(StudyResult::Qualified))];
    let funnel = |verified: bool, runtime: bool| VerdictInputs {
        reconstruction_passed: true,
        // The flat parent replay is empty in funnel mode and must not decide anything.
        raw: None,
        evidence: &[],
        child_studies: &studies,
        child_reports_verified: verified,
        funnel: true,
        child_runtime_qualified: runtime,
        child_telemetry: None,
        child_replication: None,
        deployment_attested: false,
        lease: None,
        host: None,
        restoration_verified: true,
        health_blocked: false,
    };
    let verdict = judge(&funnel(true, true));
    assert!(verdict.raw_files_verified && verdict.loaded_runtime_qualified);
    // Clean studies are not enough: the gates a campaign cannot attest (host baseline, lease receipts,
    // telemetry, fault matrix, deployment) keep the overall result from being `Qualified`.
    assert!(!verdict.final_gate_qualified);
    assert_eq!(verdict.disposition, "PartiallyQualified");
    assert!(verdict.gates.iter().any(|gate| !gate.accepted));
    // Unverified children verify no raw files; an unmatched child runtime qualifies no runtime.
    assert!(!judge(&funnel(false, true)).raw_files_verified);
    let unmatched = judge(&funnel(true, false));
    assert!(!unmatched.loaded_runtime_qualified && !unmatched.final_gate_qualified);
    assert_eq!(unmatched.disposition, "PartiallyQualified");
}
