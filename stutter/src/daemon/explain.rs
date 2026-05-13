use serde::{Deserialize, Serialize};

use crate::daemon::policy::{ActionSource, DaemonMode, DaemonPolicy};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::policy::{ActionSource, DaemonPolicy};

    #[test]
    fn policy_explanation_can_be_rendered_from_policy() {
        let policy = DaemonPolicy::apply_low_risk(ActionSource::Test);

        let explanation = DaemonPolicyExplanation::from_policy(&policy);

        assert_eq!(explanation.mode, policy.mode);
        assert_eq!(explanation.source, policy.source);
        assert_eq!(explanation.lines.len(), 1);
        assert_eq!(explanation.lines[0].rule, "policy_model");
    }
}
