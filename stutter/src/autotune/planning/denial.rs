//! Candidate denial taxonomy and aggregation helpers; this module owns denial labeling, not proposal evaluation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::autotune::planning::model::{CandidateEvaluation, PlannerDenySummary};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDenyReason {
    DisabledFamily,
    DeniedFamily,
    SafetyClassTooHigh,
    EffectScopeTooBroad,
    CapabilityMissing,
    DataQualityLow,
    HealthDegraded,
    NoExplicitTarget,
    FocusLowConfidence,
    CriticalRealtimeWarning,
    CooldownActive,
    ConflictWithActiveAction,
    ConflictWithKeptAction,
    CgroupTargetNotAllowlisted,
    SystemWideTargetNotAllowlisted,
    WorkloadPolicyBlocked,
    NotAutonomousForWorkload,
    ProviderConfidenceTooLow,
    NoEffectiveChange,
    ActiveConfigUnknown,
    ExternalMutationDetected,
    KeptActionNoLongerActive,
    ObjectiveNotAllowedForWorkload,
    ObjectiveSignalMissing,
    TargetSnapshotMissing,
    WorkloadIdle,
    ManualOnlyHighRisk,
    DryRunFailed,
    DryRunMatchedZeroTasks,
    PolicyRejected,
}

impl CandidateDenyReason {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::DisabledFamily => "disabled_family",
            Self::DeniedFamily => "denied_family",
            Self::SafetyClassTooHigh => "safety_class_too_high",
            Self::EffectScopeTooBroad => "effect_scope_too_broad",
            Self::CapabilityMissing => "capability_missing",
            Self::DataQualityLow => "data_quality_low",
            Self::HealthDegraded => "health_degraded",
            Self::NoExplicitTarget => "no_explicit_target",
            Self::FocusLowConfidence => "focus_low_confidence",
            Self::CriticalRealtimeWarning => "critical_realtime_warning",
            Self::CooldownActive => "cooldown_active",
            Self::ConflictWithActiveAction => "conflict_with_active_action",
            Self::ConflictWithKeptAction => "conflict_with_kept_action",
            Self::CgroupTargetNotAllowlisted => "cgroup_target_not_allowlisted",
            Self::SystemWideTargetNotAllowlisted => "system_wide_target_not_allowlisted",
            Self::WorkloadPolicyBlocked => "workload_policy_blocked",
            Self::NotAutonomousForWorkload => "not_autonomous_for_workload",
            Self::ProviderConfidenceTooLow => "provider_confidence_too_low",
            Self::NoEffectiveChange => "no_effective_change",
            Self::ActiveConfigUnknown => "active_config_unknown",
            Self::ExternalMutationDetected => "external_mutation_detected",
            Self::KeptActionNoLongerActive => "kept_action_no_longer_active",
            Self::ObjectiveNotAllowedForWorkload => "objective_not_allowed_for_workload",
            Self::ObjectiveSignalMissing => "objective_signal_missing",
            Self::TargetSnapshotMissing => "target_snapshot_missing",
            Self::WorkloadIdle => "workload_idle",
            Self::ManualOnlyHighRisk => "manual_only_high_risk",
            Self::DryRunFailed => "dry_run_failed",
            Self::DryRunMatchedZeroTasks => "dry_run_matched_zero_tasks",
            Self::PolicyRejected => "policy_rejected",
        }
    }
}

pub(crate) fn grouped_denials(evaluations: &[CandidateEvaluation]) -> Vec<PlannerDenySummary> {
    let mut counts = BTreeMap::<CandidateDenyReason, usize>::new();

    for evaluation in evaluations {
        for reason in &evaluation.deny_reasons {
            *counts.entry(reason.clone()).or_default() += 1;
        }
    }

    counts
        .into_iter()
        .map(|(reason, count)| PlannerDenySummary {
            reason_code: reason.reason_code().to_owned(),
            reason,
            count,
        })
        .collect()
}

pub(crate) fn names_for_reason(
    evaluations: &[CandidateEvaluation],
    reason: CandidateDenyReason,
) -> Vec<String> {
    names_for_any_reason(evaluations, &[reason])
}

pub(crate) fn names_for_any_reason(
    evaluations: &[CandidateEvaluation],
    reasons: &[CandidateDenyReason],
) -> Vec<String> {
    evaluations
        .iter()
        .filter(|evaluation| {
            reasons
                .iter()
                .any(|reason| evaluation.deny_reasons.contains(reason))
        })
        .map(|evaluation| evaluation.candidate_name.clone())
        .collect()
}

pub(crate) fn normalize_evaluation_denials(
    deny_reasons: &mut Vec<CandidateDenyReason>,
    deny_messages: &mut Vec<String>,
) {
    deny_reasons.sort_by_key(|reason| format!("{reason:?}"));
    deny_reasons.dedup();
    deny_messages.sort();
    deny_messages.dedup();
}
