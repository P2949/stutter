//! Candidate plan file I/O and apply support.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    candidate::{
        ApplyEligibility, CandidateAction, CandidateEvidence, try_promote_to_apply_candidate,
    },
    dry_run::dry_run_candidate,
    executable_plan::CandidateExecutablePlan,
    suggestion::CandidateSuggestion,
};
use crate::{
    actions::SafetyClass,
    autotune::objective::ObjectiveKind,
    daemon_policy::{ActionDescriptor, ActionSource, DaemonPolicy, PolicyIntent},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidatePlanFile {
    pub schema_version: u32,
    pub candidate: CandidatePlanSummary,
    pub descriptor: ActionDescriptor,
    pub objective: ObjectiveKind,
    pub evidence: Vec<CandidateEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_apply_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_only_reason: Option<String>,
    pub executable: Option<CandidateExecutablePlan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidatePlanSummary {
    pub candidate_name: String,
    pub action_kind: String,
    pub affected_tasks: Option<usize>,
    pub reason: Option<String>,
}

impl CandidatePlanFile {
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn from_suggestion(suggestion: &CandidateSuggestion) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            candidate: CandidatePlanSummary {
                candidate_name: suggestion.candidate_name.clone(),
                action_kind: suggestion.action_kind.clone(),
                affected_tasks: Some(suggestion.affected_tasks),
                reason: Some(suggestion.reason.clone()),
            },
            descriptor: suggestion.descriptor.clone(),
            objective: suggestion.objective,
            evidence: suggestion.evidence.clone(),
            manual_apply_command: suggestion.manual_apply_command.clone(),
            manual_only_reason: suggestion.manual_only_reason.clone(),
            executable: None,
        }
    }

    pub fn from_candidate(candidate: &CandidateAction, affected_tasks: Option<usize>) -> Self {
        let (manual_apply_command, manual_only_reason) = candidate_plan_manual_metadata(candidate);
        Self {
            schema_version: Self::SCHEMA_VERSION,
            candidate: CandidatePlanSummary {
                candidate_name: candidate.candidate_name().to_owned(),
                action_kind: candidate.action_kind().to_owned(),
                affected_tasks,
                reason: Some(candidate.describe()),
            },
            descriptor: candidate.descriptor(),
            objective: candidate.objective(),
            evidence: candidate.evidence().to_vec(),
            manual_apply_command,
            manual_only_reason,
            executable: CandidateExecutablePlan::from_candidate(candidate),
        }
    }
}

fn candidate_plan_manual_metadata(candidate: &CandidateAction) -> (Option<String>, Option<String>) {
    match candidate {
        CandidateAction::CpuAffinityProfile { plan } => (
            Some(format!(
                "stutter apply-profile --tree-pid {} --profile <generated-or-existing-profile>",
                plan.tree_pid
            )),
            Some("cpu-affinity profiles use apply-profile, not candidate-plan apply".to_owned()),
        ),
        _ => (None, candidate.manual_only_reason()),
    }
}

pub fn default_candidate_plan_dir() -> PathBuf {
    let mut path = crate::autotune::history::default_autotune_history_path();
    path.pop();
    path.push("candidate_plans");
    path
}

pub fn candidate_plan_path(candidate: &CandidateAction, plan_dir: &Path) -> PathBuf {
    plan_dir.join(format!(
        "{}-{}.json",
        sanitize_candidate_plan_component(candidate.action_kind()),
        sanitize_candidate_plan_component(candidate.candidate_name())
    ))
}

pub fn write_candidate_plan_file(
    path: &Path,
    candidate: &CandidateAction,
    affected_tasks: Option<usize>,
) -> anyhow::Result<CandidatePlanFile> {
    let plan = CandidatePlanFile::from_candidate(candidate, affected_tasks);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let bytes = serde_json::to_vec_pretty(&plan)?;
    fs::write(path, bytes)?;

    Ok(plan)
}

fn sanitize_candidate_plan_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            sanitized.push(ch);
        } else {
            sanitized.push('-');
        }
    }

    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "candidate".to_owned()
    } else {
        sanitized.to_owned()
    }
}

