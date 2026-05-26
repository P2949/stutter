use super::super::{
    ActionSource, DaemonPolicy, PolicyRejection,
    remote::{record_remote_apply_rules, record_remote_target_count_rule},
};
use crate::daemon::explain::PolicyRuleEvaluation;

pub(super) fn remote_target_count_blocks_non_apply(
    policy: &DaemonPolicy,
    evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
    first_rejection: &mut Option<PolicyRejection>,
) -> bool {
    if policy.source == ActionSource::RemoteAgent
        && !policy.mode.supports_apply()
        && !policy.remote_apply.target_count_allowed
    {
        record_remote_target_count_rule(policy, evaluated_rules, first_rejection);
        true
    } else {
        false
    }
}

pub(super) fn record_remote_apply_gates(
    policy: &DaemonPolicy,
    evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
    first_rejection: &mut Option<PolicyRejection>,
) {
    if policy.source == ActionSource::RemoteAgent && policy.mode.supports_apply() {
        record_remote_apply_rules(policy, evaluated_rules, first_rejection);
    }
}
