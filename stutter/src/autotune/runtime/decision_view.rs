//! Decision label and display helpers for runtime stream/history views.

use crate::{
    actions::SafetyClass,
    autotune::{decision::AutotuneDecision, quality::OnlineDataQuality},
};

pub(crate) fn decision_reason(decision: &AutotuneDecision) -> String {
    match decision {
        AutotuneDecision::Noop { reason }
        | AutotuneDecision::Suggest { reason, .. }
        | AutotuneDecision::StartExperiment { reason, .. }
        | AutotuneDecision::KeepCurrent { reason, .. }
        | AutotuneDecision::Revert { reason, .. }
        | AutotuneDecision::EnterCooldown { reason, .. }
        | AutotuneDecision::Fault { reason } => reason.clone(),
    }
}

pub(crate) fn rollback_policy_for_decision(decision: &AutotuneDecision) -> &'static str {
    match decision {
        AutotuneDecision::StartExperiment { .. }
        | AutotuneDecision::KeepCurrent { .. }
        | AutotuneDecision::Revert { .. } => "rollback-on-restore",
        AutotuneDecision::Suggest { .. }
        | AutotuneDecision::Noop { .. }
        | AutotuneDecision::EnterCooldown { .. }
        | AutotuneDecision::Fault { .. } => "none",
    }
}

pub(crate) fn decision_is_eligible(decision: &AutotuneDecision) -> bool {
    matches!(
        decision,
        AutotuneDecision::Suggest { .. }
            | AutotuneDecision::StartExperiment { .. }
            | AutotuneDecision::KeepCurrent { .. }
            | AutotuneDecision::Revert { .. }
    )
}

pub(crate) fn decision_label(decision: &AutotuneDecision) -> String {
    match decision {
        AutotuneDecision::Noop { .. } => "observed".to_owned(),
        AutotuneDecision::Suggest { .. } => "suggested".to_owned(),
        AutotuneDecision::StartExperiment { .. } => "candidate_started".to_owned(),
        AutotuneDecision::KeepCurrent { .. } => "candidate_kept".to_owned(),
        AutotuneDecision::Revert { .. } => "candidate_reverted".to_owned(),
        AutotuneDecision::EnterCooldown { .. } => "cooldown_entered".to_owned(),
        AutotuneDecision::Fault { .. } => "faulted".to_owned(),
    }
}

pub(crate) fn decision_candidate_name(decision: &AutotuneDecision) -> Option<String> {
    match decision {
        AutotuneDecision::Suggest { candidate, .. }
        | AutotuneDecision::StartExperiment { candidate, .. } => {
            Some(candidate.profile_name().to_owned())
        }
        AutotuneDecision::Noop { .. }
        | AutotuneDecision::KeepCurrent { .. }
        | AutotuneDecision::Revert { .. }
        | AutotuneDecision::EnterCooldown { .. }
        | AutotuneDecision::Fault { .. } => None,
    }
}

pub(crate) fn decision_action_kind(decision: &AutotuneDecision) -> Option<String> {
    match decision {
        AutotuneDecision::Suggest { candidate, .. }
        | AutotuneDecision::StartExperiment { candidate, .. } => {
            Some(candidate.action_kind().to_owned())
        }
        AutotuneDecision::Noop { .. }
        | AutotuneDecision::KeepCurrent { .. }
        | AutotuneDecision::Revert { .. }
        | AutotuneDecision::EnterCooldown { .. }
        | AutotuneDecision::Fault { .. } => None,
    }
}

pub(crate) fn decision_safety_class(decision: &AutotuneDecision) -> Option<SafetyClass> {
    match decision {
        AutotuneDecision::Suggest { candidate, .. }
        | AutotuneDecision::StartExperiment { candidate, .. } => Some(candidate.safety_class()),
        AutotuneDecision::Noop { .. }
        | AutotuneDecision::KeepCurrent { .. }
        | AutotuneDecision::Revert { .. }
        | AutotuneDecision::EnterCooldown { .. }
        | AutotuneDecision::Fault { .. } => None,
    }
}

pub(crate) fn data_quality_label(quality: &OnlineDataQuality) -> String {
    match quality {
        OnlineDataQuality::High => "High".to_owned(),
        OnlineDataQuality::Medium { reasons } => format!("Medium: {}", reasons.join("; ")),
        OnlineDataQuality::Low { reasons } => format!("Low: {}", reasons.join("; ")),
    }
}
