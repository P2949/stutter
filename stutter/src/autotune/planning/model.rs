//! Planner output models and summaries; this module owns DTO shape, not policy decisions.

use serde::{Deserialize, Serialize};

use crate::{
    actions::{ActionState, SafetyClass},
    autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::{CandidateAction, CandidateEvidence},
            denial::{
                CandidateDenyReason, grouped_denials, names_for_any_reason, names_for_reason,
            },
        },
    },
    daemon_policy::{ActionDescriptor, ActionEffectScope},
};

#[derive(Clone, Debug)]
pub struct CandidateEvaluation {
    pub candidate_name: String,
    pub action_kind: String,
    pub descriptor: ActionDescriptor,
    pub provider: String,
    pub confidence: f32,
    pub eligible: bool,
    pub deny_reasons: Vec<CandidateDenyReason>,
    pub deny_messages: Vec<String>,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
    pub rank: Option<u32>,
    pub dry_run: Option<ActionState>,
    pub candidate: CandidateAction,
}

#[derive(Clone, Debug)]
pub(crate) struct CandidateEvaluationDraft {
    pub(crate) candidate_name: String,
    pub(crate) action_kind: String,
    pub(crate) descriptor: ActionDescriptor,
    pub(crate) provider: String,
    pub(crate) confidence: f32,
    pub(crate) deny_reasons: Vec<CandidateDenyReason>,
    pub(crate) deny_messages: Vec<String>,
    pub(crate) evidence: Vec<CandidateEvidence>,
    pub(crate) objective: ObjectiveKind,
    pub(crate) rank: Option<u32>,
    pub(crate) candidate: CandidateAction,
}

#[derive(Clone, Debug, Default)]
pub struct PlanResult {
    pub selected: Option<CandidateAction>,
    pub evaluations: Vec<CandidateEvaluation>,
    pub no_action_reason: Option<String>,
}

