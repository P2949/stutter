//! Capability and context policy tests extracted from `daemon::policy`.
//!
//! Owns data-quality, system-health, rollback/cooldown, and daemon capability gate tests.
//! Does not own mode parsing, safety-class gates, remote policy, explanation rendering, or production behavior.

use super::{super::*, all_capabilities_available, descriptor, descriptor_with};
use crate::daemon::explain::PolicyDecisionKind;

#[test]
fn daemon_policy_context_blocks_apply_on_bad_data_quality() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor(SafetyClass::ReversibleLowRisk);
    let context = DaemonPolicyContext {
        data_quality_ok: false,
        data_quality_reason_code: Some("insufficient_samples".to_owned()),
        ..DaemonPolicyContext::default()
    };

    let explanation = policy.explain_action_with_context(PolicyIntent::Apply, &desc, &context);

    assert!(matches!(
        explanation.decision,
        PolicyDecisionKind::Rejected {
            rejection: PolicyRejection::DataQualityBlocked { .. }
        }
    ));
    assert_eq!(
        policy
            .check_action_with_context(PolicyIntent::Apply, &desc, &context)
            .unwrap_err()
            .reason_code(),
        "data_quality_blocked"
    );
    assert!(
        explanation
            .evaluated_rules
            .iter()
            .any(|rule| rule.rule == "data_quality_gate" && !rule.passed)
    );
}

#[test]
fn daemon_policy_context_blocks_apply_when_health_or_state_is_unsafe() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let desc = descriptor(SafetyClass::ReversibleLowRisk);

    for (context, expected) in [
        (
            DaemonPolicyContext {
                system_health_ok: false,
                system_health_reason_code: Some("thermal_degraded".to_owned()),
                ..DaemonPolicyContext::default()
            },
            "system_health_blocked",
        ),
        (
            DaemonPolicyContext {
                workload_stable: false,
                ..DaemonPolicyContext::default()
            },
            "workload_unstable",
        ),
        (
            DaemonPolicyContext {
                cooldown_active: true,
                ..DaemonPolicyContext::default()
            },
            "cooldown_active",
        ),
        (
            DaemonPolicyContext {
                rollback_pending: true,
                ..DaemonPolicyContext::default()
            },
            "rollback_pending",
        ),
    ] {
        let rejection = policy
            .check_action_with_context(PolicyIntent::Apply, &desc, &context)
            .unwrap_err();

        assert_eq!(rejection.reason_code(), expected);
    }
}

#[test]
fn daemon_policy_context_can_be_derived_from_health_snapshot() {
    let context = DaemonPolicyContext::default().with_system_health(
        &crate::daemon::health::SystemHealthSnapshot {
            ok_for_apply: false,
            reason_code: Some("low_disk".to_owned()),
            ..crate::daemon::health::SystemHealthSnapshot::default()
        },
    );

    assert!(!context.system_health_ok);
    assert_eq!(
        context.system_health_reason_code.as_deref(),
        Some("low_disk")
    );
}

#[test]
fn daemon_policy_context_does_not_block_observe_intent() {
    let policy = DaemonPolicy::observe(ActionSource::Test);
    let desc = descriptor_with(
        SafetyClass::HighRisk,
        ActionEffectScope::SystemWide,
        RollbackRequirement::Unavailable,
    );
    let context = DaemonPolicyContext {
        data_quality_ok: false,
        system_health_ok: false,
        workload_stable: false,
        cooldown_active: true,
        rollback_pending: true,
        ..DaemonPolicyContext::default()
    };

    let explanation = policy.explain_action_with_context(PolicyIntent::Observe, &desc, &context);

    assert!(matches!(explanation.decision, PolicyDecisionKind::Allowed));
    assert!(
        explanation
            .evaluated_rules
            .iter()
            .filter(|rule| rule.rule.ends_with("_gate"))
            .all(|rule| rule.passed)
    );
}

#[test]
fn daemon_policy_context_blocks_unsupported_action_capabilities() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let mut desc = descriptor(SafetyClass::ReversibleLowRisk);
    desc.action_kind = "uclamp:min".to_owned();
    let mut capabilities = all_capabilities_available();
    capabilities.uclamp_available = false;
    let context = DaemonPolicyContext {
        capabilities: Some(capabilities),
        ..DaemonPolicyContext::default()
    };

    let explanation = policy.explain_action_with_context(PolicyIntent::Apply, &desc, &context);

    assert!(matches!(
        explanation.decision,
        PolicyDecisionKind::Rejected {
            rejection: PolicyRejection::CapabilityUnavailable {
                feature: "uclamp",
                ..
            }
        }
    ));
    assert!(
        explanation
            .evaluated_rules
            .iter()
            .any(|rule| rule.rule == "capability_gate" && !rule.passed)
    );
}

#[test]
fn daemon_policy_context_uses_capabilities_for_dry_run_and_suggest_but_not_observe() {
    let policy = DaemonPolicy::suggest(ActionSource::Test);
    let mut desc = descriptor_with(
        SafetyClass::ReversibleLowRisk,
        ActionEffectScope::LocalProcessTree,
        RollbackRequirement::RequiredBeforeApply,
    );
    desc.action_kind = "ionice".to_owned();
    let mut capabilities = all_capabilities_available();
    capabilities.ionice_available = false;
    let context = DaemonPolicyContext {
        capabilities: Some(capabilities),
        ..DaemonPolicyContext::default()
    };

    let dry_run_rejection = policy
        .check_action_with_context(PolicyIntent::DryRun, &desc, &context)
        .unwrap_err();
    let suggest_rejection = policy
        .check_action_with_context(PolicyIntent::Suggest, &desc, &context)
        .unwrap_err();
    let observe = policy.check_action_with_context(PolicyIntent::Observe, &desc, &context);

    assert_eq!(dry_run_rejection.reason_code(), "capability_unavailable");
    assert_eq!(suggest_rejection.reason_code(), "capability_unavailable");
    assert!(observe.is_ok());
}
