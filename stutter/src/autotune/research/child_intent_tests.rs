use super::*;
use crate::autotune::research::{
    ResearchAction, ResearchFactor, ResearchTool,
    child_study::{Stage, plan},
    driver::build_schedule,
    tests::intent,
};

const SIX: [ProtonSource; 6] = [
    ProtonSource::ValveStable,
    ProtonSource::ValveExperimental,
    ProtonSource::CachyOs,
    ProtonSource::CachyOsV3,
    ProtonSource::Ge,
    ProtonSource::Em,
];

/// A campaign over all six required Proton sources, with an action and a factor authorized.
fn parent() -> ResearchIntentV1 {
    let mut intent = intent();
    intent.tools = SIX
        .iter()
        .enumerate()
        .map(|(index, source)| ResearchTool {
            source: *source,
            identity_sha256: format!("{index:x}").repeat(64)[..64].to_owned(),
        })
        .collect();
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    intent.factors = vec![ResearchFactor::NtSync];
    intent.max_attempts = 120;
    intent
}

fn pair() -> [ProtonSource; 2] {
    [ProtonSource::ValveStable, ProtonSource::Ge]
}

#[test]
fn a_whole_campaign_intent_keeps_its_digest_and_serialization() {
    let parent = parent();
    let json = serde_json::to_string(&parent).unwrap();
    assert!(
        !json.contains("child_study"),
        "an absent child binding is not serialized"
    );
    let back: ResearchIntentV1 = serde_json::from_str(&json).unwrap();
    assert_eq!(back, parent);
    assert_eq!(back.digest().unwrap(), parent.digest().unwrap());
}

#[test]
fn every_stage_of_a_six_proton_campaign_builds_a_schedule_over_its_own_arms() {
    let parent = parent();
    for study in plan(&parent).unwrap() {
        let arms: Vec<ProtonSource> = match study.arms() {
            StudyArms::All => SIX.to_vec(),
            StudyArms::Pair => pair().to_vec(),
        };
        let child = child_intent(&parent, &study, &arms)
            .unwrap_or_else(|e| panic!("{:?}/{:?}: {e:#}", study.profile, study.stage));
        let schedule = build_schedule(&child).unwrap();
        assert_eq!(schedule.len(), study.min_attempts as usize);
        // Every arm of the study, and only those, appears equally often (AB/BA balance).
        for arm in &arms {
            let count = schedule.iter().filter(|slot| slot.arm == *arm).count();
            assert_eq!(count, study.min_attempts as usize / arms.len());
        }
        assert!(schedule.iter().all(|slot| arms.contains(&slot.arm)));
    }
    // The stage shapes that used to be impossible with six tools.
    let plan = plan(&parent).unwrap();
    let find = |wanted: &dyn Fn(&Stage) -> bool| plan.iter().find(|s| wanted(&s.stage)).unwrap();
    assert_eq!(find(&|s| matches!(s, Stage::Confirm)).min_attempts, 8);
    assert_eq!(find(&|s| matches!(s, Stage::Action(_))).min_attempts, 2);
    assert_eq!(find(&|s| matches!(s, Stage::Feature(_))).min_attempts, 8);
}

#[test]
fn siblings_with_equal_attempt_counts_still_get_distinct_digests_and_schedules() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let cpu = child_intent(&parent, &plan[0], &SIX).unwrap();
    let gpu = child_intent(&parent, &plan[1], &SIX).unwrap();
    assert_eq!(cpu.max_attempts, gpu.max_attempts);
    assert_ne!(cpu.digest().unwrap(), gpu.digest().unwrap());
    assert_ne!(cpu.digest().unwrap(), parent.digest().unwrap());
    let (a, b) = (build_schedule(&cpu).unwrap(), build_schedule(&gpu).unwrap());
    assert_ne!(a[0].study_sha256, b[0].study_sha256);
    assert_eq!(
        a[0].context_sha256, b[0].context_sha256,
        "the host context is the same"
    );
    assert_eq!(
        a,
        build_schedule(&cpu).unwrap(),
        "a verifier regenerates the same schedule"
    );
}