impl PlanResult {
    pub fn summary(&self) -> PlannerSummary {
        PlannerSummary::from_plan(self)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PlannerSummary {
    pub total_proposals: usize,
    pub eligible_proposals: usize,
    pub selected: Option<PlannerSelectedSummary>,
    #[serde(default)]
    pub eligible_candidates: Vec<PlannerEvaluationSummary>,
    pub top_denied_candidates: Vec<PlannerEvaluationSummary>,
    pub grouped_denials: Vec<PlannerDenySummary>,
    pub missing_capabilities: Vec<String>,
    pub workload_blocked: Vec<String>,
    pub manual_only_suggestions: Vec<String>,
    pub no_action: Option<PlannerNoActionSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlannerSelectedSummary {
    pub candidate_name: String,
    pub action_kind: String,
    pub objective: ObjectiveKind,
    pub safety_class: SafetyClass,
    pub confidence: f32,
    pub rank: Option<u32>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlannerEvaluationSummary {
    pub candidate_name: String,
    pub action_kind: String,
    pub provider: String,
    pub objective: ObjectiveKind,
    pub safety_class: SafetyClass,
    pub effect_scope: ActionEffectScope,
    pub confidence: f32,
    pub eligible: bool,
    pub rank: Option<u32>,
    pub deny_reasons: Vec<CandidateDenyReason>,
    pub deny_reason_codes: Vec<String>,
    pub deny_messages: Vec<String>,
    pub dry_run_affected_tasks: Option<usize>,
    pub manual_only_reason: Option<String>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannerDenySummary {
    pub reason: CandidateDenyReason,
    pub reason_code: String,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlannerNoActionSummary {
    pub reason: String,
    pub total_proposals: usize,
    pub eligible_proposals: usize,
    pub grouped_denials: Vec<PlannerDenySummary>,
    #[serde(default)]
    pub top_denied_candidates: Vec<PlannerEvaluationSummary>,
    #[serde(default)]
    pub missing_capabilities: Vec<String>,
    #[serde(default)]
    pub workload_blocked: Vec<String>,
    #[serde(default)]
    pub manual_only_suggestions: Vec<String>,
}

impl PlannerSummary {
    pub fn from_plan(plan: &PlanResult) -> Self {
        let total_proposals = plan.evaluations.len();
        let eligible_proposals = plan
            .evaluations
            .iter()
            .filter(|evaluation| evaluation.eligible)
            .count();
        let selected = plan
            .selected
            .as_ref()
            .and_then(|selected| {
                plan.evaluations
                    .iter()
                    .find(|evaluation| evaluation.candidate.action_id() == selected.action_id())
            })
            .or_else(|| {
                plan.evaluations
                    .iter()
                    .find(|evaluation| evaluation.eligible)
            })
            .map(PlannerSelectedSummary::from_evaluation);
        let grouped_denials = grouped_denials(&plan.evaluations);
        let eligible_candidates = plan
            .evaluations
            .iter()
            .filter(|evaluation| evaluation.eligible)
            .map(PlannerEvaluationSummary::from_evaluation)
            .collect::<Vec<_>>();
        let top_denied_candidates = plan
            .evaluations
            .iter()
            .filter(|evaluation| !evaluation.eligible)
            .take(8)
            .map(PlannerEvaluationSummary::from_evaluation)
            .collect::<Vec<_>>();
        let missing_capabilities =
            names_for_reason(&plan.evaluations, CandidateDenyReason::CapabilityMissing);
        let workload_blocked = names_for_any_reason(
            &plan.evaluations,
            &[
                CandidateDenyReason::WorkloadPolicyBlocked,
                CandidateDenyReason::NotAutonomousForWorkload,
                CandidateDenyReason::ObjectiveNotAllowedForWorkload,
            ],
        );
        let manual_only_suggestions =
            names_for_reason(&plan.evaluations, CandidateDenyReason::ManualOnlyHighRisk);
        let no_action = plan
            .no_action_reason
            .as_ref()
            .map(|reason| PlannerNoActionSummary {
                reason: reason.clone(),
                total_proposals,
                eligible_proposals,
                grouped_denials: grouped_denials.clone(),
                top_denied_candidates: top_denied_candidates.clone(),
                missing_capabilities: missing_capabilities.clone(),
                workload_blocked: workload_blocked.clone(),
                manual_only_suggestions: manual_only_suggestions.clone(),
            });

        Self {
            total_proposals,
            eligible_proposals,
            selected,
            eligible_candidates,
            top_denied_candidates,
            grouped_denials,
            missing_capabilities,
            workload_blocked,
            manual_only_suggestions,
            no_action,
        }
    }
}

impl PlannerSelectedSummary {
    fn from_evaluation(evaluation: &CandidateEvaluation) -> Self {
        Self {
            candidate_name: evaluation.candidate_name.clone(),
            action_kind: evaluation.action_kind.clone(),
            objective: evaluation.objective,
            safety_class: evaluation.descriptor.safety_class.clone(),
            confidence: evaluation.confidence,
            rank: evaluation.rank,
            evidence: planner_evidence_summary(&evaluation.evidence),
        }
    }
}

impl PlannerEvaluationSummary {
    fn from_evaluation(evaluation: &CandidateEvaluation) -> Self {
        Self {
            candidate_name: evaluation.candidate_name.clone(),
            action_kind: evaluation.action_kind.clone(),
            provider: evaluation.provider.clone(),
            objective: evaluation.objective,
            safety_class: evaluation.descriptor.safety_class.clone(),
            effect_scope: evaluation.descriptor.effect_scope,
            confidence: evaluation.confidence,
            eligible: evaluation.eligible,
            rank: evaluation.rank,
            deny_reasons: evaluation.deny_reasons.clone(),
            deny_reason_codes: evaluation
                .deny_reasons
                .iter()
                .map(CandidateDenyReason::reason_code)
                .map(str::to_owned)
                .collect(),
            deny_messages: evaluation.deny_messages.clone(),
            dry_run_affected_tasks: evaluation
                .dry_run
                .as_ref()
                .map(|state| state.affected_tasks),
            manual_only_reason: evaluation.candidate.manual_only_reason(),
            evidence: planner_evidence_summary(&evaluation.evidence),
        }
    }
}

fn planner_evidence_summary(evidence: &[CandidateEvidence]) -> Vec<String> {
    evidence
        .iter()
        .take(8)
        .map(|evidence| {
            format!(
                "{}={} weight={:.2}",
                evidence.signal, evidence.value, evidence.weight
            )
        })
        .collect()
}
