use super::*;
use crate::autotune::research::child_study::{Profile, Stage, StudyResult};

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
        fault_matrix: Err("NoFaultMatrixReceipt"),
        pressure_control: Err("NoPressureControl"),
        child_resident: Ok(()),
        sealed_protected_state: Ok(()),
        protected_state: Ok(()),
        hardware: Ok(()),
        control_socket: None,
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
        fault_matrix: Err("NoFaultMatrixReceipt"),
        pressure_control: Err("NoPressureControl"),
        child_resident: Ok(()),
        sealed_protected_state: Ok(()),
        protected_state: Ok(()),
        hardware: Ok(()),
        control_socket: None,
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

#[test]
fn the_historical_result_does_not_move_with_the_host_gates() {
    let raw = raw(2, 2, 0);
    let studies = [study(Some(StudyResult::Qualified))];
    let verdict = judge(&all_true(&raw, &studies, &[]));
    // G0/G1/G6/G8 cannot be attested here, so the overall gate is not met; the historical flag depends
    // only on the sealed-evidence gates and is never true when one of those is unmet.
    assert!(!verdict.final_gate_qualified);
    let sealed = ["G2", "G3", "G4", "G5", "G7", "G9"];
    let expected = verdict
        .gates
        .iter()
        .filter(|g| sealed.contains(&g.gate))
        .all(|g| g.accepted);
    assert_eq!(verdict.historical_evidence_qualified, expected);
}

#[test]
fn a_justified_abstention_is_a_terminal_study_result_and_g5_then_needs_only_the_control() {
    let raw = raw(2, 2, 0);
    let noop = Disposition {
        stage: Stage::Action(crate::autotune::research::ResearchAction::CpuAffinityProfile),
        result: Some(StudyResult::Unqualified),
        reason: "ActionNotIndicated".into(),
        ..study(None)
    };
    let studies = [study(Some(StudyResult::Qualified)), noop];
    // Resolved as a study result, in the same sense the gates use.
    let with = |control: Result<(), &'static str>| {
        let mut inputs = all_true(&raw, &studies, &[]);
        inputs.pressure_control = control;
        judge(&inputs)
    };
    let verdict = with(Ok(()));
    assert!(verdict.child_studies_qualified);
    let g5 = |v: &Verdict| v.gates.iter().find(|g| g.gate == "G5").unwrap().clone();
    assert!(g5(&verdict).blockers.is_empty());
    assert_eq!(g5(&verdict).notes, ["ActionAbstainedNoPressure"]);
    // Without a passing control the same abstention proves nothing, and the gate says why.
    let without = with(Err("NoPressureControl"));
    assert!(without.child_studies_qualified);
    assert_eq!(g5(&without).blockers, ["NoPressureControl"]);
    assert!(!without.final_gate_qualified);
}

fn plan_of(result: StudyResult) -> Vec<Disposition> {
    use crate::autotune::research::{ResearchAction, ResearchFactor};
    let one = |profile: Profile, stage: Stage| Disposition {
        profile,
        stage,
        result: Some(result),
        ..study(None)
    };
    let mut studies = Vec::new();
    for profile in [Profile::Cpu, Profile::Gpu] {
        for stage in [Stage::Screen, Stage::Pilot, Stage::Confirm, Stage::Holdout] {
            studies.push(one(profile, stage));
        }
    }
    studies.push(one(
        Profile::Long,
        Stage::Action(ResearchAction::CpuAffinityProfile),
    ));
    studies.push(one(Profile::Cpu, Stage::Feature(ResearchFactor::NtSync)));
    studies
}

#[test]
fn the_historical_result_survives_a_later_incident_changed_saves_or_an_expired_control() {
    let raw = raw(2, 2, 0);
    let studies = plan_of(StudyResult::Qualified);
    let base = |edit: &dyn Fn(&mut VerdictInputs<'_>)| {
        let mut inputs = all_true(&raw, &studies, &[]);
        inputs.funnel = true;
        inputs.child_runtime_qualified = true;
        inputs.child_replication = Some(true);
        inputs.pressure_control = Ok(());
        edit(&mut inputs);
        judge(&inputs)
    };
    let clean = base(&|_| {});
    assert!(clean.historical_evidence_qualified, "{:?}", clean.gates);
    assert_eq!(clean.qualification_scope, "QualifiedEndToEnd");
    // The host later changes: a GPU incident, different saves, an old control receipt. The campaign's own
    // sealed result is the same; the current-host verdict is not.
    let later = base(&|inputs| {
        inputs.health_blocked = true;
        inputs.protected_state = Err("ProtectedStateChanged");
        inputs.pressure_control = Err("PressureControlNotFresh");
    });
    assert!(later.historical_evidence_qualified);
    assert!(!later.final_gate_qualified);
    // Evidence that is not sealed in the campaign does move it.
    let unqualified = {
        let mut studies = plan_of(StudyResult::Qualified);
        studies[0].result = Some(StudyResult::Unqualified);
        let mut inputs = all_true(&raw, &studies, &[]);
        inputs.funnel = true;
        inputs.child_runtime_qualified = true;
        judge(&inputs)
    };
    assert!(!unqualified.historical_evidence_qualified);
}

#[test]
fn the_historical_result_needs_the_protected_state_sealed_at_finalization() {
    let raw = raw(2, 2, 0);
    let studies = plan_of(StudyResult::Qualified);
    let judged = |sealed: Result<(), &'static str>| {
        let mut inputs = all_true(&raw, &studies, &[]);
        inputs.funnel = true;
        inputs.child_runtime_qualified = true;
        inputs.child_replication = Some(true);
        inputs.pressure_control = Ok(());
        inputs.sealed_protected_state = sealed;
        judge(&inputs)
    };
    let sealed = judged(Ok(()));
    assert!(sealed.historical_evidence_qualified && sealed.historical_notes.is_empty());
    let unsealed = judged(Err("ProtectedStateNotSealedAtFinalization"));
    assert!(!unsealed.historical_evidence_qualified);
    assert_eq!(
        unsealed.historical_notes,
        ["ProtectedStateNotSealedAtFinalization"]
    );
}
