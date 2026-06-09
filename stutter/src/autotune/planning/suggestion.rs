//! Candidate suggestion rendering and manual command metadata.

use std::path::Path;

use super::{
    candidate::{CandidateAction, CandidateEvidence},
    dry_run::CandidateDryRunRecord,
    plan_io::{candidate_plan_path, write_candidate_plan_file},
};
use crate::{
    actions::SafetyClass,
    autotune::objective::ObjectiveKind,
    daemon_policy::{
        ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy, PolicyIntent,
        RollbackRequirement,
    },
};

#[derive(Clone, Debug)]
pub struct CandidateSuggestion {
    pub candidate_name: String,
    pub action_kind: String,
    pub descriptor: ActionDescriptor,
    pub objective: ObjectiveKind,
    pub evidence: Vec<CandidateEvidence>,
    pub affected_tasks: usize,
    pub safety: SafetyClass,
    pub reason: String,
    pub dry_run_command: Option<String>,
    pub manual_apply_command: Option<String>,
    pub required_mode: DaemonMode,
    pub required_safety_class: SafetyClass,
    pub manual_only_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateManualCommands {
    pub dry_run_command: Option<String>,
    pub manual_apply_command: Option<String>,
    pub required_mode: DaemonMode,
    pub required_safety_class: SafetyClass,
    pub manual_only_reason: Option<String>,
}

impl CandidateAction {
    pub fn manual_commands(
        &self,
        plan_path: &Path,
        policy: &DaemonPolicy,
    ) -> CandidateManualCommands {
        let required_mode = required_mode_for_safety_class(&self.safety_class());
        let descriptor = self.descriptor();
        let manual_only_reason = self.manual_only_reason();
        let dry_run_command = Some(format!(
            "stutter autotune apply-candidate --candidate-json {} --dry-run",
            plan_path.display()
        ));
        let manual_apply_command = if manual_only_reason.is_none()
            && policy
                .check_action(PolicyIntent::Apply, &descriptor)
                .is_ok()
        {
            Some(format!(
                "stutter autotune apply-candidate --candidate-json {}",
                plan_path.display()
            ))
        } else {
            None
        };

        CandidateManualCommands {
            dry_run_command,
            manual_apply_command,
            required_mode,
            required_safety_class: self.safety_class(),
            manual_only_reason,
        }
    }
}

pub fn suggestion_from_candidate_dry_run_record(
    candidate: &CandidateAction,
    record: &CandidateDryRunRecord,
    plan_path: &Path,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: impl Into<String>,
) -> Option<CandidateSuggestion> {
    if !record.eligible {
        return None;
    }

    if record.safety_class > max_safety_class {
        return None;
    }

    if let CandidateAction::CpuAffinityProfile { .. } = candidate {
        return cpu_affinity_suggestion_from_dry_run_record(
            record,
            candidate.tree_pid(),
            profile_path,
            max_safety_class,
            reason,
        );
    }

    let required_mode = required_mode_for_safety_class(&record.safety_class);
    let policy = policy_for_required_mode(required_mode)
        .unwrap_or_else(|| DaemonPolicy::suggest(ActionSource::Cli));
    let commands = candidate.manual_commands(plan_path, &policy);

    Some(CandidateSuggestion {
        candidate_name: candidate.candidate_name().to_owned(),
        action_kind: candidate.action_kind().to_owned(),
        descriptor: candidate.descriptor(),
        objective: candidate.objective(),
        evidence: candidate.evidence().to_vec(),
        affected_tasks: record.affected_tasks,
        safety: record.safety_class.clone(),
        reason: reason.into(),
        dry_run_command: commands.dry_run_command,
        manual_apply_command: commands.manual_apply_command,
        required_mode: commands.required_mode,
        required_safety_class: commands.required_safety_class,
        manual_only_reason: commands.manual_only_reason,
    })
}

pub fn suggestion_from_dry_run_record(
    record: &CandidateDryRunRecord,
    tree_pid: u32,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: impl Into<String>,
) -> Option<CandidateSuggestion> {
    cpu_affinity_suggestion_from_dry_run_record(
        record,
        tree_pid,
        profile_path,
        max_safety_class,
        reason,
    )
}

fn cpu_affinity_suggestion_from_dry_run_record(
    record: &CandidateDryRunRecord,
    tree_pid: u32,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: impl Into<String>,
) -> Option<CandidateSuggestion> {
    if !record.eligible {
        return None;
    }

    if record.safety_class > max_safety_class {
        return None;
    }

    let profile_arg = profile_path
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<generated-or-existing-profile>".to_owned());
    let required_mode = required_mode_for_safety_class(&record.safety_class);
    let descriptor = suggestion_action_descriptor(record, required_mode);
    let manual_only_reason = (required_mode == DaemonMode::ApplyHighRisk)
        .then(|| "high-risk apply is not implemented; manual investigation only".to_owned());
    let dry_run_command = Some(apply_profile_command(tree_pid, &profile_arg, true, false));
    let manual_apply_command = if manual_only_reason.is_some() {
        None
    } else {
        manual_apply_command_if_policy_allows(tree_pid, &profile_arg, required_mode, &descriptor)
    };

    Some(CandidateSuggestion {
        candidate_name: record.candidate_name.clone(),
        action_kind: "cpu_affinity_profile".to_owned(),
        descriptor,
        objective: ObjectiveKind::StutterScore,
        evidence: Vec::new(),
        affected_tasks: record.affected_tasks,
        safety: record.safety_class.clone(),
        reason: reason.into(),
        dry_run_command,
        manual_apply_command,
        required_mode,
        required_safety_class: record.safety_class.clone(),
        manual_only_reason,
    })
}

fn required_mode_for_safety_class(safety_class: &SafetyClass) -> DaemonMode {
    match safety_class {
        SafetyClass::ObserveOnly | SafetyClass::ReversibleLowRisk => DaemonMode::ApplyLowRisk,
        SafetyClass::ReversibleMediumRisk => DaemonMode::ApplyMediumRisk,
        SafetyClass::HighRisk => DaemonMode::ApplyHighRisk,
    }
}

fn suggestion_action_descriptor(
    record: &CandidateDryRunRecord,
    required_mode: DaemonMode,
) -> ActionDescriptor {
    ActionDescriptor {
        action_id: crate::actions::ActionId::new(format!(
            "cpu-affinity-profile:{}",
            record.candidate_name
        )),
        action_kind: "cpu_affinity_profile".to_owned(),
        safety_class: record.safety_class.clone(),
        effect_scope: ActionEffectScope::LocalProcessTree,
        rollback: RollbackRequirement::RequiredBeforeApply,
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: required_mode.supports_apply(),
        confidence: None,
    }
}

fn policy_for_required_mode(required_mode: DaemonMode) -> Option<DaemonPolicy> {
    match required_mode {
        DaemonMode::ApplyLowRisk => Some(DaemonPolicy::apply_low_risk(ActionSource::Cli)),
        DaemonMode::ApplyMediumRisk => Some(DaemonPolicy::apply_medium_risk(ActionSource::Cli)),
        DaemonMode::Observe | DaemonMode::Suggest | DaemonMode::ApplyHighRisk => None,
    }
}

fn manual_apply_command_if_policy_allows(
    tree_pid: u32,
    profile_arg: &str,
    required_mode: DaemonMode,
    descriptor: &ActionDescriptor,
) -> Option<String> {
    let policy = policy_for_required_mode(required_mode)?;
    policy.check_action(PolicyIntent::Apply, descriptor).ok()?;

    Some(apply_profile_command(
        tree_pid,
        profile_arg,
        false,
        required_mode == DaemonMode::ApplyMediumRisk,
    ))
}

fn apply_profile_command(
    tree_pid: u32,
    profile_arg: &str,
    dry_run: bool,
    allow_medium_risk: bool,
) -> String {
    let mut command =
        format!("stutter apply-profile --tree-pid {tree_pid} --profile {profile_arg}");

    if dry_run {
        command.push_str(" --dry-run");
    }

    if allow_medium_risk {
        command.push_str(" --allow-medium-risk");
    }

    command
}

pub fn suggestions_from_candidates_and_dry_run_records(
    candidates: &[CandidateAction],
    records: &[CandidateDryRunRecord],
    plan_dir: &Path,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: &str,
) -> anyhow::Result<Vec<CandidateSuggestion>> {
    let mut suggestions = Vec::new();

    for (candidate, record) in candidates.iter().zip(records.iter()) {
        let plan_path = candidate_plan_path(candidate, plan_dir);

        if !matches!(candidate, CandidateAction::CpuAffinityProfile { .. })
            && record.eligible
            && record.safety_class <= max_safety_class
        {
            write_candidate_plan_file(&plan_path, candidate, Some(record.affected_tasks))?;
        }

        if let Some(suggestion) = suggestion_from_candidate_dry_run_record(
            candidate,
            record,
            &plan_path,
            profile_path,
            max_safety_class.clone(),
            reason.to_owned(),
        ) {
            suggestions.push(suggestion);
        }
    }

    Ok(suggestions)
}

pub fn suggestions_from_dry_run_records(
    records: &[CandidateDryRunRecord],
    tree_pid: u32,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: &str,
) -> Vec<CandidateSuggestion> {
    records
        .iter()
        .filter_map(|record| {
            suggestion_from_dry_run_record(
                record,
                tree_pid,
                profile_path,
                max_safety_class.clone(),
                reason.to_owned(),
            )
        })
        .collect()
}

pub fn render_candidate_suggestion(suggestion: &CandidateSuggestion) -> String {
    format!(
        "autotune suggestion:\n  candidate={}\n  action={}\n  action_kind={}\n  action_id={}\n  objective={:?}\n  affected_tasks={}\n  safety={:?}\n  reason=\"{}\"\n  note=\"suggest mode did not apply this change\"\n  required_mode={}\n  required_safety_class={:?}\n  rollback=\"stutter restore\"\n  dry_run_command={}\n  manual_apply_command={}\n  manual_only_reason={}",
        shell_safe_value(&suggestion.candidate_name),
        shell_safe_value(&suggestion.action_kind.replace('_', "-")),
        shell_safe_value(&suggestion.action_kind),
        shell_safe_value(suggestion.descriptor.action_id.as_str()),
        suggestion.objective,
        suggestion.affected_tasks,
        suggestion.safety,
        escape_quoted_value(&suggestion.reason),
        suggestion.required_mode,
        suggestion.required_safety_class,
        render_optional_command(&suggestion.dry_run_command),
        render_optional_command(&suggestion.manual_apply_command),
        render_optional_command(&suggestion.manual_only_reason)
    )
}

pub fn render_candidate_suggestions(suggestions: &[CandidateSuggestion]) -> String {
    suggestions
        .iter()
        .map(render_candidate_suggestion)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn print_candidate_suggestions(suggestions: &[CandidateSuggestion]) {
    for suggestion in suggestions {
        println!("{}", render_candidate_suggestion(suggestion));
    }
}

fn render_optional_command(command: &Option<String>) -> String {
    match command {
        Some(command) => format!("\"{}\"", escape_quoted_value(command)),
        None => "none".to_owned(),
    }
}

fn shell_safe_value(value: &str) -> String {
    if value.is_empty() {
        return "-".to_owned();
    }

    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/' | '@'))
    {
        value.to_owned()
    } else {
        format!("\"{}\"", escape_quoted_value(value))
    }
}

fn escape_quoted_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }

    escaped
}
