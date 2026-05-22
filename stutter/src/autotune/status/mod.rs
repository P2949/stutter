use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{
    history::{
        AutotuneHistoryEvent, AutotuneMode, ControllerPhase, TargetIdentity,
        default_autotune_history_path, read_autotune_history_events,
    },
    planner::{PlannerEvaluationSummary, PlannerSummary},
};
use crate::daemon::{
    policy::DaemonMode,
    state::{
        DaemonPhase, DaemonState, DaemonTargetState, default_daemon_state_snapshot_path,
        load_daemon_state,
    },
};

#[derive(Clone, Debug)]
pub struct AutotuneStatusCommandInput {
    pub json: bool,
    pub history_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AutotuneStatus {
    pub phase: String,
    pub mode: String,
    pub target: Option<StatusTarget>,
    pub focus_group: Option<String>,
    pub current_score: Option<u64>,
    pub active_profile: Option<String>,
    pub active_candidate: Option<String>,
    pub kept_actions: Vec<StatusKeptAction>,
    pub last_decision: String,
    pub rollback_available: bool,
    pub last_rollback_path: Option<String>,
    pub cooldown_remaining_seconds: Option<u64>,
    pub planner: Option<PlannerSummary>,
    pub data_quality: Option<String>,
    pub last_fault: Option<String>,
    pub manual_restore_command: String,
    pub history_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusTarget {
    pub comm: String,
    pub pid: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusKeptAction {
    pub candidate_name: String,
    pub action_id: String,
    pub action_kind: String,
    pub safety_class: String,
    pub kept_unix_nanos: u128,
}

pub fn autotune_status_command(input: AutotuneStatusCommandInput) -> anyhow::Result<()> {
    let history_path = input
        .history_path
        .unwrap_or_else(default_autotune_history_path);
    let status = load_autotune_status(&history_path)?;

    if input.json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        print!("{}", render_autotune_status_text(&status));
    }

    Ok(())
}

pub fn load_autotune_status(history_path: &Path) -> anyhow::Result<AutotuneStatus> {
    let daemon_state_path = daemon_state_path_for_history_path(history_path);
    if daemon_state_path.exists() {
        let state = load_daemon_state(&daemon_state_path).with_context(|| {
            format!(
                "failed to load daemon state snapshot {}",
                daemon_state_path.display()
            )
        })?;
        return Ok(status_from_daemon_state(daemon_state_path, &state));
    }

    let events = read_autotune_history_events(history_path)?;
    Ok(status_from_history_events(
        history_path.to_path_buf(),
        &events,
    ))
}

pub fn status_from_history_events(
    history_path: PathBuf,
    events: &[AutotuneHistoryEvent],
) -> AutotuneStatus {
    let Some(last) = events.last() else {
        return AutotuneStatus {
            phase: "Disabled".to_owned(),
            mode: "Observe".to_owned(),
            target: None,
            focus_group: None,
            current_score: None,
            active_profile: None,
            active_candidate: None,
            kept_actions: Vec::new(),
            last_decision: "no autotune history found".to_owned(),
            rollback_available: false,
            last_rollback_path: None,
            cooldown_remaining_seconds: None,
            data_quality: None,
            last_fault: None,
            manual_restore_command: "stutter autotune restore".to_owned(),
            planner: None,
            history_path,
        };
    };

    AutotuneStatus {
        phase: format_phase(last.phase),
        mode: format_mode(last.mode),
        target: last.target.as_ref().map(status_target_from_identity),
        focus_group: Some(format!("{:?}", last.situation)),
        current_score: Some(last.observation_summary.score_total),
        active_profile: active_profile_from_events(events),
        active_candidate: active_candidate_from_events(events),
        kept_actions: kept_actions_from_events(events),
        last_decision: format_last_decision(last),
        rollback_available: rollback_available_from_events(events),
        last_rollback_path: last_rollback_path_from_events(events),
        cooldown_remaining_seconds: cooldown_remaining_seconds_from_events(events),
        data_quality: Some(last.observation_summary.data_quality.clone()),
        last_fault: last_fault_from_events(events),
        manual_restore_command: "stutter autotune restore".to_owned(),
        planner: last.planner.clone(),
        history_path,
    }
}

pub fn status_from_daemon_state(path: PathBuf, state: &DaemonState) -> AutotuneStatus {
    AutotuneStatus {
        phase: format_daemon_phase(state.phase),
        mode: format_daemon_mode(state.mode),
        target: state
            .active_target
            .as_ref()
            .and_then(status_target_from_daemon_target),
        focus_group: None,
        current_score: state
            .last_decision
            .as_ref()
            .and_then(|decision| decision.score_total),
        active_profile: active_profile_from_daemon_state(state),
        active_candidate: active_candidate_from_daemon_state(state),
        kept_actions: kept_actions_from_daemon_state(state),
        last_decision: last_decision_from_daemon_state(state),
        rollback_available: state
            .active_rollback
            .as_ref()
            .map(|rollback| rollback.rollback_available)
            .unwrap_or(false),
        last_rollback_path: last_rollback_path_from_daemon_state(state),
        cooldown_remaining_seconds: state
            .cooldown_until_unix_nanos
            .map(cooldown_remaining_seconds_from_unix_nanos),
        data_quality: data_quality_from_daemon_state(state),
        last_fault: state.faulted.as_ref().map(|fault| fault.reason.clone()),
        manual_restore_command: manual_restore_command_from_daemon_state(state),
        planner: state.last_decision.as_ref().and_then(|d| d.planner.clone()),
        history_path: path,
    }
}

pub fn render_autotune_status_text(status: &AutotuneStatus) -> String {
    let target = match &status.target {
        Some(target) => format!("{} pid={}", target.comm, target.pid),
        None => "none".to_owned(),
    };

    let mut text = format!(
        "phase: {}\nmode: {}\ntarget: {}\nfocus_group: {}\ncurrent_score: {}\nactive_profile: {}\nactive_candidate: {}\nkept_actions: {}\nlast_decision: {}\nrollback_available: {}\nlast_rollback_path: {}\ncooldown_remaining_seconds: {}\ndata_quality: {}\nlast_fault: {}\nmanual_restore_command: {}\n",
        status.phase,
        status.mode,
        target,
        status.focus_group.as_deref().unwrap_or("none"),
        status
            .current_score
            .map(|score| score.to_string())
            .unwrap_or_else(|| "none".to_owned()),
        status.active_profile.as_deref().unwrap_or("none"),
        status.active_candidate.as_deref().unwrap_or("none"),
        format_kept_actions(&status.kept_actions),
        status.last_decision,
        if status.rollback_available {
            "yes"
        } else {
            "no"
        },
        status.last_rollback_path.as_deref().unwrap_or("none"),
        status
            .cooldown_remaining_seconds
            .map(|seconds| seconds.to_string())
            .unwrap_or_else(|| "none".to_owned()),
        status.data_quality.as_deref().unwrap_or("none"),
        status.last_fault.as_deref().unwrap_or("none"),
        status.manual_restore_command
    );

    append_planner_status_text(&mut text, status.planner.as_ref());

    text
}

fn append_planner_status_text(text: &mut String, planner: Option<&PlannerSummary>) {
    let Some(planner) = planner else {
        text.push_str("planner: none\n");
        return;
    };

    text.push_str(&format!(
        "planner: total={} eligible={}\n",
        planner.total_proposals, planner.eligible_proposals
    ));

    if let Some(selected) = planner.selected.as_ref() {
        text.push_str(&format!(
            "planner_selected: candidate={} action_kind={} objective={:?} confidence={:.3} evidence={}\n",
            selected.candidate_name,
            selected.action_kind,
            selected.objective,
            selected.confidence,
            format_planner_evidence(&selected.evidence)
        ));
    } else {
        text.push_str("planner_selected: none\n");
    }

    if planner.eligible_candidates.is_empty() {
        text.push_str("planner_eligible: none\n");
    } else {
        for eligible in &planner.eligible_candidates {
            append_planner_evaluation_text(text, "planner_eligible", eligible);
        }
    }

    if planner.top_denied_candidates.is_empty() {
        text.push_str("planner_denied: none\n");
    } else {
        for denied in &planner.top_denied_candidates {
            append_planner_evaluation_text(text, "planner_denied", denied);
        }
    }

    if planner.grouped_denials.is_empty() {
        text.push_str("planner_grouped_denials: none\n");
    } else {
        text.push_str(&format!(
            "planner_grouped_denials: {}\n",
            planner
                .grouped_denials
                .iter()
                .map(|denial| format!("{}={}", denial.reason_code, denial.count))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }

    if !planner.missing_capabilities.is_empty() {
        text.push_str(&format!(
            "planner_missing_capabilities: {}\n",
            planner.missing_capabilities.join(",")
        ));
    }

    if !planner.workload_blocked.is_empty() {
        text.push_str(&format!(
            "planner_workload_blocked: {}\n",
            planner.workload_blocked.join(",")
        ));
    }

    if !planner.manual_only_suggestions.is_empty() {
        text.push_str(&format!(
            "planner_manual_only: {}\n",
            planner.manual_only_suggestions.join(",")
        ));
    }

    if let Some(no_action) = planner.no_action.as_ref() {
        text.push_str(&format!(
            "planner_no_action: reason={} total={} eligible={}\n",
            no_action.reason, no_action.total_proposals, no_action.eligible_proposals
        ));
    }
}

fn append_planner_evaluation_text(
    text: &mut String,
    prefix: &str,
    evaluation: &PlannerEvaluationSummary,
) {
    text.push_str(&format!(
        "{prefix}: candidate={} action_kind={} objective={:?} confidence={:.3} eligible={} dry_run_affected_tasks={} reasons={} evidence={}\n",
        evaluation.candidate_name,
        evaluation.action_kind,
        evaluation.objective,
        evaluation.confidence,
        evaluation.eligible,
        evaluation
            .dry_run_affected_tasks
            .map(|affected| affected.to_string())
            .unwrap_or_else(|| "none".to_owned()),
        if evaluation.deny_reason_codes.is_empty() {
            "none".to_owned()
        } else {
            evaluation.deny_reason_codes.join(",")
        },
        format_planner_evidence(&evaluation.evidence)
    ));
}

fn format_planner_evidence(evidence: &[String]) -> String {
    if evidence.is_empty() {
        "none".to_owned()
    } else {
        evidence.join("|")
    }
}

fn daemon_state_path_for_history_path(history_path: &Path) -> PathBuf {
    history_path
        .parent()
        .map(|parent| parent.join("daemon_state.json"))
        .unwrap_or_else(default_daemon_state_snapshot_path)
}

fn format_daemon_phase(phase: DaemonPhase) -> String {
    phase.lifecycle_label().to_owned()
}

fn format_daemon_mode(mode: DaemonMode) -> String {
    format!("{mode:?}")
}

fn status_target_from_daemon_target(target: &DaemonTargetState) -> Option<StatusTarget> {
    let pid = target.root_pid?;
    let comm = target.comm.clone().unwrap_or_else(|| format!("pid-{pid}"));

    Some(StatusTarget { comm, pid })
}

fn active_profile_from_daemon_state(state: &DaemonState) -> Option<String> {
    if matches!(state.phase, DaemonPhase::Keep | DaemonPhase::Cooldown) {
        return state
            .active_experiment
            .as_ref()
            .and_then(|experiment| experiment.candidate_name.clone());
    }

    None
}

fn active_candidate_from_daemon_state(state: &DaemonState) -> Option<String> {
    if matches!(
        state.phase,
        DaemonPhase::Decide | DaemonPhase::Apply | DaemonPhase::Measure
    ) {
        return state
            .active_experiment
            .as_ref()
            .and_then(|experiment| experiment.candidate_name.clone());
    }

    None
}

fn kept_actions_from_daemon_state(state: &DaemonState) -> Vec<StatusKeptAction> {
    state
        .profile_memory
        .sorted_profiles()
        .into_iter()
        .map(|profile| StatusKeptAction {
            candidate_name: profile.candidate_name,
            action_id: profile.action_id,
            action_kind: profile.action_kind,
            safety_class: format!("{:?}", profile.safety_class),
            kept_unix_nanos: profile.kept_unix_nanos,
        })
        .collect()
}

fn last_decision_from_daemon_state(state: &DaemonState) -> String {
    if let Some(decision) = state.last_decision.as_ref() {
        let normalized = normalized_decision(&decision.decision);
        let reason = decision.reason.trim();
        let top_denied_reason = decision
            .top_denied_reason
            .as_deref()
            .filter(|value| !value.trim().is_empty());

        return match (reason.is_empty(), top_denied_reason) {
            (true, Some(top_denied_reason)) => {
                format!("{normalized}: top_denied_reason={top_denied_reason}")
            }
            (true, None) => normalized,
            (false, Some(top_denied_reason)) => {
                format!("{normalized}: {reason}; top_denied_reason={top_denied_reason}")
            }
            (false, None) => format!("{normalized}: {reason}"),
        };
    }

    if let Some(fault) = state.faulted.as_ref() {
        return format!("faulted: {}", fault.reason);
    }

    "daemon_state_snapshot_loaded".to_owned()
}

fn last_rollback_path_from_daemon_state(state: &DaemonState) -> Option<String> {
    state
        .active_rollback
        .as_ref()
        .and_then(|rollback| rollback.token.as_ref())
        .and_then(|token| token.restore_path())
        .map(|path| restore_path_display(path))
}

fn data_quality_from_daemon_state(state: &DaemonState) -> Option<String> {
    state
        .degraded
        .iter()
        .find(|status| status.category == "data_quality")
        .map(|status| status.message.clone())
}

fn manual_restore_command_from_daemon_state(state: &DaemonState) -> String {
    state
        .faulted
        .as_ref()
        .and_then(|fault| fault.manual_restore_command.clone())
        .or_else(|| {
            state
                .active_rollback
                .as_ref()
                .and_then(|rollback| rollback.manual_restore_command.clone())
        })
        .unwrap_or_else(|| "stutter autotune restore".to_owned())
}

fn cooldown_remaining_seconds_from_unix_nanos(cooldown_until: u128) -> u64 {
    let now = crate::audit::unix_nanos_now();

    if now >= cooldown_until {
        return 0;
    }

    let remaining_nanos = cooldown_until.saturating_sub(now);
    let remaining_seconds = remaining_nanos.div_ceil(1_000_000_000);
    remaining_seconds.min(u64::MAX as u128) as u64
}

fn cooldown_remaining_seconds_from_events(events: &[AutotuneHistoryEvent]) -> Option<u64> {
    for event in events.iter().rev() {
        let Some(cooldown_until) =
            cooldown_until_unix_nanos_from_policy(&event.decision.rollback_policy)
        else {
            continue;
        };

        return Some(cooldown_remaining_seconds_from_unix_nanos(cooldown_until));
    }

    None
}

fn cooldown_until_unix_nanos_from_policy(policy: &str) -> Option<u128> {
    for part in policy.split(';') {
        let trimmed = part.trim();
        let Some(value) = trimmed.strip_prefix("cooldown_until_unix_nanos=") else {
            continue;
        };

        if let Ok(parsed) = value.parse::<u128>() {
            return Some(parsed);
        }
    }

    None
}

fn status_target_from_identity(target: &TargetIdentity) -> StatusTarget {
    StatusTarget {
        comm: target.process_comm.clone(),
        pid: target.root_pid,
    }
}

fn active_profile_from_events(events: &[AutotuneHistoryEvent]) -> Option<String> {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if event.rollback_performed || decision == "candidate_reverted" || decision == "restored" {
            return None;
        }

        if decision == "candidate_kept"
            && let Some(candidate_name) = event.decision.candidate_name.as_ref()
        {
            return Some(candidate_name.clone());
        }
    }

    None
}

fn active_candidate_from_events(events: &[AutotuneHistoryEvent]) -> Option<String> {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if matches!(
            decision.as_str(),
            "candidate_reverted" | "candidate_kept" | "restored" | "cooldown_entered"
        ) || event.rollback_performed
        {
            return None;
        }

        if matches!(
            decision.as_str(),
            "candidate_started" | "candidate_applied" | "suggested"
        ) && let Some(candidate_name) = event.decision.candidate_name.as_ref()
        {
            return Some(candidate_name.clone());
        }
    }

    None
}

fn kept_actions_from_events(events: &[AutotuneHistoryEvent]) -> Vec<StatusKeptAction> {
    let mut kept = BTreeMap::<String, StatusKeptAction>::new();

    for event in events {
        let decision = normalized_decision(&event.decision.decision);

        if decision == "restored" {
            kept.clear();
            continue;
        }

        if event.rollback_performed || decision == "candidate_reverted" {
            if let Some(action_id) = event.action_id.as_ref() {
                kept.remove(action_id);
            } else {
                kept.clear();
            }
            continue;
        }

        if decision == "candidate_kept" {
            let action_id = event.action_id.clone().unwrap_or_else(|| {
                event
                    .decision
                    .candidate_name
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned())
            });
            kept.insert(
                action_id.clone(),
                StatusKeptAction {
                    candidate_name: event
                        .decision
                        .candidate_name
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    action_id,
                    action_kind: event
                        .decision
                        .action_kind
                        .clone()
                        .unwrap_or_else(|| "unknown".to_owned()),
                    safety_class: event
                        .decision
                        .safety_class
                        .as_ref()
                        .map(|safety_class| format!("{safety_class:?}"))
                        .unwrap_or_else(|| "unknown".to_owned()),
                    kept_unix_nanos: event.unix_nanos,
                },
            );
        }
    }

    kept.into_values().collect()
}

fn rollback_available_from_events(events: &[AutotuneHistoryEvent]) -> bool {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if event.rollback_performed || decision == "candidate_reverted" || decision == "restored" {
            return false;
        }

        if event.action_id.is_some()
            && matches!(
                decision.as_str(),
                "candidate_applied" | "candidate_kept" | "cooldown_entered"
            )
            && event.decision.rollback_policy.contains("rollback")
        {
            return true;
        }
    }

