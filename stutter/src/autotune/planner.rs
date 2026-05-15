use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    actions::{ActionState, SafetyClass},
    autotune::{
        active_config::ActiveConfigMatch,
        candidate::{
            CandidateAction, CandidateDryRunner, CandidateEvidence, RealCandidateDryRunner,
        },
        candidate_memory::CandidateContextHashInput,
        controller::ControllerRuntimeState,
        kept::ActiveProfileState,
        objective::ObjectiveKind,
        observation::AutotuneObservation,
        providers::{CandidateProposal, CandidateProviderInput, CandidateProviderRegistry},
        system_context::SystemContextSnapshot,
        workload_policy::WorkloadPolicyMatrix,
    },
    daemon::{DaemonCapabilities, DaemonMode, DaemonPolicy, SystemHealthSnapshot},
    daemon_policy::{ActionDescriptor, DaemonPolicyContext, PolicyIntent},
    profiles::Profile,
};

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
    WorkloadPolicyBlocked,
    NotAutonomousForWorkload,
    ProviderConfidenceTooLow,
    NoEffectiveChange,
    ExternalMutationDetected,
    KeptActionNoLongerActive,
    ObjectiveNotAllowedForWorkload,
    ObjectiveSignalMissing,
    ManualOnlyHighRisk,
    DryRunFailed,
    DryRunMatchedZeroTasks,
    PolicyRejected,
}

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
struct CandidateEvaluationDraft {
    candidate_name: String,
    action_kind: String,
    descriptor: ActionDescriptor,
    provider: String,
    confidence: f32,
    deny_reasons: Vec<CandidateDenyReason>,
    deny_messages: Vec<String>,
    evidence: Vec<CandidateEvidence>,
    objective: ObjectiveKind,
    rank: Option<u32>,
    candidate: CandidateAction,
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
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlannerEvaluationSummary {
    pub candidate_name: String,
    pub action_kind: String,
    pub provider: String,
    pub objective: ObjectiveKind,
    pub safety_class: SafetyClass,
    pub effect_scope: crate::daemon_policy::ActionEffectScope,
    pub confidence: f32,
    pub eligible: bool,
    pub rank: Option<u32>,
    pub deny_reasons: Vec<CandidateDenyReason>,
    pub deny_reason_codes: Vec<String>,
    pub deny_messages: Vec<String>,
    pub dry_run_affected_tasks: Option<usize>,
    pub manual_only_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannerDenySummary {
    pub reason: CandidateDenyReason,
    pub reason_code: String,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlannerNoActionSummary {
    pub reason: String,
    pub total_proposals: usize,
    pub eligible_proposals: usize,
    pub grouped_denials: Vec<PlannerDenySummary>,
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
            });

        Self {
            total_proposals,
            eligible_proposals,
            selected,
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
        }
    }
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
            Self::WorkloadPolicyBlocked => "workload_policy_blocked",
            Self::NotAutonomousForWorkload => "not_autonomous_for_workload",
            Self::ProviderConfidenceTooLow => "provider_confidence_too_low",
            Self::NoEffectiveChange => "no_effective_change",
            Self::ExternalMutationDetected => "external_mutation_detected",
            Self::KeptActionNoLongerActive => "kept_action_no_longer_active",
            Self::ObjectiveNotAllowedForWorkload => "objective_not_allowed_for_workload",
            Self::ObjectiveSignalMissing => "objective_signal_missing",
            Self::ManualOnlyHighRisk => "manual_only_high_risk",
            Self::DryRunFailed => "dry_run_failed",
            Self::DryRunMatchedZeroTasks => "dry_run_matched_zero_tasks",
            Self::PolicyRejected => "policy_rejected",
        }
    }
}

pub struct CandidatePlanner {
    registry: CandidateProviderRegistry,
}

impl CandidatePlanner {
    pub fn new(registry: CandidateProviderRegistry) -> Self {
        Self { registry }
    }

    pub fn default_for_policy(policy: &DaemonPolicy) -> Self {
        Self::new(CandidateProviderRegistry::default_for_policy(policy))
    }

    pub fn plan(&self, input: PlannerInput<'_>) -> PlanResult {
        if input.daemon_policy.mode == crate::daemon::DaemonMode::Observe {
            return PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some("observe mode does not suggest or apply".to_owned()),
            };
        }

        if input.observation.focus_is_idle_or_unknown() {
            return PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some("focus is idle or unknown".to_owned()),
            };
        }

        let fallback_system_context;
        let system_context = if let Some(system_context) = input.observation.system_context.as_ref()
        {
            system_context
        } else {
            fallback_system_context = SystemContextSnapshot::from_observation(input.observation);
            &fallback_system_context
        };

        let provider_input = CandidateProviderInput {
            observation: input.observation,
            daemon_policy: input.daemon_policy,
            capabilities: input.capabilities,
            system_health: input.system_health,
            system_context,
            controller_state: input.controller_state,
            profiles: input.profiles,
        };
        let proposals = self.registry.propose(&provider_input);
        let mut evaluations = evaluate_proposals(input, proposals);

        sort_candidate_evaluations(&mut evaluations);

        let selected = evaluations
            .iter()
            .find(|evaluation| evaluation.eligible)
            .map(|evaluation| evaluation.candidate.clone());

        let no_action_reason = if selected.is_none() {
            Some(no_action_reason_for_evaluations(&evaluations))
        } else {
            None
        };

        PlanResult {
            selected,
            evaluations,
            no_action_reason,
        }
    }
}

fn no_action_reason_for_evaluations(evaluations: &[CandidateEvaluation]) -> String {
    if evaluations.is_empty() {
        return "no candidate providers produced an eligible candidate".to_owned();
    }

    let grouped = grouped_denials(evaluations);
    if grouped.is_empty() {
        return "candidate providers produced no eligible candidate".to_owned();
    }

    let reason_counts = grouped
        .iter()
        .take(4)
        .map(|summary| format!("{}={}", summary.reason_code, summary.count))
        .collect::<Vec<_>>()
        .join(",");
    let top_message = evaluations
        .iter()
        .find(|evaluation| !evaluation.eligible)
        .and_then(|evaluation| evaluation.deny_messages.first())
        .cloned()
        .unwrap_or_else(|| "all candidates were denied".to_owned());

    format!("{top_message}; deny_reason_counts={reason_counts}")
}

fn sort_candidate_evaluations(evaluations: &mut [CandidateEvaluation]) {
    evaluations.sort_by(|left, right| {
        (!left.eligible)
            .cmp(&(!right.eligible))
            .then_with(|| {
                left.rank
                    .unwrap_or(u32::MAX)
                    .cmp(&right.rank.unwrap_or(u32::MAX))
            })
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| left.candidate_name.cmp(&right.candidate_name))
    });
}

