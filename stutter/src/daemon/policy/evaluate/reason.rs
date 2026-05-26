use super::super::{
    ActionDescriptor, DaemonPolicy, DaemonPolicyVerdict, PolicyIntent, PolicyRejection,
};
use crate::{
    daemon::explain::{PolicyDecisionKind, PolicyExplanation, PolicyRuleEvaluation},
    error::DaemonPolicyError,
};

pub(super) fn finish_policy_explanation(
    policy: &DaemonPolicy,
    intent: PolicyIntent,
    descriptor: &ActionDescriptor,
    evaluated_rules: Vec<PolicyRuleEvaluation>,
    rejection: Option<PolicyRejection>,
) -> PolicyExplanation {
    let (verdict, decision, final_reason) = match rejection {
        Some(rejection) => {
            let verdict = rejection.verdict();
            let final_reason = rejection.to_string();
            (
                verdict,
                PolicyDecisionKind::Rejected { rejection },
                final_reason,
            )
        }
        None => (
            DaemonPolicyVerdict::Allow,
            PolicyDecisionKind::Allowed,
            "action is allowed by daemon policy".to_owned(),
        ),
    };

    PolicyExplanation {
        verdict,
        decision,
        intent,
        action_id: descriptor.action_id.clone(),
        action_kind: descriptor.action_kind.clone(),
        mode: policy.mode,
        source: policy.source,
        evaluated_rules,
        final_reason,
    }
}

pub(in crate::daemon::policy) fn record_policy_rule(
    evaluated_rules: &mut Vec<PolicyRuleEvaluation>,
    first_rejection: &mut Option<PolicyRejection>,
    rule: &'static str,
    passed: bool,
    reason: String,
    rejection: Option<PolicyRejection>,
) {
    evaluated_rules.push(PolicyRuleEvaluation {
        rule,
        passed,
        reason,
    });

    if !passed && first_rejection.is_none() {
        *first_rejection = rejection;
    }
}

pub(super) fn validate_policy_descriptor_shape(
    descriptor: &ActionDescriptor,
) -> Result<(), DaemonPolicyError> {
    if descriptor.action_kind.trim().is_empty() {
        return Err(DaemonPolicyError::InvalidInput {
            message: "action_kind must not be empty".to_owned(),
        });
    }
    Ok(())
}
