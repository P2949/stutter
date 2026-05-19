#![allow(dead_code)] // Transitional pure evaluator target while DaemonPolicy owns evaluation.

use super::{ActionDescriptor, DaemonPolicy, PolicyDecisionKind, PolicyIntent};

pub(crate) struct PolicyInput<'a> {
    pub policy: &'a DaemonPolicy,
    pub intent: PolicyIntent,
    pub descriptor: &'a ActionDescriptor,
}

pub(crate) struct PolicyDecision {
    pub kind: PolicyDecisionKind,
}

pub(crate) fn evaluate_policy(input: PolicyInput<'_>) -> PolicyDecision {
    PolicyDecision {
        kind: input
            .policy
            .explain_action(input.intent, input.descriptor)
            .decision,
    }
}