fn grouped_denials(evaluations: &[CandidateEvaluation]) -> Vec<PlannerDenySummary> {
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

fn names_for_reason(
    evaluations: &[CandidateEvaluation],
    reason: CandidateDenyReason,
) -> Vec<String> {
    names_for_any_reason(evaluations, &[reason])
}

fn names_for_any_reason(
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

#[derive(Clone, Copy)]
pub struct PlannerInput<'a> {
    pub observation: &'a AutotuneObservation,
    pub daemon_policy: &'a DaemonPolicy,
    pub capabilities: &'a DaemonCapabilities,
    pub system_health: &'a SystemHealthSnapshot,
    pub controller_state: &'a ControllerRuntimeState,
    pub active_profile_state: Option<&'a ActiveProfileState>,
    pub workload_policy: &'a WorkloadPolicyMatrix,
    pub profiles: &'a [Profile],
}

fn evaluate_proposals(
    input: PlannerInput<'_>,
    proposals: Vec<CandidateProposal>,
) -> Vec<CandidateEvaluation> {
    let mut dry_runner = RealCandidateDryRunner;
    evaluate_proposals_with_runner(input, proposals, &mut dry_runner)
}

fn evaluate_proposals_with_runner<R: CandidateDryRunner>(
    input: PlannerInput<'_>,
    proposals: Vec<CandidateProposal>,
    dry_runner: &mut R,
) -> Vec<CandidateEvaluation> {
    proposals
        .into_iter()
        .map(|proposal| {
            let draft = evaluate_proposal_static(input, proposal);
            dry_run_candidate_if_still_eligible(input, draft, dry_runner)
        })
        .collect()
}

fn evaluate_proposal_static(
    input: PlannerInput<'_>,
    proposal: CandidateProposal,
) -> CandidateEvaluationDraft {
    let mut descriptor = proposal.candidate.descriptor();
    descriptor.confidence = Some(proposal.confidence);

    let mut deny_reasons = Vec::new();
    let mut deny_messages = proposal.deny_reasons.clone();

    if !policy_family_enabled(input.daemon_policy, &descriptor.action_kind) {
        deny_reasons.push(CandidateDenyReason::DisabledFamily);
        deny_messages.push(format!(
            "action family is not enabled by daemon policy: {}",
            descriptor.action_kind
        ));
    }

    if policy_family_denied(input.daemon_policy, &descriptor.action_kind) {
        deny_reasons.push(CandidateDenyReason::DeniedFamily);
        deny_messages.push(format!(
            "action family is denied by daemon policy: {}",
            descriptor.action_kind
        ));
    }

    if proposal.candidate.is_high_risk_system_adjacent()
        && input.daemon_policy.mode != DaemonMode::Suggest
    {
        deny_reasons.push(CandidateDenyReason::ManualOnlyHighRisk);
        deny_messages.push(proposal.candidate.manual_only_reason().unwrap_or_else(|| {
            "manual-only high-risk/system-adjacent candidate cannot be selected for live apply"
                .to_owned()
        }));
    }

    let policy_context = policy_context_for_input(input);
    if let Err(rejection) = input.daemon_policy.check_action_with_context(
        policy_intent_for_mode(input.daemon_policy.mode),
        &descriptor,
        &policy_context,
    ) {
        deny_reasons.push(deny_reason_from_policy(rejection.reason_code()));
        deny_messages.push(rejection.to_string());
    }

    if input.observation.focus_confidence < input.daemon_policy.min_confidence {
        deny_reasons.push(CandidateDenyReason::FocusLowConfidence);
        deny_messages.push(format!(
            "focus confidence {:.3} below policy minimum {:.3}",
            input.observation.focus_confidence, input.daemon_policy.min_confidence
        ));
    }

    let provider_confidence_threshold = input
        .daemon_policy
        .provider_confidence_threshold(&descriptor);
    if !proposal.confidence.is_finite() || proposal.confidence < provider_confidence_threshold {
        deny_reasons.push(CandidateDenyReason::ProviderConfidenceTooLow);
        deny_messages.push(format!(
            "provider confidence {:.3} below policy minimum {:.3} for mode {} action_kind={} safety={:?}",
            proposal.confidence,
            provider_confidence_threshold,
            input.daemon_policy.mode,
            descriptor.action_kind,
            descriptor.safety_class
        ));
    }

    if input.observation.focus_has_critical_realtime_warning() {
        deny_reasons.push(CandidateDenyReason::CriticalRealtimeWarning);
        deny_messages.push("critical realtime focus warning blocks mutation".to_owned());
    }

    let workload_policy = input
        .workload_policy
        .rule_for(input.observation.primary_situation);
    if !workload_policy.allows_candidate(&proposal.candidate) {
        deny_reasons.push(CandidateDenyReason::WorkloadPolicyBlocked);
        deny_messages.push(format!(
            "workload policy for {:?} does not allow action family {}",
            input.observation.primary_situation,
            proposal.candidate.action_kind()
        ));
    }

    if mode_requires_autonomous_workload_family(input.daemon_policy.mode)
        && !workload_policy.allows_autonomous_candidate(&proposal.candidate)
    {
        deny_reasons.push(CandidateDenyReason::NotAutonomousForWorkload);
        deny_messages.push(format!(
            "workload policy for {:?} does not allow autonomous apply for action family {}; autonomous_families={:?}",
            input.observation.primary_situation,
            proposal.candidate.action_kind(),
            workload_policy.autonomous_families
        ));
    }

    if !workload_policy.allows_objective(proposal.objective) {
        deny_reasons.push(CandidateDenyReason::ObjectiveNotAllowedForWorkload);
        deny_messages.push(format!(
            "workload policy for {:?} does not allow objective {:?}",
            input.observation.primary_situation, proposal.objective
        ));
    }

    let memory_context =
        CandidateContextHashInput::from_observation(&proposal.candidate, input.observation, None);

    if memory_context.workload_hash.is_none()
        && input
            .controller_state
            .candidate_memory
            .cooldown_remaining_for_action(
                &proposal.candidate.action_id(),
                input.observation.now_unix_nanos,
            )
            .is_some()
    {
        deny_reasons.push(CandidateDenyReason::CooldownActive);
        deny_messages.push("candidate action is cooling down".to_owned());
    }

    if input
        .controller_state
        .candidate_memory
        .cooldown_remaining_for_workload_action(
            &proposal.candidate,
            &memory_context,
            input.observation.now_unix_nanos,
        )
        .is_some()
    {
        deny_reasons.push(CandidateDenyReason::CooldownActive);
        deny_messages
            .push("candidate action is cooling down for this workload identity".to_owned());
    }

    if let Some(active_experiment) = input.controller_state.active_experiment.as_ref()
        && proposal
            .candidate
            .conflicts_with(&active_experiment.candidate)
    {
        deny_reasons.push(CandidateDenyReason::ConflictWithActiveAction);
        deny_messages.push(format!(
            "candidate conflict group {:?} conflicts with active experiment {} ({:?})",
            proposal.candidate.conflict_group(),
            active_experiment.candidate.candidate_name(),
            active_experiment.candidate.conflict_group()
        ));
    }

    if let Some(kept) = input
        .active_profile_state
        .and_then(|state| state.current.as_ref())
        && proposal.candidate.conflicts_with(&kept.candidate)
    {
        deny_reasons.push(CandidateDenyReason::ConflictWithKeptAction);
        deny_messages.push(format!(
            "candidate conflict group {:?} conflicts with kept action {} ({:?})",
            proposal.candidate.conflict_group(),
            kept.candidate.candidate_name(),
            kept.candidate.conflict_group()
        ));
    }

    if let Some(cgroup_target) = proposal.candidate.cgroup_target_path()
        && !input
            .daemon_policy
            .cgroup_targets
            .contains_path(cgroup_target)
    {
        deny_reasons.push(CandidateDenyReason::CgroupTargetNotAllowlisted);
        deny_messages.push(format!(
            "cgroup target {} is not allowlisted by daemon policy",
            cgroup_target.display()
        ));
    }

    if let Some(snapshot) = input.observation.active_config_snapshot.as_ref() {
        if let ActiveConfigMatch::Matches { summary } =
            proposal.candidate.matches_active_config(snapshot)
        {
            deny_reasons.push(CandidateDenyReason::NoEffectiveChange);
            deny_messages.push(format!(
                "candidate would not change active configuration: {summary}"
            ));
        }

        if let Some(active_experiment) = input.controller_state.active_experiment.as_ref()
            && proposal
                .candidate
                .conflicts_with(&active_experiment.candidate)
            && let ActiveConfigMatch::Differs { expected, actual } =
                active_experiment.candidate.matches_active_config(snapshot)
        {
            deny_reasons.push(CandidateDenyReason::ExternalMutationDetected);
            deny_messages.push(format!(
                "active experiment no longer matches live state for conflict group {:?}; expected {}; actual {}; restore or resync before applying another conflicting action",
                active_experiment.candidate.conflict_group(),
                expected,
                actual
            ));
        }

        if let Some(kept) = input
            .active_profile_state
            .and_then(|state| state.current.as_ref())
            && let ActiveConfigMatch::Differs { expected, actual } =
                kept.candidate.matches_active_config(snapshot)
        {
            deny_reasons.push(CandidateDenyReason::KeptActionNoLongerActive);
            deny_messages.push(format!(
                "kept action {} no longer matches live state; expected {}; actual {}; restore or resync before planning new candidates",
                kept.candidate.candidate_name(),
                expected,
                actual
            ));
        }
    }

    normalize_evaluation_denials(&mut deny_reasons, &mut deny_messages);

    let candidate_name = proposal.candidate.candidate_name().to_owned();
    let action_kind = proposal.candidate.action_kind().to_owned();
    let evidence = proposal.candidate.evidence().to_vec();

    CandidateEvaluationDraft {
        candidate_name,
        action_kind,
        descriptor,
        provider: proposal.provider.to_owned(),
        confidence: proposal.confidence,
        deny_reasons,
        deny_messages,
        evidence,
        objective: proposal.objective,
        rank: Some(rank_with_workload_memory(
            proposal.rank_hint,
            input
                .controller_state
                .candidate_memory
                .workload_rank_hint(&proposal.candidate, &memory_context),
        )),
        candidate: proposal.candidate,
    }
}

fn rank_with_workload_memory(rank_hint: u32, memory_adjustment: Option<i32>) -> u32 {
    let adjusted = rank_hint as i64 + i64::from(memory_adjustment.unwrap_or(0));
    adjusted.clamp(0, u32::MAX as i64) as u32
}

fn dry_run_candidate_if_still_eligible<R: CandidateDryRunner>(
    input: PlannerInput<'_>,
    draft: CandidateEvaluationDraft,
    dry_runner: &mut R,
) -> CandidateEvaluation {
    let mut deny_reasons = draft.deny_reasons;
    let mut deny_messages = draft.deny_messages;
    let mut dry_run_state = None;

    if deny_reasons.is_empty()
        && !(input.daemon_policy.mode == DaemonMode::Suggest
            && draft.candidate.is_high_risk_system_adjacent())
    {
        let record = dry_runner.dry_run(&draft.candidate);
        dry_run_state = Some(ActionState {
            applied: false,
            affected_tasks: record.affected_tasks,
            checked_tasks: record.affected_tasks,
            pending_changes: record.affected_tasks,
            warnings: record.warnings.clone(),
        });

        if !record.eligible {
            deny_reasons.push(if record.affected_tasks == 0 {
                CandidateDenyReason::DryRunMatchedZeroTasks
            } else {
                CandidateDenyReason::DryRunFailed
            });
            if let Some(reason) = &record.reason {
                deny_messages.push(reason.clone());
            }
        }

        if record.safety_class > input.daemon_policy.max_safety_class {
            deny_reasons.push(CandidateDenyReason::SafetyClassTooHigh);
            deny_messages.push(format!(
                "candidate safety {:?} exceeds mode maximum {:?}",
                record.safety_class, input.daemon_policy.max_safety_class
            ));
        }
    }

    normalize_evaluation_denials(&mut deny_reasons, &mut deny_messages);

    CandidateEvaluation {
        candidate_name: draft.candidate_name,
        action_kind: draft.action_kind,
        descriptor: draft.descriptor,
        provider: draft.provider,
        confidence: draft.confidence,
        eligible: deny_reasons.is_empty(),
        deny_reasons,
        deny_messages,
        evidence: draft.evidence,
        objective: draft.objective,
        rank: draft.rank,
        dry_run: dry_run_state,
        candidate: draft.candidate,
    }
}

fn normalize_evaluation_denials(
    deny_reasons: &mut Vec<CandidateDenyReason>,
    deny_messages: &mut Vec<String>,
) {
    deny_reasons.sort_by_key(|reason| format!("{reason:?}"));
    deny_reasons.dedup();
    deny_messages.sort();
    deny_messages.dedup();
}

fn policy_intent_for_mode(mode: DaemonMode) -> PolicyIntent {
    if mode.supports_apply() {
        PolicyIntent::Apply
    } else {
        PolicyIntent::Suggest
    }
}

fn policy_family_enabled(policy: &DaemonPolicy, action_kind: &str) -> bool {
    policy.enabled_action_families.is_empty()
        || policy
            .enabled_action_families
            .iter()
            .any(|family| policy_family_matches(action_kind, family))
}

fn policy_family_denied(policy: &DaemonPolicy, action_kind: &str) -> bool {
    policy
        .denied_action_families
        .iter()
        .any(|family| policy_family_matches(action_kind, family))
}

fn policy_family_matches(action_kind: &str, family: &str) -> bool {
    action_kind == family
        || action_kind.strip_prefix(family).is_some_and(|suffix| {
            matches!(
                suffix.as_bytes().first(),
                Some(b':') | Some(b'-') | Some(b'_')
            )
        })
}

fn mode_requires_autonomous_workload_family(mode: DaemonMode) -> bool {
    matches!(
        mode,
        DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk | DaemonMode::ApplyHighRisk
    )
}

fn policy_context_for_input(input: PlannerInput<'_>) -> DaemonPolicyContext {
    DaemonPolicyContext {
        data_quality_ok: !input.observation.data_quality.blocks_action(),
        data_quality_reason_code: input
            .observation
            .data_quality
            .reason_code_strings()
            .first()
            .cloned(),
        system_health_ok: input.system_health.ok_for_apply,
        system_health_reason_code: input.system_health.reason_code.clone(),
        workload_stable: input.observation.workload_identity.is_some(),
        cooldown_active: input
            .controller_state
            .cooldown_until_unix_nanos
            .is_some_and(|until| until > input.observation.now_unix_nanos),
        rollback_pending: input.controller_state.active_experiment.is_some(),
        capabilities: Some(input.capabilities.clone()),
    }
}

fn deny_reason_from_policy(reason_code: &str) -> CandidateDenyReason {
    match reason_code {
        "action_family_not_enabled" => CandidateDenyReason::DisabledFamily,
        "action_family_denied" => CandidateDenyReason::DeniedFamily,
        "safety_class_too_high" => CandidateDenyReason::SafetyClassTooHigh,
        "effect_scope_not_allowed" | "system_wide_action_blocked" => {
            CandidateDenyReason::EffectScopeTooBroad
        }
        "capability_unavailable" => CandidateDenyReason::CapabilityMissing,
        "data_quality_blocked" => CandidateDenyReason::DataQualityLow,
        "system_health_blocked" => CandidateDenyReason::HealthDegraded,
        "explicit_target_required" => CandidateDenyReason::NoExplicitTarget,
        "confidence_too_low" => CandidateDenyReason::ProviderConfidenceTooLow,
        "cooldown_active" => CandidateDenyReason::CooldownActive,
        "high_risk_apply_not_implemented" | "medium_risk_apply_requires_explicit_unlock" => {
            CandidateDenyReason::ManualOnlyHighRisk
        }
        _ => CandidateDenyReason::PolicyRejected,
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::{
        actions::{
            RollbackToken, SafetyClass, TaskIdentity,
            cgroup::{CgroupPlacementAction, CgroupPlacementTarget},
            cpu_power::CpuPowerAction,
            gpu_power::GpuPowerAction,
            ioprio::{IoPrioAction, IoPrioPolicy, IoPrioValue},
            irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
            nice::{NiceAction, NicePolicy},
            uclamp::{UclampAction, UclampValues},
            vm_knobs::{VmKnobAction, VmKnobChange},
        },
        affinity::CpuMask,
        autotune::{
            candidate::{
                CandidateDryRunRecord, CgroupPlacementActionPlan, CpuPowerActionPlan,
                IoPrioActionPlan, IrqAffinityActionPlan, NiceActionPlan, UclampActionPlan,
            },
            candidate_memory::CandidateMemoryResult,
            controller::ActiveExperiment,
            experiment::{ExperimentId, WindowScore},
            kept::{ActiveProfileState, KeptCandidateState},
            observation::{
                ActiveConfigSnapshot, ActiveNiceSnapshot, ActiveTaskSnapshot, AutotuneObservation,
                WorkloadIdentity,
            },
            providers::{CandidateProvider, CandidateProviderInput, CandidateProviderRegistry},
            quality::OnlineDataQuality,
            state::SituationKind,
        },
        daemon::{ActionSource, DaemonMode},
        daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
        focus::FocusGroupKind,
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
        scorer::StutterScore,
    };

    fn policy(mode: DaemonMode) -> DaemonPolicy {
        let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            mode,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        config.safety.allow_system_wide_suggestions = true;
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    fn rollback_token() -> RollbackToken {
        RollbackToken::CpuAffinityRestoreFile {
            path: std::path::PathBuf::from("/tmp/stutter-restore.json"),
            affected_tasks: 1,
        }
    }

    fn active_nice_snapshot(tid: u32, nice: i32) -> ActiveConfigSnapshot {
        ActiveConfigSnapshot {
            nice: ActiveNiceSnapshot {
                per_tid: std::collections::BTreeMap::from([(tid, nice)]),
            },
            ..ActiveConfigSnapshot::default()
        }
    }

    fn observation_with_active_nice(
        situation: SituationKind,
        focus_kind: FocusGroupKind,
        tid: u32,
        nice: i32,
    ) -> AutotuneObservation {
        let mut observation = observation_for_situation(situation, focus_kind);
        observation.active_config_snapshot = Some(active_nice_snapshot(tid, nice));
        observation
    }

    fn suggest_policy_with_compile_cgroup() -> DaemonPolicy {
        let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        config.safety.cgroup_targets.compile_cgroup = Some(std::path::PathBuf::from(
            "/user.slice/stutter-compile.slice",
        ));
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }

    fn observation() -> AutotuneObservation {
        let mut observation = AutotuneObservation {
            now_unix_nanos: 1_000,
            target_present: true,
            target_root_pid: Some(1234),
            active_target_count: 1,
            scored_task_count: 1,
            interval_count: 3,
            scored_samples: 100,
            score: StutterScore {
                total: 100,
                over_1ms: 10,
                ..StutterScore::default()
            },
            data_quality: OnlineDataQuality::High,
            primary_situation: SituationKind::GameCpuSchedulerPressure,
            focus_kind: Some(FocusGroupKind::Game),
            focus_confidence: 0.95,
            focus_roots: vec![1234],
            workload_identity: Some(WorkloadIdentity {
                root_pid: 1234,
                process_starttime_ticks: Some(10),
                exe_dev: Some(1),
                exe_ino: Some(2),
                cgroup_path: Some("/user.slice/app.scope".to_owned()),
                focus_kind: Some(FocusGroupKind::Game),
                class_distribution: std::collections::BTreeMap::from([("game".to_owned(), 1)]),
                stable_hash: "test-workload".to_owned(),
            }),
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation
    }

    #[derive(Clone)]
    struct StaticProvider {
        candidate: CandidateAction,
    }

    impl CandidateProvider for StaticProvider {
        fn family(&self) -> &'static str {
            self.candidate.action_kind()
        }

        fn propose(&self, _input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
            vec![CandidateProposal {
                candidate: self.candidate.clone(),
                provider: self.family(),
                confidence: 1.0,
                deny_reasons: Vec::new(),
                objective: self.candidate.objective(),
                rank_hint: 1,
            }]
        }
    }

    fn cpu_affinity_candidate(name: &str) -> CandidateAction {
        CandidateAction::cpu_affinity_profile(
            Profile {
                name: name.to_owned(),
                rules: vec![ProfileRule {
                    affinity: Some(CpuMask::parse("0").unwrap()),
                    nice: None,
                    ionice: None,
                    match_class: vec![TaskClass::Game],
                    match_comm: Vec::new(),
                }],
            },
            1234,
        )
    }

    fn cgroup_candidate(target_cgroup: &str) -> CandidateAction {
        CandidateAction::CgroupPlacement {
            plan: CgroupPlacementActionPlan {
                name: format!("cgroup-to-{target_cgroup}"),
                action: CgroupPlacementAction {
                    cgroup_root: std::path::PathBuf::from("/sys/fs/cgroup"),
                    target_cgroup: std::path::PathBuf::from(target_cgroup),
                    targets: vec![CgroupPlacementTarget {
                        identity: TaskIdentity {
                            tid: 1234,
                            process_pid: Some(1234),
                            comm: Some("rustc".to_owned()),
                            starttime_ticks: None,
                        },
                        class: TaskClass::Compiler,
                    }],
                    cpuset_cpus: None,
                    cpuset_mems: None,
                },
                target_root_pid: Some(1234),
                evidence: Vec::new(),
                objective: ObjectiveKind::CompileThroughputWithForegroundProtection,
            },
        }
    }

    fn nice_candidate() -> CandidateAction {
        CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "nice-background-compile".to_owned(),
                action: NiceAction {
                    targets: vec![TaskIdentity {
                        tid: 1234,
                        process_pid: Some(1234),
                        comm: Some("rustc".to_owned()),
                        starttime_ticks: None,
                    }],
                    nice: 5,
                    policy: NicePolicy::default(),
                },
                target_root_pid: Some(1234),
                evidence: Vec::new(),
                objective: ObjectiveKind::DesktopInteractivity,
            },
        }
    }

    fn task_identity() -> TaskIdentity {
        TaskIdentity {
            tid: 1234,
            process_pid: Some(1234),
            comm: Some("stutter-test".to_owned()),
            starttime_ticks: None,
        }
    }

    fn uclamp_candidate(name: &str) -> CandidateAction {
        CandidateAction::Uclamp {
            plan: UclampActionPlan {
                name: name.to_owned(),
                action: UclampAction {
                    targets: vec![task_identity()],
                    values: UclampValues {
                        sched_util_min: Some(128),
                        sched_util_max: Some(1024),
                    },
                },
                target_root_pid: Some(1234),
                evidence: Vec::new(),
                objective: ObjectiveKind::DesktopInteractivity,
            },
        }
    }

    fn ioprio_candidate(name: &str) -> CandidateAction {
        CandidateAction::IoPrio {
            plan: IoPrioActionPlan {
                name: name.to_owned(),
                action: IoPrioAction {
                    targets: vec![task_identity()],
                    ioprio: IoPrioValue::idle(),
                    policy: IoPrioPolicy {
                        allow_ioprio_changes: true,
                        require_strong_block_io_evidence: false,
                        strong_block_io_evidence: true,
                        ..IoPrioPolicy::default()
                    },
                },
                target_root_pid: Some(1234),
                evidence: Vec::new(),
                objective: ObjectiveKind::IoLatency,
            },
        }
    }

    fn irq_affinity_candidate(name: &str) -> CandidateAction {
        let evidence = IrqAffinityEvidence {
            strong_irq_evidence: true,
            stable_irq_identity: true,
            known_device_mapping: true,
            observed_irq: Some(42),
            observed_device_hint: Some("test-device".to_owned()),
            reason: "test IRQ evidence".to_owned(),
        };

        CandidateAction::IrqAffinity {
            plan: IrqAffinityActionPlan {
                name: name.to_owned(),
                action: IrqAffinityAction::new(
                    42,
                    "test-device".to_owned(),
                    "1".to_owned(),
                    IrqAffinityRisk::ReversibleMediumRisk,
                    evidence,
                ),
                evidence: Vec::new(),
                objective: ObjectiveKind::IrqOverlapReduction,
            },
        }
    }

    fn cpu_power_candidate(name: &str) -> CandidateAction {
        CandidateAction::CpuPower {
            plan: CpuPowerActionPlan {
                name: name.to_owned(),
                action: CpuPowerAction {
                    sysfs_root: std::path::PathBuf::from("/sys"),
                    cpus: vec![0],
                    scaling_governor: Some("performance".to_owned()),
                    energy_performance_preference: Some("performance".to_owned()),
                },
                evidence: Vec::new(),
                objective: ObjectiveKind::GameRunnableLatency,
            },
        }
    }

    fn gpu_power_candidate(name: &str) -> CandidateAction {
        CandidateAction::GpuPower {
            plan: crate::autotune::candidate::GpuPowerActionPlan {
                name: name.to_owned(),
                action: GpuPowerAction {
                    sysfs_root: std::path::PathBuf::from("/sys"),
                    drm_card: "card0".to_owned(),
                    power_dpm_force_performance_level: Some("high".to_owned()),
                    pp_power_profile_mode: None,
                },
                evidence: vec![CandidateEvidence::new("gpu_bound", "true", 0.9)],
                objective: ObjectiveKind::ThermalRecovery,
            },
        }
    }

    fn vm_knob_candidate(name: &str) -> CandidateAction {
        CandidateAction::VmKnob {
            plan: crate::autotune::candidate::VmKnobActionPlan {
                name: name.to_owned(),
                action: VmKnobAction {
                    root: std::path::PathBuf::from("/proc/sys"),
                    changes: vec![VmKnobChange {
                        path: std::path::PathBuf::from("vm/swappiness"),
                        value: "10".to_owned(),
                    }],
                },
                evidence: vec![CandidateEvidence::new("swap_activity", "high", 0.9)],
                objective: ObjectiveKind::IoLatency,
            },
        }
    }

    fn candidate_for_action_kind(action_kind: &str) -> CandidateAction {
        match action_kind {
            "cpu_affinity_profile" => cpu_affinity_candidate("fixture-cpu-affinity"),
            "nice" => nice_candidate(),
            "ionice" => ioprio_candidate("fixture-ionice"),
            "uclamp" => uclamp_candidate("fixture-uclamp"),
            "cgroup_placement" => cgroup_candidate("/user.slice/stutter-compile.slice"),
            "irq_affinity" => irq_affinity_candidate("fixture-irq"),
            "cpu_power" => cpu_power_candidate("fixture-cpu-power"),
            "gpu_power" => gpu_power_candidate("fixture-gpu-power"),
            "vm_knob" => vm_knob_candidate("fixture-vm-knob"),
            other => panic!("unsupported fixture action_kind {other}"),
        }
    }

    fn observation_for_situation(
        situation: SituationKind,
        focus_kind: FocusGroupKind,
    ) -> AutotuneObservation {
        let mut observation = observation();
        observation.primary_situation = situation;
        observation.focus_kind = Some(focus_kind);
        observation
    }

    fn enable_capability_for_candidate(
        observation: &mut AutotuneObservation,
        candidate: &CandidateAction,
    ) {
        match candidate.action_kind() {
            "uclamp" => observation.capabilities.uclamp_available = true,
            "ionice" => observation.capabilities.ionice_available = true,
            "irq_affinity" => observation.capabilities.irq_affinity_available = true,
            "gpu_power" => observation.capabilities.gpu_sysfs_available = true,
            _ => {}
        }
    }

    fn eligible_dry_run(candidate: &CandidateAction) -> CandidateDryRunRecord {
        CandidateDryRunRecord {
            candidate_name: candidate.candidate_name().to_owned(),
            affected_tasks: 1,
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        }
    }

    #[derive(Default)]
    struct CountingDryRunner {
        calls: usize,
    }

    impl CandidateDryRunner for CountingDryRunner {
        fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord {
            self.calls += 1;
            eligible_dry_run(candidate)
        }
    }

    fn proposal_for_candidate(candidate: CandidateAction, confidence: f32) -> CandidateProposal {
        CandidateProposal {
            candidate: candidate.clone(),
            provider: candidate.action_kind(),
            confidence,
            deny_reasons: Vec::new(),
            objective: candidate.objective(),
            rank_hint: 1,
        }
    }

    fn evaluate_candidate_with_runner(
        policy: &DaemonPolicy,
        observation: &AutotuneObservation,
        capabilities: &crate::daemon::DaemonCapabilities,
        controller_state: &ControllerRuntimeState,
        candidate: CandidateAction,
        confidence: f32,
        dry_runner: &mut CountingDryRunner,
    ) -> CandidateEvaluation {
        let proposals = vec![proposal_for_candidate(candidate, confidence)];
        let mut evaluations = evaluate_proposals_with_runner(
            PlannerInput {
                observation,
                daemon_policy: policy,
                capabilities,
                system_health: &observation.system_health,
                controller_state,
                active_profile_state: None,
                workload_policy: &WorkloadPolicyMatrix::default_rules(),
                profiles: &[],
            },
            proposals,
            dry_runner,
        );

        assert_eq!(evaluations.len(), 1);
        evaluations.remove(0)
    }

    fn evaluate_static_candidate(
        mode: DaemonMode,
        situation: SituationKind,
        focus_kind: FocusGroupKind,
        candidate: CandidateAction,
    ) -> CandidateEvaluation {
        evaluate_static_candidate_with_confidence(mode, situation, focus_kind, candidate, 1.0)
    }

    fn evaluate_static_candidate_with_confidence(
        mode: DaemonMode,
        situation: SituationKind,
        focus_kind: FocusGroupKind,
        candidate: CandidateAction,
        confidence: f32,
    ) -> CandidateEvaluation {
        let policy = policy(mode);
        let mut observation = observation_for_situation(situation, focus_kind);
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            confidence,
            &mut dry_runner,
        )
    }

    fn assert_autonomous_denial_mentions_context(
        evaluation: &CandidateEvaluation,
        situation: SituationKind,
        action_kind: &str,
    ) {
        assert!(!evaluation.eligible);
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::NotAutonomousForWorkload)
        );
        assert!(
            evaluation.deny_messages.iter().any(|message| {
                message.contains(&format!("{situation:?}"))
                    && message.contains(action_kind)
                    && message.contains("autonomous_families=")
            }),
            "missing autonomous denial context in {:?}",
            evaluation.deny_messages
        );
    }

    fn window_score(total: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 1,
            finished_unix_nanos: 2,
            interval_count: 5,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total,
                ..StutterScore::default()
            },
        }
    }

    #[test]
    fn low_confidence_suggest_candidate_is_denied_by_provider_threshold() {
        let evaluation = evaluate_static_candidate_with_confidence(
            DaemonMode::Suggest,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            cpu_affinity_candidate("low-confidence-suggest"),
            0.49,
        );

        assert_eq!(evaluation.confidence, 0.49);
        assert_eq!(evaluation.descriptor.confidence, Some(0.49));
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ProviderConfidenceTooLow)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ObjectiveNotAllowedForWorkload)
        );
    }

    #[test]
    fn low_confidence_high_risk_suggestion_is_denied_even_when_workload_allows_it() {
        let evaluation = evaluate_static_candidate_with_confidence(
            DaemonMode::Suggest,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            cpu_power_candidate("low-confidence-cpu-power"),
            0.89,
        );

        assert_eq!(evaluation.confidence, 0.89);
        assert_eq!(evaluation.descriptor.confidence, Some(0.89));
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ProviderConfidenceTooLow)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ObjectiveNotAllowedForWorkload)
        );
    }

    #[test]
    fn low_confidence_medium_risk_candidate_is_denied_even_when_family_and_objective_allowed() {
        let evaluation = evaluate_static_candidate_with_confidence(
            DaemonMode::ApplyMediumRisk,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            uclamp_candidate("low-confidence-uclamp"),
            0.84,
        );

        assert_eq!(evaluation.confidence, 0.84);
        assert_eq!(evaluation.descriptor.confidence, Some(0.84));
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ProviderConfidenceTooLow)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ObjectiveNotAllowedForWorkload)
        );
    }

    #[test]
    fn equal_rank_candidates_sort_by_higher_confidence_before_candidate_name() {
        let low_confidence = cpu_affinity_candidate("aaa-low-confidence");
        let high_confidence = cpu_affinity_candidate("zzz-high-confidence");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation();
        let mut dry_runner = CountingDryRunner::default();
        let proposals = vec![
            CandidateProposal {
                candidate: low_confidence.clone(),
                provider: low_confidence.action_kind(),
                confidence: 0.80,
                deny_reasons: Vec::new(),
                objective: low_confidence.objective(),
                rank_hint: 1,
            },
            CandidateProposal {
                candidate: high_confidence.clone(),
                provider: high_confidence.action_kind(),
                confidence: 0.90,
                deny_reasons: Vec::new(),
                objective: high_confidence.objective(),
                rank_hint: 1,
            },
        ];

        let mut evaluations = evaluate_proposals_with_runner(
            PlannerInput {
                observation: &observation,
                daemon_policy: &policy,
                capabilities: &observation.capabilities,
                system_health: &observation.system_health,
                controller_state: &ControllerRuntimeState::default(),
                active_profile_state: None,
                workload_policy: &WorkloadPolicyMatrix::default_rules(),
                profiles: &[],
            },
            proposals,
            &mut dry_runner,
        );
        sort_candidate_evaluations(&mut evaluations);

        assert_eq!(dry_runner.calls, 2);
        assert_eq!(evaluations[0].candidate_name, "zzz-high-confidence");
        assert_eq!(evaluations[0].confidence, 0.90);
        assert_eq!(evaluations[1].candidate_name, "aaa-low-confidence");
        assert_eq!(evaluations[1].confidence, 0.80);
    }

    #[test]
    fn no_effective_change_candidate_is_denied_before_dry_run() {
        let candidate = nice_candidate();
        let policy = policy(DaemonMode::Suggest);
        let observation = observation_with_active_nice(
            SituationKind::CompileCpuBound,
            FocusGroupKind::Compile,
            1234,
            5,
        );
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::NoEffectiveChange)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("would not change active configuration"))
        );
    }

    #[test]
    fn active_experiment_external_mutation_blocks_conflicting_candidate_before_dry_run() {
        let active_candidate = nice_candidate();
        let candidate = uclamp_candidate("external-mutation-uclamp");
        let policy = policy(DaemonMode::Suggest);
        let mut observation = observation_with_active_nice(
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            1234,
            0,
        );
        enable_capability_for_candidate(&mut observation, &candidate);

        let controller_state = ControllerRuntimeState {
            active_experiment: Some(ActiveExperiment {
                experiment_id: ExperimentId::new("external-mutation"),
                candidate: active_candidate,
                baseline_score_total: 1_000,
            }),
            ..ControllerRuntimeState::default()
        };
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &controller_state,
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ExternalMutationDetected)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("restore or resync"))
        );
    }

    #[test]
    fn kept_action_no_longer_active_blocks_new_candidates_before_dry_run() {
        let kept_candidate = nice_candidate();
        let candidate = cpu_affinity_candidate("kept-no-longer-active-cpu-placement");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation_with_active_nice(
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            1234,
            0,
        );
        let active_profile_state = ActiveProfileState {
            current: Some(KeptCandidateState::new(
                ExperimentId::new("kept-no-longer-active"),
                kept_candidate,
                window_score(1_000),
                window_score(800),
                rollback_token(),
                observation.now_unix_nanos,
                "kept for test",
            )),
            history: Vec::new(),
        };
        let mut dry_runner = CountingDryRunner::default();
        let proposals = vec![proposal_for_candidate(candidate, 1.0)];

        let mut evaluations = evaluate_proposals_with_runner(
            PlannerInput {
                observation: &observation,
                daemon_policy: &policy,
                capabilities: &observation.capabilities,
                system_health: &observation.system_health,
                controller_state: &ControllerRuntimeState::default(),
                active_profile_state: Some(&active_profile_state),
                workload_policy: &WorkloadPolicyMatrix::default_rules(),
                profiles: &[],
            },
            proposals,
            &mut dry_runner,
        );

        assert_eq!(evaluations.len(), 1);
        let evaluation = evaluations.remove(0);
        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::KeptActionNoLongerActive)
        );
        assert!(
            evaluation
                .deny_messages
                .iter()
                .any(|message| message.contains("restore or resync"))
        );
    }

    #[test]
    fn policy_denied_candidate_does_not_call_dry_run() {
        let candidate = cpu_affinity_candidate("policy-denied-no-dry-run");
        let mut policy = policy(DaemonMode::Suggest);
        policy
            .denied_action_families
            .insert("cpu_affinity_profile".to_owned());
        let observation = observation();
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::DeniedFamily)
        );
    }

    #[test]
    fn low_confidence_candidate_does_not_call_dry_run() {
        let candidate = cpu_affinity_candidate("low-confidence-no-dry-run");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation();
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            0.49,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::ProviderConfidenceTooLow)
        );
    }

    #[test]
    fn non_autonomous_apply_candidate_does_not_call_dry_run() {
        let candidate = uclamp_candidate("non-autonomous-no-dry-run");
        let policy = policy(DaemonMode::ApplyMediumRisk);
        let mut observation = observation_for_situation(
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
        );
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::NotAutonomousForWorkload)
        );
    }

    #[test]
    fn cooldown_blocked_candidate_does_not_call_dry_run() {
        let candidate = cpu_affinity_candidate("cooldown-no-dry-run");
        let policy = policy(DaemonMode::Suggest);
        let observation = observation();
        let mut controller_state = ControllerRuntimeState::default();
        controller_state.record_candidate_result(
            &candidate,
            &observation,
            None,
            CandidateMemoryResult::Tried,
            None,
            None,
            None,
            Some(observation.now_unix_nanos + 1_000_000_000),
        );
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &observation.capabilities,
            &controller_state,
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::CooldownActive)
        );
    }

    #[test]
    fn cgroup_not_allowlisted_candidate_does_not_call_dry_run() {
        let candidate = cgroup_candidate("/user.slice/not-allowlisted.slice");
        let policy = policy(DaemonMode::Suggest);
        let mut observation =
            observation_for_situation(SituationKind::CompileCpuBound, FocusGroupKind::Compile);
        enable_capability_for_candidate(&mut observation, &candidate);
        let mut capabilities = observation.capabilities.clone();
        capabilities.cgroup_v2_available = true;
        let mut dry_runner = CountingDryRunner::default();

        let evaluation = evaluate_candidate_with_runner(
            &policy,
            &observation,
            &capabilities,
            &ControllerRuntimeState::default(),
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert_eq!(dry_runner.calls, 0);
        assert!(evaluation.dry_run.is_none());
        assert!(
            evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::CgroupTargetNotAllowlisted)
        );
    }

    #[test]
    fn observe_mode_returns_no_suggestions() {
        let policy = policy(DaemonMode::Observe);
        let planner = CandidatePlanner::default_for_policy(&policy);
        let observation = observation();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(result.selected.is_none());
        assert!(result.evaluations.is_empty());
    }

    #[test]
    fn low_risk_policy_denies_medium_risk_fake_candidate() {
        let policy = policy(DaemonMode::ApplyLowRisk);
        let candidate = CandidateAction::Fake {
            action_id: crate::actions::ActionId("fake-medium".to_owned()),
            safety_class: SafetyClass::ReversibleMediumRisk,
        };

        assert!(candidate.safety_class() > policy.max_safety_class);
    }

    #[test]
    fn cgroup_provider_candidate_is_denied_when_cgroup_v2_capability_missing() {
        let policy = suggest_policy_with_compile_cgroup();
        let planner = CandidatePlanner::default_for_policy(&policy);
        let mut observation = observation();
        observation.primary_situation = SituationKind::CompileCpuBound;
        observation.focus_kind = Some(FocusGroupKind::Compile);
        observation.active_tasks = vec![ActiveTaskSnapshot {
            tid: 1234,
            process_pid: 1234,
            comm: "rustc".to_owned(),
            class: TaskClass::Compiler,
            process_starttime_ticks: Some(10),
            task_starttime_ticks: Some(10),
            cgroup_path: Some("/user.slice/app.scope".to_owned()),
        }];
        observation.refresh_situation_classification();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        let cgroup = result
            .evaluations
            .iter()
            .find(|evaluation| evaluation.action_kind == "cgroup_placement")
            .expect("expected cgroup evaluation");
        assert!(
            cgroup
                .deny_reasons
                .contains(&CandidateDenyReason::CapabilityMissing)
        );
    }

    #[test]
    fn planner_denies_cgroup_candidate_outside_allowlist() {
        let policy = suggest_policy_with_compile_cgroup();
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cgroup_candidate("/user.slice/not-allowlisted.slice"),
        }));
        let planner = CandidatePlanner::new(registry);
        let observation = observation();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::CgroupTargetNotAllowlisted)
        );
    }

    #[test]
    fn active_cpu_placement_experiment_blocks_cgroup_placement_candidate() {
        let policy = suggest_policy_with_compile_cgroup();
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cgroup_candidate("/user.slice/stutter-compile.slice"),
        }));
        let planner = CandidatePlanner::new(registry);
        let observation = observation();
        let mut capabilities = observation.capabilities.clone();
        capabilities.cgroup_v2_available = true;
        let controller_state = ControllerRuntimeState {
            active_experiment: Some(ActiveExperiment {
                experiment_id: ExperimentId::new("active-cpu-placement"),
                candidate: cpu_affinity_candidate("game-main"),
                baseline_score_total: 100,
            }),
            ..ControllerRuntimeState::default()
        };

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &capabilities,
            system_health: &observation.system_health,
            controller_state: &controller_state,
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::ConflictWithActiveAction)
        );
    }

    #[test]
    fn independent_nice_candidate_does_not_conflict_with_kept_cpu_placement() {
        let policy = policy(DaemonMode::Suggest);
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: nice_candidate(),
        }));
        let planner = CandidatePlanner::new(registry);
        let observation = observation();
        let kept = KeptCandidateState::new(
            ExperimentId::new("kept-cpu-placement"),
            cpu_affinity_candidate("game-main"),
            window_score(100),
            window_score(80),
            rollback_token(),
            10,
            "kept test candidate",
        );
        let active_profile_state = ActiveProfileState {
            current: Some(kept),
            history: Vec::new(),
        };

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: Some(&active_profile_state),
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(
            !result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::ConflictWithKeptAction)
        );
    }

    #[test]
    fn recording_situation_blocks_game_only_cpu_affinity_candidate() {
        let policy = policy(DaemonMode::Suggest);
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cpu_affinity_candidate("game-main"),
        }));
        let planner = CandidatePlanner::new(registry);
        let mut observation = observation();
        observation.primary_situation = SituationKind::Recording;
        observation.focus_kind = Some(FocusGroupKind::Recording);
        observation.refresh_situation_classification();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
    }

    #[test]
    fn planner_summary_groups_denials_and_manual_only_suggestions() {
        let policy = policy(DaemonMode::ApplyLowRisk);
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: irq_affinity_candidate("irq-manual"),
        }));
        let planner = CandidatePlanner::new(registry);
        let mut observation = observation();
        observation.primary_situation = SituationKind::IrqPressure;
        observation.focus_kind = Some(FocusGroupKind::Game);
        observation.capabilities.irq_affinity_available = true;
        observation.refresh_situation_classification();

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });
        let summary = result.summary();

        assert_eq!(summary.total_proposals, 1);
        assert_eq!(summary.eligible_proposals, 0);
        assert!(
            summary
                .grouped_denials
                .iter()
                .any(|denial| denial.reason_code == "manual_only_high_risk")
        );
        assert_eq!(summary.manual_only_suggestions, vec!["irq-manual"]);
        assert!(summary.no_action.is_some());
    }

    #[test]
    fn workload_memory_cools_down_same_workload_without_blocking_other_workload() {
        let policy = policy(DaemonMode::Suggest);
        let candidate = nice_candidate();
        let mut controller_state = ControllerRuntimeState::default();
        let mut same_workload = observation();
        same_workload.primary_situation = SituationKind::CompileCpuBound;
        same_workload.focus_kind = Some(FocusGroupKind::Compile);
        same_workload.refresh_situation_classification();
        controller_state.record_candidate_result(
            &candidate,
            &same_workload,
            None,
            CandidateMemoryResult::Reverted,
            Some(100),
            Some(120),
            Some("regressed".to_owned()),
            Some(same_workload.now_unix_nanos + 10_000),
        );

        let mut dry_runner = CountingDryRunner::default();
        let same_eval = evaluate_candidate_with_runner(
            &policy,
            &same_workload,
            &same_workload.capabilities,
            &controller_state,
            candidate.clone(),
            1.0,
            &mut dry_runner,
        );
        assert!(
            same_eval
                .deny_reasons
                .contains(&CandidateDenyReason::CooldownActive)
        );

        let mut other_workload = same_workload.clone();
        other_workload
            .workload_identity
            .as_mut()
            .unwrap()
            .stable_hash = "different-workload".to_owned();
        other_workload.workload_identity.as_mut().unwrap().exe_ino = Some(99);
        let mut dry_runner = CountingDryRunner::default();
        let other_eval = evaluate_candidate_with_runner(
            &policy,
            &other_workload,
            &other_workload.capabilities,
            &controller_state,
            candidate,
            1.0,
            &mut dry_runner,
        );

        assert!(
            !other_eval
                .deny_reasons
                .contains(&CandidateDenyReason::CooldownActive)
        );
    }

    #[test]
    fn game_cpu_affinity_profile_stays_apply_low_risk_eligible() {
        let evaluation = evaluate_static_candidate(
            DaemonMode::ApplyLowRisk,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            cpu_affinity_candidate("game-main"),
        );

        assert!(
            evaluation.eligible,
            "expected cpu_affinity_profile to remain apply-low-risk eligible; deny_reasons={:?} deny_messages={:?}",
            evaluation.deny_reasons, evaluation.deny_messages
        );
        assert!(
            !evaluation
                .deny_reasons
                .contains(&CandidateDenyReason::NotAutonomousForWorkload)
        );
    }

    #[test]
    fn game_uclamp_is_suggest_eligible_but_not_autonomous_for_apply() {
        let candidate = uclamp_candidate("game-uclamp");
        let suggest = evaluate_static_candidate(
            DaemonMode::Suggest,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            candidate.clone(),
        );

        assert!(
            suggest.eligible,
            "expected game uclamp to remain suggest eligible; deny_reasons={:?} deny_messages={:?}",
            suggest.deny_reasons, suggest.deny_messages
        );

        let apply = evaluate_static_candidate(
            DaemonMode::ApplyMediumRisk,
            SituationKind::GameCpuSchedulerPressure,
            FocusGroupKind::Game,
            candidate,
        );

        assert_autonomous_denial_mentions_context(
            &apply,
            SituationKind::GameCpuSchedulerPressure,
            "uclamp",
        );
    }

    #[test]
    fn compile_nice_is_suggest_eligible_but_not_autonomous_for_apply() {
        let candidate = nice_candidate();
        let suggest = evaluate_static_candidate(
            DaemonMode::Suggest,
            SituationKind::CompileCpuBound,
            FocusGroupKind::Compile,
            candidate.clone(),
        );

        assert!(
            suggest.eligible,
            "expected compile nice candidate to remain suggest eligible; deny_reasons={:?} deny_messages={:?}",
            suggest.deny_reasons, suggest.deny_messages
        );

        let apply = evaluate_static_candidate(
            DaemonMode::ApplyMediumRisk,
            SituationKind::CompileCpuBound,
            FocusGroupKind::Compile,
            candidate,
        );

        assert_autonomous_denial_mentions_context(&apply, SituationKind::CompileCpuBound, "nice");
    }

    #[test]
    fn empty_autonomous_families_block_browser_recording_irq_and_io_apply() {
        let cases = vec![
            (
                "browser nice",
                SituationKind::BrowserCpuPressure,
                FocusGroupKind::Browser,
                nice_candidate(),
            ),
            (
                "recording uclamp",
                SituationKind::Recording,
                FocusGroupKind::Recording,
                uclamp_candidate("recording-uclamp"),
            ),
            (
                "irq affinity",
                SituationKind::IrqPressure,
                FocusGroupKind::Desktop,
                irq_affinity_candidate("irq-affinity"),
            ),
            (
                "io ionice",
                SituationKind::IoPressure,
                FocusGroupKind::Desktop,
                ioprio_candidate("io-ionice"),
            ),
        ];

        for (label, situation, focus_kind, candidate) in cases {
            let action_kind = candidate.action_kind();
            let suggest = evaluate_static_candidate(
                DaemonMode::Suggest,
                situation,
                focus_kind,
                candidate.clone(),
            );

            assert!(
                suggest.eligible,
                "expected {label} to remain suggest eligible; deny_reasons={:?} deny_messages={:?}",
                suggest.deny_reasons, suggest.deny_messages
            );

            let apply = evaluate_static_candidate(
                DaemonMode::ApplyMediumRisk,
                situation,
                focus_kind,
                candidate,
            );

            assert_autonomous_denial_mentions_context(&apply, situation, action_kind);
        }
    }

    #[test]
    fn browser_foreground_blocks_compile_throughput_optimization() {
        let policy = suggest_policy_with_compile_cgroup();
        let mut registry = CandidateProviderRegistry::default();
        registry.register(Box::new(StaticProvider {
            candidate: cgroup_candidate("/user.slice/stutter-compile.slice"),
        }));
        let planner = CandidatePlanner::new(registry);
        let mut observation = observation();
        observation.primary_situation = SituationKind::BrowserFocused;
        observation.focus_kind = Some(FocusGroupKind::Browser);
        observation.refresh_situation_classification();
        let mut capabilities = observation.capabilities.clone();
        capabilities.cgroup_v2_available = true;

        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &capabilities,
            system_health: &observation.system_health,
            controller_state: &ControllerRuntimeState::default(),
            active_profile_state: None,
            workload_policy: &WorkloadPolicyMatrix::default_rules(),
            profiles: &[],
        });

        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::WorkloadPolicyBlocked)
        );
        assert!(
            result.evaluations[0]
                .deny_reasons
                .contains(&CandidateDenyReason::ObjectiveNotAllowedForWorkload)
        );
    }

    #[derive(Debug, Deserialize)]
    struct PlannerGoldenCase {
        situation: String,
        focus_kind: String,
        mode: String,
        candidate_action_kind: String,
        expected_selected_action_kind: Option<String>,
        #[serde(default)]
        expected_denials: Vec<ExpectedDenial>,
        #[serde(default)]
        low_data_quality: bool,
        #[serde(default)]
        critical_realtime: bool,
        #[serde(default)]
        cooldown_active: bool,
        #[serde(default)]
        kept_conflict: bool,
        #[serde(default)]
        external_mutation: bool,
    }

    #[derive(Debug, Deserialize)]
    struct ExpectedDenial {
        action_kind: String,
        reason: String,
    }

    #[test]
    fn planner_golden_cases() {
        let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("testdata/autotune/planner");
        let mut paths = std::fs::read_dir(&fixture_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        paths.sort();

        assert_eq!(paths.len(), 20);

        for path in paths {
            let text = std::fs::read_to_string(&path).unwrap();
            let case: PlannerGoldenCase = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
            run_planner_golden_case(&path, case);
        }
    }

    fn run_planner_golden_case(path: &std::path::Path, case: PlannerGoldenCase) {
        let mode = case.mode.parse::<DaemonMode>().unwrap();
        let mut policy = policy(mode);
        policy.cgroup_targets.compile_cgroup = Some(std::path::PathBuf::from(
            "/user.slice/stutter-compile.slice",
        ));
        let candidate = candidate_for_action_kind(&case.candidate_action_kind);
        let mut observation = observation_for_situation(
            parse_fixture_situation(&case.situation),
            parse_fixture_focus_kind(&case.focus_kind),
        );
        observation.capabilities.uclamp_available = true;
        observation.capabilities.ionice_available = true;
        observation.capabilities.cgroup_v2_available = true;
        observation.capabilities.irq_affinity_available = true;
        observation.capabilities.gpu_sysfs_available = true;
        if case.low_data_quality {
            observation.data_quality = OnlineDataQuality::Low {
                reasons: vec!["fixture low data quality".to_owned()],
            };
        }
        if case.critical_realtime {
            observation
                .focus_reasons
                .push("critical realtime input process present".to_owned());
        }
        observation.refresh_situation_classification();

        let mut controller_state = ControllerRuntimeState::default();
        if case.cooldown_active {
            controller_state.record_candidate_result(
                &candidate,
                &observation,
                None,
                CandidateMemoryResult::Reverted,
                Some(100),
                Some(120),
                Some("fixture cooldown".to_owned()),
                Some(observation.now_unix_nanos + 10_000),
            );
        }
        if case.external_mutation {
            controller_state.active_experiment = Some(ActiveExperiment {
                experiment_id: ExperimentId::new("fixture-external-mutation"),
                candidate: candidate.clone(),
                baseline_score_total: 100,
            });
            observation.active_config_snapshot = Some(active_nice_snapshot(1234, 0));
        }
        let active_profile_state = case.kept_conflict.then(|| ActiveProfileState {
            current: Some(KeptCandidateState::new(
                ExperimentId::new("fixture-kept-conflict"),
                candidate.clone(),
                window_score(100),
                window_score(90),
                rollback_token(),
                observation.now_unix_nanos,
                "fixture kept conflict",
            )),
            history: Vec::new(),
        });

        let proposals = vec![proposal_for_candidate(candidate, 1.0)];
        let mut dry_runner = CountingDryRunner::default();
        let mut evaluations = evaluate_proposals_with_runner(
            PlannerInput {
                observation: &observation,
                daemon_policy: &policy,
                capabilities: &observation.capabilities,
                system_health: &observation.system_health,
                controller_state: &controller_state,
                active_profile_state: active_profile_state.as_ref(),
                workload_policy: &WorkloadPolicyMatrix::default_rules(),
                profiles: &[],
            },
            proposals,
            &mut dry_runner,
        );
        sort_candidate_evaluations(&mut evaluations);
        let selected = evaluations
            .iter()
            .find(|evaluation| evaluation.eligible)
            .map(|evaluation| evaluation.action_kind.clone());

        assert_eq!(
            selected,
            case.expected_selected_action_kind,
            "fixture {} selected action mismatch; evaluations={evaluations:#?}",
            path.display()
        );

        for expected in case.expected_denials {
            let expected_reason = CandidateDenyReason::from_reason_code(&expected.reason)
                .unwrap_or_else(|| panic!("unknown expected reason {}", expected.reason));
            assert!(
                evaluations.iter().any(|evaluation| {
                    evaluation.action_kind == expected.action_kind
                        && evaluation.deny_reasons.contains(&expected_reason)
                }),
                "fixture {} missing denial {:?} for {}; evaluations={evaluations:#?}",
                path.display(),
                expected_reason,
                expected.action_kind
            );
        }
    }

    impl CandidateDenyReason {
        fn from_reason_code(value: &str) -> Option<Self> {
            [
                CandidateDenyReason::DataQualityLow,
                CandidateDenyReason::CriticalRealtimeWarning,
                CandidateDenyReason::CooldownActive,
                CandidateDenyReason::ConflictWithKeptAction,
                CandidateDenyReason::ExternalMutationDetected,
            ]
            .into_iter()
            .find(|reason| reason.reason_code() == value)
        }
    }

    fn parse_fixture_situation(value: &str) -> SituationKind {
        crate::autotune::workload_policy::parse_situation_kind(value).unwrap()
    }

    fn parse_fixture_focus_kind(value: &str) -> FocusGroupKind {
        match value {
            "Game" => FocusGroupKind::Game,
            "Desktop" => FocusGroupKind::Desktop,
            "Browser" => FocusGroupKind::Browser,
            "Compile" => FocusGroupKind::Compile,
            "Recording" => FocusGroupKind::Recording,
            "Media" => FocusGroupKind::Media,
            "VirtualMachine" => FocusGroupKind::VirtualMachine,
            other => panic!("unsupported focus kind {other}"),
        }
    }
}
