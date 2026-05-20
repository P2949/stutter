//! Policy explanation tests extracted from `daemon::policy`.
//!
//! Owns policy verdict, explanation, deterministic build, enabled-family, and denied-family tests.
//! Does not own mode parsing, safety-class gates, capability/context gates, remote policy, or production behavior.

use super::{super::*, descriptor, descriptor_with};
use crate::daemon::explain::PolicyDecisionKind;

#[test]
fn policy_verdicts_distinguish_delay_observe_only_manual_and_reject() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor(SafetyClass::ReversibleLowRisk);

    assert_eq!(
        policy.verdict_for_action_with_context(
            PolicyIntent::Apply,
            &desc,
            &DaemonPolicyContext {
                cooldown_active: true,
                ..DaemonPolicyContext::default()
            },
        ),
        DaemonPolicyVerdict::Delay
    );
    assert_eq!(
        policy.verdict_for_action_with_context(
            PolicyIntent::Apply,
            &desc,
            &DaemonPolicyContext {
                system_health_ok: false,
                system_health_reason_code: Some("thermal_degraded".to_owned()),
                ..DaemonPolicyContext::default()
            },
        ),
        DaemonPolicyVerdict::RequireObserveOnly
    );

    let mut high_risk = descriptor(SafetyClass::HighRisk);
    high_risk.confidence = Some(1.0);
    assert_eq!(
        policy.verdict_for_action(PolicyIntent::Apply, &high_risk),
        DaemonPolicyVerdict::RequireManualConfirmation
    );

    let no_rollback = descriptor_with(
        SafetyClass::ReversibleLowRisk,
        ActionEffectScope::LocalProcessTree,
        RollbackRequirement::Unavailable,
    );
    assert_eq!(
        policy.verdict_for_action(PolicyIntent::Apply, &no_rollback),
        DaemonPolicyVerdict::Reject
    );
}

#[test]
fn policy_explanation_exposes_verdict_for_machine_clients() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor(SafetyClass::ReversibleLowRisk);

    let allowed = policy.explain_action(PolicyIntent::Apply, &desc);
    let delayed = policy.explain_action_with_context(
        PolicyIntent::Apply,
        &desc,
        &DaemonPolicyContext {
            cooldown_active: true,
            ..DaemonPolicyContext::default()
        },
    );

    assert_eq!(allowed.verdict, DaemonPolicyVerdict::Allow);
    assert!(allowed.verdict.is_allowed());
    assert_eq!(delayed.verdict, DaemonPolicyVerdict::Delay);
    assert!(!delayed.verdict.is_allowed());
    assert_eq!(DaemonPolicyVerdict::Delay.as_str(), "delay");
}

#[test]
fn explain_action_allowed_includes_identity_final_reason_and_rules() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor_with(
        SafetyClass::ReversibleLowRisk,
        ActionEffectScope::LocalProcessTree,
        RollbackRequirement::RequiredBeforeApply,
    );

    let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

    assert!(matches!(explanation.decision, PolicyDecisionKind::Allowed));
    assert_eq!(explanation.intent, PolicyIntent::Apply);
    assert_eq!(explanation.action_id, desc.action_id);
    assert_eq!(explanation.action_kind, "test");
    assert_eq!(explanation.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(explanation.source, ActionSource::Test);
    assert_eq!(
        explanation.final_reason,
        "action is allowed by daemon policy"
    );
    assert!(
        explanation
            .evaluated_rules
            .iter()
            .any(|rule| { rule.rule == "rollback_available" && rule.passed })
    );
}

#[test]
fn explain_action_rejection_includes_identity_final_reason_and_failing_rule() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor(SafetyClass::ReversibleMediumRisk);

    let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

    assert!(matches!(
        explanation.decision,
        PolicyDecisionKind::Rejected {
            rejection: PolicyRejection::SafetyClassTooHigh {
                mode: DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::ReversibleMediumRisk
            }
        }
    ));
    assert_eq!(explanation.intent, PolicyIntent::Apply);
    assert_eq!(explanation.action_id, desc.action_id);
    assert_eq!(explanation.action_kind, "test");
    assert_eq!(explanation.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(explanation.source, ActionSource::Test);
    assert!(
        explanation
            .final_reason
            .contains("safety class ReversibleMediumRisk")
    );

    let failing_rule = explanation
        .evaluated_rules
        .iter()
        .find(|rule| !rule.passed)
        // invariant: policy tests expect a failing rule here
        .expect("expected a failing policy rule");
    assert_eq!(failing_rule.rule, "safety_class_allowed");
    assert!(failing_rule.reason.contains("ReversibleMediumRisk"));
}

#[test]
fn build_daemon_policy_is_deterministic_for_same_config() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::ApplyMediumRisk,
        source: ActionSource::Test,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config.safety.allow_system_wide_suggestions = true;
    config.safety.allow_system_wide_apply = true;
    config.safety.allow_persistent_effects = true;
    config
        .safety
        .denied_action_families
        .insert("gpu-power".to_owned());

    let first = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    });
    let second = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    });

    assert_eq!(first, second);
    assert_eq!(first.mode, DaemonMode::ApplyMediumRisk);
    assert_eq!(first.max_safety_class, SafetyClass::ReversibleMediumRisk);
    assert!(first.allow_system_wide_suggestions);
    assert!(!first.allow_system_wide_apply);
    assert!(first.allow_persistent_effects);
    assert!(first.denied_action_families.contains("gpu-power"));
}

#[test]
fn preset_enabled_action_families_gate_apply_actions() {
    let config = crate::daemon::config::DaemonConfig::from_preset(
        crate::daemon::config::DaemonPreset::GamingLowRisk,
        ActionSource::Test,
    );
    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    });

    let mut cpu = descriptor(SafetyClass::ReversibleLowRisk);
    cpu.action_kind = "cpu_affinity_profile".to_owned();
    assert!(policy.check_action(PolicyIntent::Apply, &cpu).is_ok());

    let mut nice = descriptor(SafetyClass::ReversibleLowRisk);
    nice.action_kind = "nice".to_owned();
    let rejection = policy.check_action(PolicyIntent::Apply, &nice).unwrap_err();

    assert_eq!(rejection.reason_code(), "action_family_not_enabled");
}

#[test]
fn denied_action_families_gate_apply_actions() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::ApplyLowRisk,
        source: ActionSource::Test,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config
        .safety
        .denied_action_families
        .insert("cpu_affinity_profile".to_owned());
    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    });
    let mut desc = descriptor(SafetyClass::ReversibleLowRisk);
    desc.action_kind = "cpu_affinity_profile".to_owned();

    let rejection = policy.check_action(PolicyIntent::Apply, &desc).unwrap_err();

    assert_eq!(rejection.reason_code(), "action_family_denied");
}
