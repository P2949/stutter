//! Legitimate exclusions stand for a qualified study only where PLAN allows: a feature no authorized
//! build supports. Nothing else that was blocked is ever read as acceptable.

use super::*;

fn blocked(stage: Stage, reason: &str) -> Disposition {
    Disposition {
        reason: reason.into(),
        ..study(stage, Some(StudyResult::Blocked))
    }
}

#[test]
fn a_feature_no_build_supports_is_a_finding_and_resolves_g7() {
    let feature = Stage::Feature(ResearchFactor::NtSync);
    let excluded = blocked(feature, "BlockedNoEligibleSource");
    assert!(legitimately_excluded(&excluded) && resolved(&excluded));
    let verdicts = run(std::slice::from_ref(&excluded), true, false);
    assert!(of(&verdicts, "G7").blockers.is_empty());

    // Mixed with a qualified feature it still resolves; with an unqualified one it does not.
    let qualified = study(
        Stage::Feature(ResearchFactor::Fsr4),
        Some(StudyResult::Qualified),
    );
    assert!(
        of(&run(&[excluded.clone(), qualified], true, false), "G7")
            .blockers
            .is_empty()
    );
    let unqualified = study(
        Stage::Feature(ResearchFactor::Fsr4),
        Some(StudyResult::Unqualified),
    );
    assert_eq!(
        of(&run(&[excluded, unqualified], true, false), "G7").blockers,
        ["FeatureNotQualified"]
    );
}

#[test]
fn nothing_else_blocked_is_acceptable() {
    // A different reason, a different stage, or a failure is never an exclusion.
    for stage in [
        Stage::Screen,
        Stage::Pilot,
        Stage::Confirm,
        Stage::Holdout,
        Stage::Action(ResearchAction::CpuAffinityProfile),
    ] {
        assert!(
            !resolved(&blocked(stage, "BlockedNoEligibleSource")),
            "{stage:?}"
        );
    }
    let feature = Stage::Feature(ResearchFactor::NtSync);
    assert!(!resolved(&blocked(feature, "BlockedGpuStability")));
    assert!(!resolved(&Disposition {
        reason: "BlockedNoEligibleSource".into(),
        ..study(feature, Some(StudyResult::Failed))
    }));
    assert!(!resolved(&study(feature, None)));
}

#[test]
fn a_feature_that_reached_the_game_but_has_no_function_report_is_excluded_by_name_only() {
    let feature = Stage::Feature(ResearchFactor::Fsr4);
    let unobservable = Disposition {
        reason: "FeatureNotObservable".into(),
        ..study(feature, Some(StudyResult::Unqualified))
    };
    assert!(legitimately_excluded(&unobservable) && resolved(&unobservable));
    // Reported off, unproven activation or missing function proof are never exclusions.
    for reason in [
        "FeatureActivatedButNotFunctional",
        "FeatureFunctionalUnproven",
        "FeatureActivationUnproven",
    ] {
        let study = Disposition {
            reason: reason.into(),
            ..study(feature, Some(StudyResult::Unqualified))
        };
        assert!(!resolved(&study), "{reason}");
    }
    // Never for an action or comparison study.
    let action = Disposition {
        reason: "FeatureNotObservable".into(),
        ..study(
            Stage::Action(ResearchAction::CpuAffinityProfile),
            Some(StudyResult::Unqualified),
        )
    };
    assert!(!resolved(&action));
}

#[test]
fn a_justified_no_op_resolves_g5_only_beside_a_passing_owned_pressure_control() {
    let action = Stage::Action(ResearchAction::CpuAffinityProfile);
    let noop = Disposition {
        reason: "ActionNotIndicated".into(),
        ..study(action, Some(StudyResult::Unqualified))
    };
    assert!(not_indicated(&noop) && resolved(&noop));
    let mut studies = full(Some(StudyResult::Qualified));
    studies.retain(|s| !matches!(s.stage, Stage::Action(_)));
    studies.push(noop.clone());
    let g5 = |control: Result<(), &'static str>| {
        let verdicts = gates(&GateInputs {
            raw_files_verified: true,
            loaded_runtime_qualified: true,
            child_studies: &studies,
            child_reports_verified: true,
            restoration_verified: true,
            health_blocked: false,
            telemetry: None,
            replication: Some(true),
            fault_matrix: Err("NoFaultMatrixReceipt"),
            pressure_control: control,
            protected_state: Ok(()),
            hardware: Ok(()),
            control_socket: None,
            deployment_attested: false,
            lease: None,
            host: None,
        });
        of(&verdicts, "G5").clone()
    };
    let with = g5(Ok(()));
    assert!(with.blockers.is_empty() && with.notes == ["ActionAbstainedNoPressure"]);
    // Without the control the abstention proves nothing about the controller's ability to act.
    assert_eq!(g5(Err("NoPressureControl")).blockers, ["NoPressureControl"]);
    // A refusal for another reason is never a justified no-op.
    let refused = Disposition {
        reason: "ActionEvidenceMissing".into(),
        ..noop
    };
    assert!(!not_indicated(&refused));
}
