//! Candidate evaluation and dry-run gating; this module owns turning proposals into planner evaluations.

use crate::{
    actions::ActionState,
    autotune::{
        active_config::{ActiveConfigMatch, ActiveConfigMatchInput},
        activity::ActivityLevel,
        candidate::{CandidateAction, CandidateDryRunner, CandidateEvidence},
        candidate_memory::CandidateContextHashInput,
        planning::{
            denial::{CandidateDenyReason, normalize_evaluation_denials},
            model::{CandidateEvaluation, CandidateEvaluationDraft},
            planner::PlannerInput,
            policy::{
                deny_reason_from_policy, mode_requires_autonomous_workload_family,
                policy_context_for_input, policy_family_denied, policy_family_enabled,
                policy_intent_for_mode,
            },
            ranking::rank_with_workload_memory,
        },
        providers::CandidateProposal,
    },
    daemon::{DaemonPolicy, policy::DaemonMode},
};

pub(crate) fn evaluate_proposals_with_runner<R: CandidateDryRunner>(
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

pub(crate) fn evaluate_proposal_static(
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
        match proposal
            .candidate
            .matches_active_config(active_config_input)
        {
            ActiveConfigMatch::Matches { summary } => {
                deny_reasons.push(CandidateDenyReason::NoEffectiveChange);
                deny_messages.push(format!(
                    "candidate would not change active configuration: {summary}"
                ));
            }
            ActiveConfigMatch::Differs { .. } => {}
            ActiveConfigMatch::Unknown { summary } => {
                deny_reasons.push(CandidateDenyReason::ActiveConfigUnknown);
                deny_messages.push(format!(
                    "candidate active configuration could not be verified conservatively: {summary}"
                ));
            }
        }

        if let Some(active_experiment) = input.controller_state.active_experiment.as_ref()
            && proposal
                .candidate
                .conflicts_with(&active_experiment.candidate)
        {
            match active_experiment
                .candidate
                .matches_active_config(active_config_input)
            {
                ActiveConfigMatch::Differs { expected, actual } => {
                    deny_reasons.push(CandidateDenyReason::ExternalMutationDetected);
                    deny_messages.push(format!(
                        "active experiment no longer matches live state for conflict group {:?}; expected {}; actual {}; restore or resync before applying another conflicting action",
                        active_experiment.candidate.conflict_group(),
                        expected,
                        actual
                    ));
                }
                ActiveConfigMatch::Unknown { summary } => {
                    deny_reasons.push(CandidateDenyReason::ActiveConfigUnknown);
                    deny_messages.push(format!(
                        "active experiment configuration is unknown for conflict group {:?}: {summary}; restore or resync before applying another conflicting action",
                        active_experiment.candidate.conflict_group()
                    ));
                }
                ActiveConfigMatch::Matches { .. } => {}
            }
        }

        if let Some(state) = input.active_profile_state {
            for kept in state.kept_actions.values() {
                match kept.candidate.matches_active_config(active_config_input) {
                    ActiveConfigMatch::Differs { expected, actual } => {
                        deny_reasons.push(CandidateDenyReason::KeptActionNoLongerActive);
                        deny_messages.push(format!(
                            "kept action {} no longer matches live state; expected {}; actual {}; restore or resync before planning new candidates",
                            kept.candidate.candidate_name(),
                            expected,
                            actual
                        ));
                    }
                    ActiveConfigMatch::Unknown { summary } => {
                        deny_reasons.push(CandidateDenyReason::ActiveConfigUnknown);
                        deny_messages.push(format!(
                            "kept action {} active configuration is unknown: {summary}; restore or resync before planning new candidates",
                            kept.candidate.candidate_name()
                        ));
                    }
                    ActiveConfigMatch::Matches { .. } => {}
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

pub(crate) fn dry_run_candidate_if_still_eligible<R: CandidateDryRunner>(
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
