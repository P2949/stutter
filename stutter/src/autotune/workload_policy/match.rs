use super::model::WorkloadPolicyRule;
use crate::autotune::{objective::ObjectiveKind, planning::candidate::CandidateAction};

impl WorkloadPolicyRule {
    pub fn allows_candidate(&self, candidate: &CandidateAction) -> bool {
        self.allowed_families
            .iter()
            .any(|family| family_matches(candidate.action_kind(), family))
    }

    pub fn allows_autonomous_candidate(&self, candidate: &CandidateAction) -> bool {
        self.autonomous_families
            .iter()
            .any(|family| family_matches(candidate.action_kind(), family))
    }

    pub fn allows_objective(&self, objective: ObjectiveKind) -> bool {
        self.allowed_objectives.is_empty() || self.allowed_objectives.contains(&objective)
    }
}

pub(super) fn family_matches(action_kind: &str, family: &str) -> bool {
    action_kind == family
        || action_kind.strip_prefix(family).is_some_and(|suffix| {
            matches!(
                suffix.as_bytes().first(),
                Some(b':') | Some(b'-') | Some(b'_')
            )
        })
}
