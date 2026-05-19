//! Remote policy tests extracted from `daemon::policy`.
//!
//! Owns remote policy context, remote limit, remote auth, remote bind, and remote target-count tests.
//! Does not own local mode parsing, local safety gates, capability/context gates, explanation rendering, or production behavior.

use super::{super::*, descriptor};

#[test]
fn build_daemon_policy_remote_context_is_deterministic_and_respects_limits() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::ApplyLowRisk,
        source: ActionSource::RemoteAgent,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config.remote.allow_remote_apply = true;
    config.safety.allow_system_wide_suggestions = true;
    config.safety.allow_system_wide_apply = true;

    let remote_context = RemotePolicyContext {
        bind_is_loopback: true,
        auth_configured: true,
        request_authorized: true,
        limits: AgentAutotuneLimits {
            max_targets: 3,
            ..AgentAutotuneLimits::default()
        },
    };

    let first = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(remote_context.clone()),
    });
    let second = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(remote_context),
    });

    assert_eq!(first, second);
    assert!(first.remote_apply.allow_remote_apply);
    assert!(first.remote_apply.require_loopback_bind);
    assert!(first.remote_apply.require_auth);
    assert_eq!(first.remote_apply.max_remote_targets, 3);
    assert!(!first.allow_system_wide_suggestions);
    assert!(!first.allow_system_wide_apply);
}

#[test]
fn remote_limits_cap_system_wide_suggestion_and_apply_separately() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::ApplyHighRisk,
        source: ActionSource::RemoteAgent,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config.remote.allow_remote_apply = true;
    config.safety.allow_high_risk = true;
    config.safety.allow_system_wide_suggestions = true;
    config.safety.allow_system_wide_apply = true;

    let suggestion_only_context = RemotePolicyContext {
        bind_is_loopback: true,
        auth_configured: true,
        request_authorized: true,
        limits: AgentAutotuneLimits {
            allow_system_wide_suggestions: true,
            allow_system_wide_apply: false,
            allow_high_risk: true,
            max_mode: DaemonMode::ApplyHighRisk,
            max_safety_class: SafetyClass::HighRisk,
            ..AgentAutotuneLimits::default()
        },
    };

    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(suggestion_only_context),
    });

    assert!(policy.allow_system_wide_suggestions);
    assert!(!policy.allow_system_wide_apply);
}

#[test]
fn remote_apply_explanation_rejects_unconfigured_auth() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::ApplyLowRisk,
        source: ActionSource::RemoteAgent,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config.remote.allow_remote_apply = true;
    config.target.tree_pids.push(1234);

    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(RemotePolicyContext {
            bind_is_loopback: true,
            auth_configured: false,
            request_authorized: true,
            limits: AgentAutotuneLimits::default(),
        }),
    });
    let desc = descriptor(SafetyClass::ReversibleLowRisk);

    let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

    assert!(matches!(
        explanation.decision,
        PolicyDecisionKind::Rejected {
            rejection: PolicyRejection::RemoteApplyRequiresConfiguredAuth
        }
    ));
    assert!(
        explanation
            .evaluated_rules
            .iter()
            .any(|rule| rule.rule == "remote_auth_configured" && !rule.passed)
    );
    assert!(explanation.final_reason.contains("configured bearer token"));
}

#[test]
fn remote_apply_explanation_rejects_non_loopback_bind() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::ApplyLowRisk,
        source: ActionSource::RemoteAgent,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config.remote.allow_remote_apply = true;
    config.target.tree_pids.push(1234);

    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(RemotePolicyContext {
            bind_is_loopback: false,
            auth_configured: true,
            request_authorized: true,
            limits: AgentAutotuneLimits::default(),
        }),
    });
    let desc = descriptor(SafetyClass::ReversibleLowRisk);

    let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

    assert!(matches!(
        explanation.decision,
        PolicyDecisionKind::Rejected {
            rejection: PolicyRejection::RemoteApplyRequiresLoopbackBind
        }
    ));
    assert!(
        explanation
            .evaluated_rules
            .iter()
            .any(|rule| rule.rule == "remote_loopback_bind" && !rule.passed)
    );
    assert!(explanation.final_reason.contains("loopback"));
}

#[test]
fn remote_apply_high_risk_reports_disabled_before_remote_auth() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::ApplyHighRisk,
        source: ActionSource::RemoteAgent,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config.remote.allow_remote_apply = true;
    config.safety.allow_high_risk = true;
    config.target.tree_pids.push(1234);

    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(RemotePolicyContext {
            bind_is_loopback: true,
            auth_configured: false,
            request_authorized: true,
            limits: AgentAutotuneLimits::default(),
        }),
    });
    let desc = descriptor(SafetyClass::HighRisk);

    let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

    assert!(matches!(
        explanation.decision,
        PolicyDecisionKind::Rejected {
            rejection: PolicyRejection::HighRiskApplyNotImplemented
        }
    ));
    let first_failed_rule = explanation
        .evaluated_rules
        .iter()
        .find(|rule| !rule.passed)
        // invariant: remote policy tests expect a failing remote rule here
        .expect("expected a failing remote policy rule");
    assert_eq!(first_failed_rule.rule, "high_risk_apply_support");
    assert!(
        explanation
            .final_reason
            .contains("high-risk apply is not implemented")
    );
}

#[test]
fn remote_apply_explanation_rejects_mode_over_limits() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::ApplyHighRisk,
        source: ActionSource::RemoteAgent,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config.remote.allow_remote_apply = true;
    config.safety.allow_high_risk = true;
    config.target.tree_pids.push(1234);

    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(RemotePolicyContext {
            bind_is_loopback: true,
            auth_configured: true,
            request_authorized: true,
            limits: AgentAutotuneLimits::default(),
        }),
    });
    let desc = descriptor(SafetyClass::HighRisk);

    let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

    assert!(matches!(
        explanation.decision,
        PolicyDecisionKind::Rejected {
            rejection: PolicyRejection::HighRiskApplyNotImplemented
        }
    ));
    assert!(
        explanation
            .evaluated_rules
            .iter()
            .any(|rule| rule.rule == "high_risk_apply_support" && !rule.passed)
    );
    assert!(
        explanation
            .final_reason
            .contains("high-risk apply is not implemented")
    );
}

#[test]
fn remote_apply_explanation_rejects_target_count_over_limits() {
    let mut config = crate::daemon::config::DaemonConfig {
        mode: DaemonMode::ApplyLowRisk,
        source: ActionSource::RemoteAgent,
        ..crate::daemon::config::DaemonConfig::default()
    };
    config.remote.allow_remote_apply = true;
    config.target.tree_pids.push(1234);
    config.target.watch_process = Some("game".to_owned());

    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(RemotePolicyContext {
            bind_is_loopback: true,
            auth_configured: true,
            request_authorized: true,
            limits: AgentAutotuneLimits::default(),
        }),
    });
    let desc = descriptor(SafetyClass::ReversibleLowRisk);

    let explanation = policy.explain_action(PolicyIntent::Apply, &desc);

    assert!(matches!(
        explanation.decision,
        PolicyDecisionKind::Rejected {
            rejection: PolicyRejection::RemoteTargetCountTooHigh {
                target_count: 2,
                max_targets: 1
            }
        }
    ));
    assert!(
        explanation
            .evaluated_rules
            .iter()
            .any(|rule| rule.rule == "remote_target_count" && !rule.passed)
    );
    assert!(explanation.final_reason.contains("max_targets"));
}
