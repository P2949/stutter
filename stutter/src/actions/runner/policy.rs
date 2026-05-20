//! Policy-check adapter for audited action runs.
//!
//! Owns mapping daemon-policy explanations into action-runner errors. It must not execute actions,
//! write audit events, or perform rollback.

use crate::{
    actions::ActionError,
    daemon_policy::{
        ActionDescriptor, DaemonPolicy, DaemonPolicyContext, PolicyDecisionKind, PolicyIntent,
    },
};

pub(super) fn check_action_with_explanation(
    policy: &DaemonPolicy,
    context: &DaemonPolicyContext,
    intent: PolicyIntent,
    descriptor: &ActionDescriptor,
) -> Result<(), ActionError> {
    let explanation = policy.explain_action_with_context(intent, descriptor, context);
    match explanation.decision {
        PolicyDecisionKind::Allowed => Ok(()),
        PolicyDecisionKind::Rejected { .. } => {
            Err(ActionError::policy_rejected(explanation.final_reason))
        }
    }
}
