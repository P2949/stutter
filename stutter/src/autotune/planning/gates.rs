//! Candidate safety and eligibility gate helpers.

use super::candidate::CandidateAction;
use crate::actions::SafetyClass;

impl CandidateAction {
    pub fn is_high_risk_system_adjacent(&self) -> bool {
        let descriptor = self.descriptor();
        descriptor.safety_class == SafetyClass::HighRisk
    }

    pub fn manual_only_reason(&self) -> Option<String> {
        self.is_high_risk_system_adjacent().then(|| {
            format!(
                "manual-only high-risk/system-adjacent candidate; autonomous apply is disabled for action_kind={}",
                self.action_kind()
            )
        })
    }
}
