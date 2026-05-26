use super::super::{ActionDescriptor, DaemonMode, DaemonPolicy};
use crate::actions::SafetyClass;

pub(super) fn provider_confidence_threshold(
    policy: &DaemonPolicy,
    descriptor: &ActionDescriptor,
) -> f32 {
    match policy.mode {
        DaemonMode::Observe => 1.0,
        DaemonMode::Suggest if descriptor.safety_class == SafetyClass::HighRisk => {
            policy.confidence.min_high_risk_suggestion_confidence
        }
        DaemonMode::Suggest => policy.confidence.min_suggest_confidence,
        DaemonMode::ApplyLowRisk => policy.confidence.min_apply_low_risk_confidence,
        DaemonMode::ApplyMediumRisk => policy.confidence.min_apply_medium_risk_confidence,
        DaemonMode::ApplyHighRisk => policy.confidence.min_high_risk_suggestion_confidence,
    }
}
