//! Mode policy tests extracted from `daemon::policy`.
//!
//! Owns daemon mode parsing/serialization and observe/suggest mode policy gates.
//! Does not own safety-class gates, capability/context gates, remote policy, explanation rendering, or production behavior.

use super::{super::*, all_safety_classes, descriptor, descriptor_with};

#[test]
fn daemon_mode_parses_and_formats_kebab_case() {
    assert_eq!(
        // invariant: valid test mode parse
        "observe".parse::<DaemonMode>().unwrap(),
        DaemonMode::Observe
    );
    assert_eq!(
        // invariant: valid test mode parse
        "suggest".parse::<DaemonMode>().unwrap(),
        DaemonMode::Suggest
    );
    assert_eq!(
        // invariant: valid test mode parse
        "apply-low-risk".parse::<DaemonMode>().unwrap(),
        DaemonMode::ApplyLowRisk
    );
    assert_eq!(
        // invariant: valid test mode parse
        "apply-medium-risk".parse::<DaemonMode>().unwrap(),
        DaemonMode::ApplyMediumRisk
    );
    assert_eq!(
        // invariant: valid test mode parse
        "apply-high-risk".parse::<DaemonMode>().unwrap(),
        DaemonMode::ApplyHighRisk
    );
    assert_eq!(DaemonMode::ApplyHighRisk.to_string(), "apply-high-risk");
    assert!(matches!(
        "bad-mode".parse::<DaemonMode>(),
        Err(PolicyRejection::UnsupportedMode { .. })
    ));
}

#[test]
fn daemon_mode_serializes_as_kebab_case() {
    // invariant: serialize test mode
    let json = serde_json::to_string(&DaemonMode::ApplyMediumRisk).unwrap();

    assert_eq!(json, "\"apply-medium-risk\"");
}

#[test]
fn new_policy_fields_are_populated_by_existing_constructors() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);

    assert_eq!(policy.max_safety_class, SafetyClass::ReversibleLowRisk);
    assert!(
        policy
            .allowed_effect_scopes
            .contains(&ActionEffectScope::LocalProcess)
    );
    assert!(
        policy
            .allowed_effect_scopes
            .contains(&ActionEffectScope::LocalProcessTree)
    );
    assert!(policy.rollback_required_before_apply);
    assert!(!policy.allow_system_wide_suggestions);
    assert!(!policy.allow_system_wide_apply);
    assert!(!policy.allow_high_risk);
    assert!(!policy.allow_persistent_effects);
    assert_eq!(policy.confidence.min_suggest_confidence, 0.50);
    assert_eq!(
        policy.confidence.min_apply_low_risk_confidence,
        policy.min_confidence
    );
    assert_eq!(policy.confidence.min_apply_medium_risk_confidence, 0.85);
    assert_eq!(policy.confidence.min_high_risk_suggestion_confidence, 0.90);
    assert!(!policy.remote_apply.allow_remote_apply);
}

#[test]
fn observe_rejects_apply_for_all_safety_classes() {
    let policy = DaemonPolicy::observe(ActionSource::Test);

    for safety_class in all_safety_classes() {
        let desc = descriptor(safety_class);
        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::IntentNotAllowed {
                mode: DaemonMode::Observe,
                intent: PolicyIntent::Apply
            })
        ));
    }
}

#[test]
fn observe_allows_dry_run_for_all_safety_classes() {
    let policy = DaemonPolicy::observe(ActionSource::Test);

    for safety_class in all_safety_classes() {
        let desc = descriptor_with(
            safety_class,
            ActionEffectScope::SystemWide,
            RollbackRequirement::Unavailable,
        );
        assert!(policy.check_action(PolicyIntent::DryRun, &desc).is_ok());
    }
}

#[test]
fn suggest_rejects_apply_for_all_safety_classes() {
    let policy = DaemonPolicy::suggest(ActionSource::Test);

    for safety_class in all_safety_classes() {
        let desc = descriptor(safety_class);
        assert!(matches!(
            policy.check_action(PolicyIntent::Apply, &desc),
            Err(PolicyRejection::IntentNotAllowed {
                mode: DaemonMode::Suggest,
                intent: PolicyIntent::Apply
            })
        ));
    }
}
