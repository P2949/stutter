//! Golden policy decision snapshots.
//!
//! Owns fixture-backed coverage for policy decision shape drift.

use super::{super::*, descriptor};
use crate::{daemon::explain::PolicyExplanation, remote::AgentAutotuneLimits};

#[test]
fn observe_policy_decision_snapshot_matches_fixture() {
    let policy = DaemonPolicy::observe(ActionSource::Test);
    let explanation = policy.explain_action(
        PolicyIntent::Apply,
        &descriptor(SafetyClass::ReversibleLowRisk),
    );

    assert_policy_snapshot(
        &explanation,
        include_str!("../../../tests/fixtures/policy/observe.json"),
    );
}

#[test]
fn apply_low_risk_policy_decision_snapshot_matches_fixture() {
    let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let explanation = policy.explain_action(
        PolicyIntent::Apply,
        &descriptor(SafetyClass::ReversibleLowRisk),
    );

    assert_policy_snapshot(
        &explanation,
        include_str!("../../../tests/fixtures/policy/apply_low_risk.json"),
    );
}

#[test]
fn remote_non_loopback_policy_decision_snapshot_matches_fixture() {
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
    let explanation = policy.explain_action(
        PolicyIntent::Apply,
        &descriptor(SafetyClass::ReversibleLowRisk),
    );

    assert_policy_snapshot(
        &explanation,
        include_str!("../../../tests/fixtures/policy/remote_non_loopback.json"),
    );
}

fn assert_policy_snapshot(explanation: &PolicyExplanation, expected: &str) {
    let actual = serde_json::to_string_pretty(explanation).expect("policy explanation serializes");
    let expected = expected.trim();

    assert!(
        actual == expected,
        "policy snapshot mismatch; update fixture to:\n{actual}\n"
    );
}
