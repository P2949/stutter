use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    actions::{ActionState, SafetyClass},
    autotune::{
        active_config::{ActiveConfigMatch, ActiveConfigMatchInput},
        activity::ActivityLevel,
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
    daemon::{
        DaemonPolicy, capabilities::DaemonCapabilities, health::SystemHealthSnapshot,
        policy::DaemonMode,
    },
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
    SystemWideTargetNotAllowlisted,
    WorkloadPolicyBlocked,
    NotAutonomousForWorkload,
    ProviderConfidenceTooLow,
    NoEffectiveChange,
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
    pub effect_scope: crate::daemon_policy::ActionEffectScope,
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
        let mut dry_runner = RealCandidateDryRunner;
        self.plan_with_dry_runner(input, &mut dry_runner)
    }

    fn plan_with_dry_runner<R: CandidateDryRunner>(
        &self,
        input: PlannerInput<'_>,
        dry_runner: &mut R,
    ) -> PlanResult {
        if input.daemon_policy.mode == crate::daemon::policy::DaemonMode::Observe {
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
        let mut evaluations = evaluate_proposals_with_runner(input, proposals, dry_runner);

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

    if input.observation.activity_level == ActivityLevel::Idle
        && !matches!(
            input.observation.primary_situation,
            crate::autotune::situation::SituationKind::Idle
        )
    {
        deny_reasons.push(CandidateDenyReason::WorkloadIdle);
        deny_messages.push(format!(
            "workload activity is idle for non-idle situation {:?}",
            input.observation.primary_situation
        ));
    }

    if proposal.deny_reasons.iter().any(|reason| {
        reason.contains("target_selection_fallback_root")
            || reason.contains("target_snapshot_missing")
    }) {
        deny_reasons.push(CandidateDenyReason::TargetSnapshotMissing);
    }

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

    if proposal.candidate.is_high_risk_system_adjacent() {
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
        .and_then(|state| state.conflicting_kept_action(&proposal.candidate))
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

    if let Some(message) = system_wide_allowlist_denial(&proposal.candidate, input.daemon_policy) {
        deny_reasons.push(CandidateDenyReason::SystemWideTargetNotAllowlisted);
        deny_messages.push(message);
    }

    if let Some(snapshot) = input.observation.active_config_snapshot.as_ref() {
        let active_config_input = ActiveConfigMatchInput {
            snapshot,
            active_tasks: &input.observation.active_tasks,
        };
        if let ActiveConfigMatch::Matches { summary } = proposal
            .candidate
            .matches_active_config(active_config_input)
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
            && let ActiveConfigMatch::Differs { expected, actual } = active_experiment
                .candidate
                .matches_active_config(active_config_input)
        {
            deny_reasons.push(CandidateDenyReason::ExternalMutationDetected);
            deny_messages.push(format!(
                "active experiment no longer matches live state for conflict group {:?}; expected {}; actual {}; restore or resync before applying another conflicting action",
                active_experiment.candidate.conflict_group(),
                expected,
                actual
            ));
        }

        if let Some(state) = input.active_profile_state {
            for kept in state.kept_actions.values() {
                if let ActiveConfigMatch::Differs { expected, actual } =
                    kept.candidate.matches_active_config(active_config_input)
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
        }
    }

    normalize_evaluation_denials(&mut deny_reasons, &mut deny_messages);

    let candidate_name = proposal.candidate.candidate_name().to_owned();
    let action_kind = proposal.candidate.action_kind().to_owned();
    let mut evidence = proposal.candidate.evidence().to_vec();
    evidence.push(crate::autotune::candidate::CandidateEvidence::new(
        "situation",
        format!("{:?}", input.observation.primary_situation),
        input.observation.situation.confidence,
    ));

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
    let high_risk_suggest_dry_run = input.daemon_policy.mode == DaemonMode::Suggest
        && input.daemon_policy.high_risk_dry_run
        && draft.candidate.is_high_risk_system_adjacent()
        && deny_reasons
            .iter()
            .all(|reason| *reason == CandidateDenyReason::ManualOnlyHighRisk);

    if deny_reasons.is_empty() || high_risk_suggest_dry_run {
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

fn system_wide_allowlist_denial(
    candidate: &CandidateAction,
    policy: &DaemonPolicy,
) -> Option<String> {
    let allowlist = &policy.system_wide_allowlist;

    match candidate {
        CandidateAction::CpuPower { plan } => {
            let policies = evidence_token_values(&plan.evidence, "policy=");
            if policies.is_empty() {
                return Some(
                    "CPU power policy target is not identified or allowlisted by daemon policy"
                        .to_owned(),
                );
            }

            policies
                .into_iter()
                .find(|target| !allowlist.cpu_policies.contains(target))
                .map(|target| {
                    format!("CPU power policy {target} is not allowlisted by daemon policy")
                })
        }
        CandidateAction::GpuPower { plan } => {
            let pci_id = evidence_token_values(&plan.evidence, "pci_id=")
                .into_iter()
                .next();

            (!allowlist.allows_gpu(&plan.action.drm_card, pci_id.as_deref())).then(
                || match pci_id {
                    Some(pci_id) => format!(
                        "GPU target {} ({pci_id}) is not allowlisted by daemon policy",
                        plan.action.drm_card
                    ),
                    None => format!(
                        "GPU target {} is not allowlisted by daemon policy",
                        plan.action.drm_card
                    ),
                },
            )
        }
        CandidateAction::IrqAffinity { plan } => {
            (!allowlist.allows_irq_device(&plan.action.device_hint)).then(|| {
                format!(
                    "IRQ device {} is not allowlisted by daemon policy",
                    plan.action.device_hint
                )
            })
        }
        CandidateAction::VmKnob { plan } => plan
            .action
            .changes
            .iter()
            .find(|change| !allowlist.allows_vm_knob(&change.path))
            .map(|change| {
                format!(
                    "VM knob {} is not allowlisted by daemon policy",
                    change.path.display()
                )
            }),
        _ => None,
    }
}

fn evidence_token_values(evidence: &[CandidateEvidence], prefix: &str) -> Vec<String> {
    evidence
        .iter()
        .flat_map(|entry| entry.value.split_whitespace())
        .filter_map(|token| token.strip_prefix(prefix))
        .filter_map(normalize_evidence_token_value)
        .collect()
}

fn normalize_evidence_token_value(value: &str) -> Option<String> {
    let mut value = value.trim().trim_end_matches(',');
    if value == "None" {
        return None;
    }

    if let Some(inner) = value
        .strip_prefix("Some(")
        .and_then(|value| value.strip_suffix(')'))
    {
        value = inner;
    }

    let value = value.trim_matches('"');
    (!value.is_empty()).then(|| value.to_owned())
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
#[path = "../planner_tests/mod.rs"]
mod planner_tests;
