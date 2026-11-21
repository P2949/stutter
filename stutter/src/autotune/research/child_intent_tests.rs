use super::*;
use crate::autotune::research::pilot_rule::{FrozenConfirmationRule, freeze};

fn rule() -> FrozenConfirmationRule {
    freeze(&[70.0, 73.0, 67.0]).unwrap()
}

/// Build a child intent, giving a confirmation the rule its pilot would have frozen.
fn build(
    parent: &ResearchIntentV1,
    study: &ChildStudy,
    arms: &[ProtonSource],
) -> anyhow::Result<ResearchIntentV1> {
    let rule = matches!(
        study.stage,
        Stage::Confirm | Stage::Holdout | Stage::MatchedCachyOs
    )
    .then(rule);
    child_intent(parent, study, arms, rule.as_ref())
}
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
            StudyArms::All | StudyArms::Viable => SIX.to_vec(),
            StudyArms::Pair if study.stage == Stage::MatchedCachyOs => {
                vec![ProtonSource::CachyOs, ProtonSource::CachyOsV3]
            }
            StudyArms::Pair => pair().to_vec(),
            StudyArms::Single => vec![ProtonSource::ValveStable],
            StudyArms::Listed => parent.prefix_init_arms.clone(),
        };
        let child = build(&parent, &study, &arms)
            .unwrap_or_else(|e| panic!("{:?}/{:?}: {e:#}", study.profile, study.stage));
        let schedule = build_schedule(&child).unwrap();
        let expected = if study.stage == Stage::Pilot {
            arms.len() * 3
        } else {
            study.min_attempts as usize
        };
        assert_eq!(schedule.len(), expected);
        if study.arms() == StudyArms::Single {
            // One build, two treatments: the arm is that build in every slot, the treatment is carried
            // beside it, balanced off and on.
            assert!(schedule.iter().all(|slot| slot.arm == arms[0]));
            let treatments = crate::autotune::research::treatment::treatments_of(&child)
                .unwrap()
                .unwrap();
            assert_eq!(treatments.len(), schedule.len());
            assert_eq!(child.treatment, study.treatment_spec());
            continue;
        }
        // Every arm of the study, and only those, appears equally often (AB/BA balance).
        for arm in &arms {
            let count = schedule.iter().filter(|slot| slot.arm == *arm).count();
            assert_eq!(count, expected / arms.len());
        }
        assert!(schedule.iter().all(|slot| arms.contains(&slot.arm)));
    }
    // The stage shapes that used to be impossible with six tools.
    let plan = plan(&parent).unwrap();
    let find = |wanted: &dyn Fn(&Stage) -> bool| plan.iter().find(|s| wanted(&s.stage)).unwrap();
    assert_eq!(find(&|s| matches!(s, Stage::Confirm)).min_attempts, 8);
    assert_eq!(find(&|s| matches!(s, Stage::Action(_))).min_attempts, 8);
    assert_eq!(find(&|s| matches!(s, Stage::Feature(_))).min_attempts, 8);
}

#[test]
fn siblings_with_equal_attempt_counts_still_get_distinct_digests_and_schedules() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let cpu = build(&parent, &plan[0], &SIX).unwrap();
    let gpu = build(&parent, &plan[1], &SIX).unwrap();
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
    let child = build(&parent, confirm, &pair()).unwrap();
    let mut expected = parent.clone();
    expected.tools = parent
        .tools
        .iter()
        .filter(|t| pair().contains(&t.source))
        .cloned()
        .collect();
    expected.max_attempts = confirm.min_attempts;
    expected.child_study_sha256 = Some(confirm.study_sha256.clone());
    expected.confirmation_rule = Some(rule());
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
    assert!(build(&parent, screen, &pair()).is_err());
    assert!(build(&parent, confirm, &SIX).is_err());
    assert!(build(&parent, confirm, &[ProtonSource::Ge]).is_err());
    assert!(
        build(&parent, confirm, &[ProtonSource::Ge, ProtonSource::Ge]).is_err(),
        "a duplicate is one tool, not two"
    );
    // A tool the campaign never authorized cannot be introduced.
    let mut narrow = parent.clone();
    narrow.tools.truncate(4);
    narrow.max_attempts = 120;
    assert!(build(&narrow, confirm, &[ProtonSource::Ge, ProtonSource::Em]).is_err());
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
        build(&parent, &odd, &pair()).is_err(),
        "not whole blocks across 2 arms"
    );
    let mut zero = confirm.clone();
    zero.min_attempts = 0;
    assert!(build(&parent, &zero, &pair()).is_err());
    let mut small = parent.clone();
    small.max_attempts = 4;
    assert!(
        build(&small, confirm, &pair()).is_err(),
        "more than the campaign allows"
    );
    let nested = build(&parent, &plan[0], &SIX).unwrap();
    assert!(build(&nested, &plan[1], &SIX).is_err());
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
    let one = [ProtonSource::ValveStable];
    assert!(build(&parent, feature, &one).is_ok());
    assert!(build(&parent, action, &one).is_ok());
    // A within-source study is one build, never a pair.
    assert!(build(&parent, feature, &pair()).is_err());
    // The same studies against a campaign that never authorized the factor or the action.
    let mut no_factor = parent.clone();
    no_factor.factors.clear();
    assert!(build(&no_factor, feature, &one).is_err());
    let mut no_action = parent.clone();
    no_action.actions.clear();
    assert!(build(&no_action, action, &one).is_err());
    let mut other = parent.clone();
    other.factors = vec![ResearchFactor::Hdr];
    assert!(
        build(&other, feature, &pair()).is_err(),
        "a different factor is not this one"
    );
}

