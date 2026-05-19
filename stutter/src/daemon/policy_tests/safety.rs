//! Safety policy tests extracted from `daemon::policy`.
//!
//! Owns safety-class, effect-scope, rollback, confidence, persistent-effect, and system-wide permission gates.
//! Does not own mode parsing, capability/context gates, remote policy, explanation rendering, or production behavior.

use super::{super::*, descriptor, descriptor_with};

#[test]
fn suggest_rejects_system_wide_suggestion_without_permission() {
    let policy = DaemonPolicy::suggest(ActionSource::Test);
    let mut desc = descriptor_with(
        SafetyClass::HighRisk,
        ActionEffectScope::SystemWide,
        RollbackRequirement::Unavailable,
    );
    desc.touches_system_wide_state = true;

    assert!(matches!(
        policy.check_action(PolicyIntent::Suggest, &desc),
        Err(PolicyRejection::SystemWideActionBlocked)
    ));
}

#[test]
fn suggest_allows_system_wide_suggestion_with_suggestion_permission() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::Suggest,
        source: ActionSource::Test,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config.safety.allow_system_wide_suggestions = true;
    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    });
    let mut desc = descriptor_with(
        SafetyClass::HighRisk,
        ActionEffectScope::SystemWide,
        RollbackRequirement::Unavailable,
    );
    desc.touches_system_wide_state = true;

    assert!(policy.check_action(PolicyIntent::Suggest, &desc).is_ok());
    assert!(policy.allow_system_wide_suggestions);
    assert!(!policy.allow_system_wide_apply);
}

#[test]
fn apply_low_risk_allows_reversible_low_risk_local_process_tree_with_required_rollback() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor_with(
        SafetyClass::ReversibleLowRisk,
        ActionEffectScope::LocalProcessTree,
        RollbackRequirement::RequiredBeforeApply,
    );

    assert!(policy.check_action(PolicyIntent::Apply, &desc).is_ok());
}

#[test]
fn apply_low_risk_rejects_medium_risk() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor(SafetyClass::ReversibleMediumRisk);

    assert!(matches!(
        policy.check_action(PolicyIntent::Apply, &desc),
        Err(PolicyRejection::SafetyClassTooHigh {
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleMediumRisk
        })
    ));
}

#[test]
fn apply_low_risk_rejects_high_risk() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor(SafetyClass::HighRisk);

    assert!(matches!(
        policy.check_action(PolicyIntent::Apply, &desc),
        Err(PolicyRejection::SafetyClassTooHigh {
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::HighRisk
        })
    ));
}

#[test]
fn apply_low_risk_rejects_system_wide_even_when_low_risk() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let mut desc = descriptor_with(
        SafetyClass::ReversibleLowRisk,
        ActionEffectScope::LocalProcessTree,
        RollbackRequirement::RequiredBeforeApply,
    );
    desc.touches_system_wide_state = true;

    assert!(matches!(
        policy.check_action(PolicyIntent::Apply, &desc),
        Err(PolicyRejection::SystemWideActionBlocked)
    ));
}

#[test]
fn apply_low_risk_rejects_missing_rollback() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor_with(
        SafetyClass::ReversibleLowRisk,
        ActionEffectScope::LocalProcessTree,
        RollbackRequirement::BestEffortOnly,
    );

    assert!(matches!(
        policy.check_action(PolicyIntent::Apply, &desc),
        Err(PolicyRejection::RollbackRequired {
            rollback: RollbackRequirement::BestEffortOnly
        })
    ));
}

#[test]
fn apply_low_risk_rejects_low_confidence() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let mut desc = descriptor(SafetyClass::ReversibleLowRisk);
    desc.confidence = Some(0.10);

    assert!(matches!(
        policy.check_action(PolicyIntent::Apply, &desc),
        Err(PolicyRejection::ConfidenceTooLow { .. })
    ));
}

#[test]
fn apply_medium_risk_allows_medium_risk_only_when_explicit() {
    let medium_desc = descriptor(SafetyClass::ReversibleMediumRisk);
    let low_policy = DaemonPolicy::apply_low_risk(ActionSource::Test);

    assert!(matches!(
        low_policy.check_action(PolicyIntent::Apply, &medium_desc),
        Err(PolicyRejection::SafetyClassTooHigh {
            mode: DaemonMode::ApplyLowRisk,
            ..
        })
    ));

    let medium_policy = DaemonPolicy::apply_medium_risk(ActionSource::Test);
    assert!(
        medium_policy
            .check_action(PolicyIntent::Apply, &medium_desc)
            .is_ok()
    );
}

#[test]
fn apply_high_risk_is_disabled_even_with_explicit_high_risk_unlock() {
    let mut policy = DaemonPolicy::apply_high_risk_explicit(ActionSource::Test);
    policy.allow_high_risk = false;
    let high = descriptor(SafetyClass::HighRisk);

    assert!(matches!(
        policy.check_action(PolicyIntent::Apply, &high),
        Err(PolicyRejection::HighRiskApplyNotImplemented)
    ));

    let explicit = DaemonPolicy::apply_high_risk_explicit(ActionSource::Test);
    assert!(matches!(
        explicit.check_action(PolicyIntent::Apply, &high),
        Err(PolicyRejection::HighRiskApplyNotImplemented)
    ));
}

#[test]
fn apply_rejects_persistent_effect_without_explicit_permission() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let mut desc = descriptor(SafetyClass::ReversibleLowRisk);
    desc.persistent_effect = true;

    assert!(matches!(
        policy.check_action(PolicyIntent::Apply, &desc),
        Err(PolicyRejection::PersistentEffectBlocked)
    ));
}

#[test]
fn apply_rejects_unavailable_rollback() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor_with(
        SafetyClass::ReversibleLowRisk,
        ActionEffectScope::LocalProcessTree,
        RollbackRequirement::Unavailable,
    );

    assert!(matches!(
        policy.check_action(PolicyIntent::Apply, &desc),
        Err(PolicyRejection::RollbackUnavailable)
    ));
}
