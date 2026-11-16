use super::*;
use crate::autotune::research::{
    ProtonSource, ResearchTool, child_study::plan, driver::build_schedule, tests::intent,
};

fn parent() -> ResearchIntentV1 {
    let mut intent = intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.max_attempts = 40;
    intent
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
fn every_child_has_its_own_digest_schedule_and_budget_even_with_equal_attempt_counts() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let cpu_screen = child_intent(&parent, &plan[0]).unwrap();
    let gpu_screen = child_intent(&parent, &plan[1]).unwrap();
    // The two screens need the same number of attempts yet are different studies.
    assert_eq!(cpu_screen.max_attempts, gpu_screen.max_attempts);
    assert_ne!(cpu_screen.digest().unwrap(), gpu_screen.digest().unwrap());
    assert_ne!(cpu_screen.digest().unwrap(), parent.digest().unwrap());
    let a = build_schedule(&cpu_screen).unwrap();
    let b = build_schedule(&gpu_screen).unwrap();
    assert_eq!(a.len(), plan[0].min_attempts as usize);
    assert_ne!(
        a[0].study_sha256, b[0].study_sha256,
        "slots never collide across siblings"
    );
    assert_eq!(
        a[0].context_sha256, b[0].context_sha256,
        "the host context is the same"
    );
    // Deterministic: a verifier regenerates the same schedule from the same child intent.
    assert_eq!(a, build_schedule(&cpu_screen).unwrap());
}

#[test]
fn nothing_but_the_budget_and_the_binding_differs_from_the_parent() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let child = child_intent(&parent, &plan[2]).unwrap();
    let mut expected = parent.clone();
    expected.max_attempts = plan[2].min_attempts;
    expected.child_study_sha256 = Some(plan[2].study_sha256.clone());
    assert_eq!(child, expected);
}

#[test]
fn a_study_that_does_not_fit_the_blocks_the_budget_or_is_nested_is_refused() {
    let parent = parent();
    let plan = plan(&parent).unwrap();
    let mut odd = plan[0].clone();
    odd.min_attempts = 3;
    assert!(
        child_intent(&parent, &odd).is_err(),
        "not whole blocks across 2 tools"
    );
    let mut zero = plan[0].clone();
    zero.min_attempts = 0;
    assert!(child_intent(&parent, &zero).is_err());
    let mut small = parent.clone();
    small.max_attempts = 2;
    assert!(
        child_intent(&small, &plan[2]).is_err(),
        "more than the campaign allows"
    );
    let nested = child_intent(&parent, &plan[0]).unwrap();
    assert!(child_intent(&nested, &plan[1]).is_err());
    let mut bad = parent.clone();
    bad.child_study_sha256 = Some("xyz".into());
    assert!(
        bad.validate().is_err(),
        "a malformed child digest is invalid"
    );
}