#[test]
fn only_a_confirmation_carries_a_rule_and_it_must_be_one_a_pilot_could_have_frozen() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let confirm = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Confirm))
        .unwrap();
    let screen = &plan[0];
    let frozen = rule();
    // The rule is bound into the confirmation's frozen intent (and its digest).
    let with_rule = child_intent(&parent, confirm, &pair(), Some(&frozen)).unwrap();
    assert_eq!(with_rule.confirmation_rule.as_ref(), Some(&frozen));
    let other = freeze(&[70.0, 70.1, 69.9]).unwrap();
    let with_other = child_intent(&parent, confirm, &pair(), Some(&other)).unwrap();
    assert_ne!(with_rule.digest().unwrap(), with_other.digest().unwrap());
    // No pilot rule, no confirmation; a rule anywhere else is refused.
    assert!(child_intent(&parent, confirm, &pair(), None).is_err());
    assert!(child_intent(&parent, screen, &SIX, Some(&frozen)).is_err());
    // A hand-loosened rule (even re-digested) is not one a pilot could have frozen.
    let mut loosened = frozen.clone();
    loosened.min_effect_bp *= 4;
    assert!(child_intent(&parent, confirm, &pair(), Some(&loosened)).is_err());
    // And a rule cannot be smuggled onto an intent that is not a child.
    let mut smuggled = parent.clone();
    smuggled.confirmation_rule = Some(frozen);
    assert!(smuggled.validate().is_err());
}

mod holdout {
    use super::*;

    fn find(
        plan: &[ChildStudy],
        profile: crate::autotune::research::child_study::Profile,
        stage: fn(&Stage) -> bool,
    ) -> ChildStudy {
        plan.iter()
            .find(|s| s.profile == profile && stage(&s.stage))
            .unwrap()
            .clone()
    }

    #[test]
    fn a_holdout_is_a_fresh_separately_seeded_repetition_of_its_confirmation_under_the_same_rule() {
        use crate::autotune::research::child_study::Profile;
        let parent = parent();
        let plan = plan(&parent).unwrap();
        let confirm = find(&plan, Profile::Cpu, |s| matches!(s, Stage::Confirm));
        let holdout = find(&plan, Profile::Cpu, |s| matches!(s, Stage::Holdout));
        let pilot = find(&plan, Profile::Cpu, |s| matches!(s, Stage::Pilot));
        // It follows the confirmation it repeats and descends from its pilot (whose rule it runs under).
        assert!(holdout.depends_on.contains(&confirm.study_sha256));
        assert!(holdout.depends_on.contains(&pilot.study_sha256));
        assert_eq!(holdout.min_attempts, confirm.min_attempts);
        assert_eq!(holdout.arms(), confirm.arms());
        let as_confirm = build(&parent, &confirm, &pair()).unwrap();
        let as_holdout = build(&parent, &holdout, &pair()).unwrap();
        // The same pair and the same frozen rule, but a different digest and so a different seed.
        assert_eq!(as_confirm.tools, as_holdout.tools);
        assert_eq!(as_confirm.confirmation_rule, as_holdout.confirmation_rule);
        assert_ne!(as_confirm.digest().unwrap(), as_holdout.digest().unwrap());
        assert_ne!(
            build_schedule(&as_confirm).unwrap()[0].study_sha256,
            build_schedule(&as_holdout).unwrap()[0].study_sha256
        );
        // Without its pilot's rule it cannot be built, like a confirmation.
        assert!(child_intent(&parent, &holdout, &pair(), None).is_err());
    }
}

#[test]
fn a_selector_naming_the_arms_in_any_order_still_yields_the_campaigns_own_tool_order() {
    let parent = parent();
    let confirm = plan(&parent)
        .unwrap()
        .into_iter()
        .find(|study| study.stage == Stage::Confirm)
        .unwrap();
    // The selector names the control first, the candidate second: the reverse of the campaign's order.
    let child = build(
        &parent,
        &confirm,
        &[ProtonSource::Ge, ProtonSource::ValveStable],
    )
    .unwrap();
    let in_parent_order: Vec<_> = parent
        .tools
        .iter()
        .filter(|tool| [ProtonSource::Ge, ProtonSource::ValveStable].contains(&tool.source))
        .cloned()
        .collect();
    assert_eq!(child.tools, in_parent_order);
    // And the verifier, which demands exactly that order, accepts it.
    let name = &confirm.study_sha256[..16];
    assert!(crate::autotune::research::child_verify::derived_from(
        &child, &parent, name
    ));
}
