use serde::{Deserialize, Serialize};

use crate::{
    actions::ActionId,
    daemon::policy::{ActionSource, DaemonMode, DaemonPolicy, PolicyIntent, PolicyRejection},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyExplainLine {
    pub rule: String,
    pub outcome: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonPolicyExplanation {
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub lines: Vec<PolicyExplainLine>,
}

impl DaemonPolicyExplanation {
    pub fn from_policy(policy: &DaemonPolicy) -> Self {
        Self {
            mode: policy.mode,
            source: policy.source,
            lines: vec![PolicyExplainLine {
                rule: "policy_model".to_owned(),
                outcome: "available".to_owned(),
                reason: "daemon policy model is available; action-level explanations are added in a later patch".to_owned(),
            }],
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PolicyExplanation {
    pub decision: PolicyDecisionKind,
    pub intent: PolicyIntent,
    pub action_id: ActionId,
    pub action_kind: String,
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub evaluated_rules: Vec<PolicyRuleEvaluation>,
    pub final_reason: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyDecisionKind {
    Allowed,
    Rejected { rejection: PolicyRejection },
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PolicyRuleEvaluation {
    pub rule: &'static str,
    pub passed: bool,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::ActionId,
        daemon::policy::{ActionSource, DaemonPolicy, PolicyIntent},
    };

    #[test]
    fn policy_explanation_can_be_rendered_from_policy() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);

        let explanation = DaemonPolicyExplanation::from_policy(&policy);

        assert_eq!(explanation.mode, policy.mode);
        assert_eq!(explanation.source, policy.source);
        assert_eq!(explanation.lines.len(), 1);
        assert_eq!(explanation.lines[0].rule, "policy_model");
    }

    #[test]
    fn structured_policy_explanation_serializes() {
        let explanation = PolicyExplanation {
            decision: PolicyDecisionKind::Allowed,
            intent: PolicyIntent::Apply,
            action_id: ActionId("test-action".to_owned()),
            action_kind: "test".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::Test,
            evaluated_rules: vec![PolicyRuleEvaluation {
                rule: "intent_allowed",
                passed: true,
                reason: "apply intent is allowed in daemon mode apply-low-risk".to_owned(),
            }],
            final_reason: "action is allowed by daemon policy".to_owned(),
        };

        let json = serde_json::to_string(&explanation).unwrap();

        assert!(json.contains("\"decision\""));
        assert!(json.contains("\"evaluated_rules\""));
        assert!(json.contains("intent_allowed"));
    }
}