pub fn apply_candidate_plan_file(path: &Path, dry_run: bool) -> anyhow::Result<CandidatePlanFile> {
    let bytes = std::fs::read(path)?;
    let plan: CandidatePlanFile = serde_json::from_slice(&bytes)?;

    if plan.schema_version != CandidatePlanFile::SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported_candidate_plan_schema: got {} expected {}",
            plan.schema_version,
            CandidatePlanFile::SCHEMA_VERSION
        );
    }

    if dry_run {
        if let Some(executable) = plan.executable.clone() {
            let candidate = executable.into_candidate();
            let record = dry_run_candidate(&candidate);
            if !record.eligible {
                anyhow::bail!(
                    "candidate_plan_dry_run_failed: candidate '{}' action_kind={} reason={}",
                    plan.candidate.candidate_name,
                    plan.candidate.action_kind,
                    record
                        .reason
                        .as_deref()
                        .unwrap_or("dry-run did not produce an eligible candidate")
                );
            }
        }
        return Ok(plan);
    }

    if plan.descriptor.safety_class == SafetyClass::HighRisk
        || plan.descriptor.touches_system_wide_state
    {
        anyhow::bail!(
            "manual_only_high_risk: candidate '{}' action_kind={} cannot be applied by this command",
            plan.candidate.candidate_name,
            plan.candidate.action_kind
        );
    }

    let Some(executable) = plan.executable.clone() else {
        if let Some(reason) = plan.manual_only_reason.as_deref() {
            anyhow::bail!(
                "candidate_plan_manual_only: candidate '{}' action_kind={} reason={} manual_apply_command={}",
                plan.candidate.candidate_name,
                plan.candidate.action_kind,
                reason,
                plan.manual_apply_command.as_deref().unwrap_or("none")
            );
        }
        anyhow::bail!(
            "candidate_plan_payload_not_executable: candidate '{}' action_kind={} requires an executable candidate payload",
            plan.candidate.candidate_name,
            plan.candidate.action_kind
        );
    };

    let candidate = executable.into_candidate();
    if candidate.descriptor().action_id != plan.descriptor.action_id
        || candidate.action_kind() != plan.descriptor.action_kind
    {
        anyhow::bail!(
            "candidate_plan_descriptor_mismatch: candidate '{}' executable payload does not match descriptor",
            plan.candidate.candidate_name
        );
    }

    let policy = match plan.descriptor.safety_class {
        SafetyClass::ObserveOnly | SafetyClass::ReversibleLowRisk => {
            DaemonPolicy::apply_low_risk(ActionSource::Cli)
        }
        SafetyClass::ReversibleMediumRisk => DaemonPolicy::apply_medium_risk(ActionSource::Cli),
        SafetyClass::HighRisk => {
            anyhow::bail!(
                "manual_only_high_risk: candidate '{}' action_kind={} cannot be applied by this command",
                plan.candidate.candidate_name,
                plan.candidate.action_kind
            );
        }
    };
    policy.check_action(PolicyIntent::Apply, &plan.descriptor)?;

    let apply_candidate = try_promote_to_apply_candidate(candidate, ApplyEligibility::approved())
        .map_err(|eligibility| {
        crate::autotune::AutotunePlanError::ApplyDenied {
            message: format!(
                "candidate_plan_apply_denied: {}",
                eligibility.denial_message()
            ),
        }
    })?;
    let executor = crate::autotune::apply::executor_for_apply_candidate(apply_candidate)?;
    let result = executor.apply_with_audit(crate::actions::runner::ActionRunPolicy {
        policy,
        context: crate::daemon_policy::DaemonPolicyContext::default(),
        max_affected_tasks: None,
        max_total_duration: None,
        dry_run: false,
    })?;
    if result.rollback.is_none() {
        anyhow::bail!(
            "candidate_plan_apply_missing_rollback: candidate '{}' action_kind={} applied without rollback token",
            plan.candidate.candidate_name,
            plan.candidate.action_kind
        );
    }

    Ok(plan)
}