#[test]
fn only_the_arms_and_the_budget_and_the_binding_differ_from_the_parent() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let confirm = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Confirm))
        .unwrap();
    let child = child_intent(&parent, confirm, &pair()).unwrap();
    let mut expected = parent.clone();
    expected.tools = parent
        .tools
        .iter()
        .filter(|t| pair().contains(&t.source))
        .cloned()
        .collect();
    expected.max_attempts = confirm.min_attempts;
    expected.child_study_sha256 = Some(confirm.study_sha256.clone());
    assert_eq!(child, expected);
    // The arms keep the campaign's own frozen identities, in the campaign's own order.
    assert_eq!(
        child.tools.iter().map(|t| t.source).collect::<Vec<_>>(),
        [ProtonSource::ValveStable, ProtonSource::Ge]
    );
}

#[test]
fn arms_outside_the_authorization_or_of_the_wrong_shape_are_refused() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let screen = &plan[0];
    let confirm = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Confirm))
        .unwrap();
    // A pair for a screen, six arms for a pair stage, one arm, a duplicate.
    assert!(child_intent(&parent, screen, &pair()).is_err());
    assert!(child_intent(&parent, confirm, &SIX).is_err());
    assert!(child_intent(&parent, confirm, &[ProtonSource::Ge]).is_err());
    assert!(
        child_intent(&parent, confirm, &[ProtonSource::Ge, ProtonSource::Ge]).is_err(),
        "a duplicate is one tool, not two"
    );
    // A tool the campaign never authorized cannot be introduced.
    let mut narrow = parent.clone();
    narrow.tools.truncate(4);
    narrow.max_attempts = 120;
    assert!(child_intent(&narrow, confirm, &[ProtonSource::Ge, ProtonSource::Em]).is_err());
}

#[test]
fn a_study_that_does_not_fill_its_blocks_exceeds_the_budget_or_is_nested_is_refused() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let confirm = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Confirm))
        .unwrap();
    let mut odd = confirm.clone();
    odd.min_attempts = 3;
    assert!(
        child_intent(&parent, &odd, &pair()).is_err(),
        "not whole blocks across 2 arms"
    );
    let mut zero = confirm.clone();
    zero.min_attempts = 0;
    assert!(child_intent(&parent, &zero, &pair()).is_err());
    let mut small = parent.clone();
    small.max_attempts = 4;
    assert!(
        child_intent(&small, confirm, &pair()).is_err(),
        "more than the campaign allows"
    );
    let nested = child_intent(&parent, &plan[0], &SIX).unwrap();
    assert!(child_intent(&nested, &plan[1], &SIX).is_err());
    let mut bad = parent.clone();
    bad.child_study_sha256 = Some("xyz".into());
    assert!(
        bad.validate().is_err(),
        "a malformed child digest is invalid"
    );
}

#[test]
fn a_feature_or_action_study_needs_the_campaigns_own_authorization_for_that_factor_or_action() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let feature = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Feature(_)))
        .unwrap();
    let action = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Action(_)))
        .unwrap();
    assert!(child_intent(&parent, feature, &pair()).is_ok());
    assert!(child_intent(&parent, action, &pair()).is_ok());
    // The same studies against a campaign that never authorized the factor or the action.
    let mut no_factor = parent.clone();
    no_factor.factors.clear();
    assert!(child_intent(&no_factor, feature, &pair()).is_err());
    let mut no_action = parent.clone();
    no_action.actions.clear();
    assert!(child_intent(&no_action, action, &pair()).is_err());
    let mut other = parent.clone();
    other.factors = vec![ResearchFactor::Hdr];
    assert!(
        child_intent(&other, feature, &pair()).is_err(),
        "a different factor is not this one"
    );
}
