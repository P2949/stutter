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
