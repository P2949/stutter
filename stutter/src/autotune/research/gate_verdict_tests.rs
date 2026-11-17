use super::*;
use crate::autotune::research::{
    ResearchAction, ResearchFactor,
    child_study::{Profile, Stage},
};

fn study(stage: Stage, result: Option<StudyResult>) -> Disposition {
    Disposition {
        study_sha256: "a".repeat(64),
        profile: Profile::Cpu,
        stage,
        result,
        reason: String::new(),
    }
}

fn full(result: Option<StudyResult>) -> Vec<Disposition> {
    vec![
        study(Stage::Screen, result),
        study(Stage::Pilot, result),
        study(Stage::Confirm, result),
        study(Stage::Holdout, result),
        study(Stage::Action(ResearchAction::CpuAffinityProfile), result),
        study(Stage::Feature(ResearchFactor::NtSync), result),
    ]
}

fn run(studies: &[Disposition], restored: bool, health: bool) -> Vec<GateVerdict> {
    gates(&GateInputs {
        raw_files_verified: true,
        loaded_runtime_qualified: true,
        child_studies: studies,
        child_reports_verified: true,
        restoration_verified: restored,
        health_blocked: health,
    })
}

fn of<'a>(verdicts: &'a [GateVerdict], name: &str) -> &'a GateVerdict {
    verdicts
        .iter()
        .find(|verdict| verdict.gate == name)
        .unwrap()
}

#[test]
fn there_is_one_verdict_per_gate_and_the_unattestable_ones_never_pass() {
    let verdicts = run(&full(Some(StudyResult::Qualified)), true, false);
    let names: Vec<_> = verdicts.iter().map(|verdict| verdict.gate).collect();
    assert_eq!(
        names,
        ["G0", "G1", "G2", "G3", "G4", "G5", "G6", "G7", "G8", "G9"]
    );
    // A campaign verification cannot attest a host baseline, the lease receipts, resident
    // telemetry, a fault matrix or a deployment: they are named, not inferred from the studies.
    for name in ["G0", "G1", "G3", "G6", "G8"] {
        let verdict = of(&verdicts, name);
        assert!(!verdict.accepted && verdict.blockers.len() == 1, "{name}");
    }
    for name in ["G2", "G4", "G5", "G7", "G9"] {
        let verdict = of(&verdicts, name);
        assert!(verdict.accepted && verdict.blockers.is_empty(), "{name}");
    }
}

#[test]
fn each_gate_names_what_is_missing() {
    // Nothing decided: every study-backed gate is blocked by its own study kind.
    let verdicts = run(&full(None), false, false);
    assert_eq!(of(&verdicts, "G2").blockers, ["ScreenNotQualified"]);
    assert_eq!(of(&verdicts, "G4").blockers, ["ComparisonNotQualified"]);
    assert_eq!(
        of(&verdicts, "G5").blockers,
        ["ActionNotQualified", "RestorationUnverified"]
    );
    assert_eq!(of(&verdicts, "G7").blockers, ["FeatureNotQualified"]);
    assert!(
        of(&verdicts, "G9")
            .blockers
            .contains(&"HoldoutNotQualified")
    );
    // A plan with no such studies says so, rather than passing vacuously.
    let none = run(&[], true, false);
    assert_eq!(of(&none, "G5").blockers, ["NoActionStudy"]);
    assert_eq!(of(&none, "G7").blockers, ["NoFeatureStudy"]);
    assert_eq!(of(&none, "G9").blockers[0], "NoHoldoutStudy");
}

#[test]
fn qualified_studies_do_not_accept_an_action_without_a_proven_restoration() {
    let verdicts = run(&full(Some(StudyResult::Qualified)), false, false);
    assert_eq!(of(&verdicts, "G5").blockers, ["RestorationUnverified"]);
    assert_eq!(of(&verdicts, "G9").blockers, ["G5NotAccepted"]);
}

#[test]
fn a_health_incident_blocks_every_gate_that_needs_a_game() {
    let verdicts = run(&full(Some(StudyResult::Qualified)), true, true);
    for name in ["G2", "G4", "G5", "G7", "G9"] {
        assert_eq!(
            of(&verdicts, name).blockers,
            ["GpuHealthIncident"],
            "{name}"
        );
    }
}

#[test]
fn a_blocked_or_failed_study_is_not_a_qualified_one() {
    for result in [
        StudyResult::Unqualified,
        StudyResult::Failed,
        StudyResult::Blocked,
        StudyResult::DeferredBudget,
    ] {
        let verdicts = run(&full(Some(result)), true, false);
        assert!(!of(&verdicts, "G4").accepted, "{result:?}");
        assert!(!of(&verdicts, "G9").accepted, "{result:?}");
    }
}
