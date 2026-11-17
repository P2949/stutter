use super::*;
use crate::autotune::research::{
    ResearchAction, ResearchFactor, ResearchTool, child_study::plan, tests::intent,
};

fn unit() -> LongWorkUnit {
    LongWorkUnit {
        id: "save-12-route-a".into(),
        duration_seconds: 25 * 60,
        definition_sha256: "ab".repeat(32),
    }
}

fn studies() -> Vec<ChildStudy> {
    let mut campaign = intent();
    campaign.tools.push(ResearchTool {
        source: crate::autotune::research::ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    campaign.actions = vec![ResearchAction::CpuAffinityProfile];
    campaign.factors = vec![ResearchFactor::NtSync];
    plan(&campaign).unwrap()
}

#[test]
fn without_a_work_unit_only_the_long_profile_studies_are_blocked_and_the_reason_is_typed() {
    let all = studies();
    assert!(all.iter().any(|study| study.profile == Profile::Long));
    for study in &all {
        let blocked = long_unit_blocker(study, None);
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
    assert_eq!(long_unit_blocker(&long, Some(&unit())), None);
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
            long_unit_blocker(&long, Some(&bad)),
            Some("BlockedInvalidLongWorkUnit"),
            "{bad:?}"
        );
    }
}