    false
}

fn last_rollback_path_from_events(events: &[AutotuneHistoryEvent]) -> Option<String> {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if event.rollback_performed || decision == "candidate_reverted" || decision == "restored" {
            return None;
        }

        if event.decision.rollback_policy.contains("rollback") {
            return Some(default_restore_path_display());
        }
    }

    None
}

fn last_fault_from_events(events: &[AutotuneHistoryEvent]) -> Option<String> {
    for event in events.iter().rev() {
        let decision = normalized_decision(&event.decision.decision);

        if decision == "restored" {
            return None;
        }

        if matches!(event.phase, ControllerPhase::Faulted) || decision == "faulted" {
            return Some(event.reason.clone());
        }
    }

    None
}

fn default_restore_path_display() -> String {
    let path = crate::affinity::default_restore_path();
    restore_path_display(&path)
}

fn restore_path_display(path: &Path) -> String {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return path.display().to_string();
    };

    match path.strip_prefix(&home) {
        Ok(stripped) => format!("~/{}", stripped.display()),
        Err(_) => path.display().to_string(),
    }
}

fn format_kept_actions(actions: &[StatusKeptAction]) -> String {
    if actions.is_empty() {
        return "none".to_owned();
    }

    actions
        .iter()
        .map(|action| {
            format!(
                "{} action_id={} action_kind={} safety_class={}",
                action.candidate_name, action.action_id, action.action_kind, action.safety_class
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_last_decision(event: &AutotuneHistoryEvent) -> String {
    let decision = normalized_decision(&event.decision.decision);

    if let Some(improvement) = improvement_percent(event)
        && decision == "candidate_kept"
    {
        return format!("candidate_kept, improvement={:.1}%", improvement);
    }

    if event.rollback_performed {
        return format!("{decision}, rollback performed");
    }

    if !event.reason.trim().is_empty() {
        return format!("{decision}: {}", event.reason);
    }

    decision
}

fn normalized_decision(decision: &str) -> String {
    match decision {
        "Noop" | "noop" | "observed" => "observed".to_owned(),
        "Suggest" | "suggest" | "suggested" => "suggested".to_owned(),
        "StartExperiment" | "candidate_started" => "candidate_started".to_owned(),
        "candidate_applied" => "candidate_applied".to_owned(),
        "KeepCurrent" | "Kept" | "Keep" | "Improved" | "candidate_kept" => {
            "candidate_kept".to_owned()
        }
        "Revert" | "Reverted" | "candidate_reverted" => "candidate_reverted".to_owned(),
        "EnterCooldown" | "Cooldown" | "cooldown" | "cooldown_entered" => {
            "cooldown_entered".to_owned()
        }
        "Fault" | "Faulted" | "EmergencyRestoreFault" | "CrashRecoveryFault" | "faulted" => {
            "faulted".to_owned()
        }
        "EmergencyRestore" | "CrashRecoveryRollback" | "restored" => "restored".to_owned(),
        other => humanize_decision(other).replace(' ', "_"),
    }
}

fn improvement_percent(event: &AutotuneHistoryEvent) -> Option<f64> {
    let before = event.score_before.as_ref()?.score.total;
    let after = event.score_after.as_ref()?.score.total;

    if before == 0 || after >= before {
        return None;
    }

    Some(((before - after) as f64 / before as f64) * 100.0)
}

fn humanize_decision(decision: &str) -> String {
    let mut out = String::new();

    for (idx, ch) in decision.chars().enumerate() {
        if idx > 0 && ch.is_uppercase() {
            out.push(' ');
        }
        out.push(ch.to_ascii_lowercase());
    }

    out
}

fn format_phase(phase: ControllerPhase) -> String {
    format!("{phase:?}")
}

fn format_mode(mode: AutotuneMode) -> String {
    format!("{mode:?}")
}

#[cfg(test)]
mod tests;
