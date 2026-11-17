use super::*;
use crate::autotune::research::{
    ResearchAction, ResearchFactor, ResearchIntentV1, ResearchTool, child_study::plan,
    tests::intent,
};

fn unit() -> LongWorkUnit {
    LongWorkUnit {
        id: "save-12-route-a".into(),
        duration_seconds: 25 * 60,
        definition_sha256: "ab".repeat(32),
    }
}

fn studies_parent() -> ResearchIntentV1 {
    let mut campaign = intent();
    campaign.tools.push(ResearchTool {
        source: crate::autotune::research::ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    campaign.actions = vec![ResearchAction::CpuAffinityProfile];
    campaign.factors = vec![ResearchFactor::NtSync];
    campaign
}

fn studies() -> Vec<ChildStudy> {
    plan(&studies_parent()).unwrap()
}

#[test]
fn without_a_work_unit_only_the_long_profile_studies_are_blocked_and_the_reason_is_typed() {
    let all = studies();
    assert!(all.iter().any(|study| study.profile == Profile::Long));
    for study in &all {
        let blocked = long_unit_blocker(study, None, Some(3600));
        if study.profile == Profile::Long {
            assert_eq!(blocked, Some("BlockedNoComparableLongWorkUnit"));
        } else {
            assert_eq!(
                blocked, None,
                "{:?} is not a long-session study",
                study.stage
            );
        }
    }
}

#[test]
fn a_valid_work_unit_unblocks_the_long_profile_and_an_invalid_one_does_not() {
    let long = studies()
        .into_iter()
        .find(|study| study.profile == Profile::Long)
        .unwrap();
    assert_eq!(long_unit_blocker(&long, Some(&unit()), Some(3600)), None);
    let mut short = unit();
    short.duration_seconds = 5 * 60;
    let mut endless = unit();
    endless.duration_seconds = 3 * 60 * 60;
    let mut nameless = unit();
    nameless.id = "  ".into();
    let mut undefined = unit();
    undefined.definition_sha256 = "not-a-digest".into();
    for bad in [short, endless, nameless, undefined] {
        assert_eq!(
            long_unit_blocker(&long, Some(&bad), Some(3600)),
            Some("BlockedInvalidLongWorkUnit"),
            "{bad:?}"
        );
    }
}

#[test]
fn a_work_unit_must_fit_the_separately_authorized_long_window() {
    let long = studies()
        .into_iter()
        .find(|study| study.profile == Profile::Long)
        .unwrap();
    // No window authorized: the short benchmark limit cannot hold a long session.
    assert_eq!(
        long_unit_blocker(&long, Some(&unit()), None),
        Some("BlockedNoLongAttemptAuthorization")
    );
    // A window shorter than the session would terminate it before it ends.
    assert_eq!(
        long_unit_blocker(&long, Some(&unit()), Some(20 * 60)),
        Some("BlockedLongWorkUnitExceedsAuthorizedWindow")
    );
    assert_eq!(long_unit_blocker(&long, Some(&unit()), Some(25 * 60)), None);
}

#[test]
fn a_long_child_runs_under_the_long_window_and_every_other_child_under_the_short_limit() {
    use crate::autotune::research::child_intent::attempt_seconds;
    let mut campaign = studies_parent();
    campaign.long_attempt_seconds = Some(30 * 60);
    campaign.validate().unwrap();
    assert_eq!(
        campaign.max_attempt_seconds, 720,
        "the short limit is unchanged"
    );
    for study in plan(&campaign).unwrap() {
        let expected = if study.profile == Profile::Long {
            30 * 60
        } else {
            720
        };
        assert_eq!(attempt_seconds(&campaign, &study), expected);
    }
    // Without the authorization a long study keeps the short limit (and is blocked before it runs).
    campaign.long_attempt_seconds = None;
    let long = plan(&campaign)
        .unwrap()
        .into_iter()
        .find(|study| study.profile == Profile::Long)
        .unwrap();
    assert_eq!(attempt_seconds(&campaign, &long), 720);
}

#[test]
fn only_a_bounded_long_window_that_fits_the_active_budget_is_authorized() {
    let mut campaign = studies_parent();
    for bad in [1, 719, 20 * 60 - 1, 60 * 60 + 1] {
        campaign.long_attempt_seconds = Some(bad);
        assert!(campaign.validate().is_err(), "{bad}");
    }
    // Longer than the whole active budget.
    campaign.max_active_seconds = 25 * 60;
    campaign.long_attempt_seconds = Some(30 * 60);
    assert!(campaign.validate().is_err());
    // A whole-campaign intent never carries a long attempt limit as its own.
    let mut whole = studies_parent();
    whole.max_attempt_seconds = 30 * 60;
    whole.long_attempt_seconds = Some(30 * 60);
    assert!(whole.validate().is_err());
}

#[test]
fn a_long_child_intent_carries_the_long_window_validates_and_is_derived_from_its_campaign() {
    use crate::autotune::research::{
        child_intent::child_intent,
        child_study::{StudyArms, plan},
        child_verify::derived_from,
    };
    let mut campaign = studies_parent();
    campaign.long_attempt_seconds = Some(30 * 60);
    campaign.validate().unwrap();
    let mut long_children = 0;
    for study in plan(&campaign).unwrap() {
        // A confirmation needs the rule its pilot froze; its own stage tests cover it.
        if matches!(
            study.stage,
            crate::autotune::research::child_study::Stage::Confirm
                | crate::autotune::research::child_study::Stage::Holdout
        ) {
            continue;
        }
        let sources: Vec<_> = campaign.tools.iter().map(|tool| tool.source).collect();
        let arms = match study.arms() {
            StudyArms::All => sources,
            StudyArms::Pair => sources[..2].to_vec(),
            StudyArms::Single => sources[..1].to_vec(),
        };
        let Ok(child) = child_intent(&campaign, &study, &arms, None) else {
            continue;
        };
        let long = study.profile == Profile::Long;
        assert_eq!(
            child.max_attempt_seconds,
            if long { 30 * 60 } else { 720 },
            "{:?}",
            study.stage
        );
        assert!(derived_from(&child, &campaign, &study.study_sha256[..16]));
        if long {
            long_children += 1;
        }
        if long {
            // The long window belongs to the child only: the campaign itself keeps the short limit.
            assert_eq!(campaign.max_attempt_seconds, 720);
            // A long child that claims a window the campaign never authorized is not derived from it.
            let mut forged = child.clone();
            forged.max_attempt_seconds = 3600;
            forged.long_attempt_seconds = Some(3600);
            assert!(!derived_from(&forged, &campaign, &study.study_sha256[..16]));
        }
    }
    assert!(long_children > 0, "no long child was exercised");
}
