use std::path::{Path, PathBuf};

use super::{
    history::improvement_percent,
    model::{AutotuneStatus, StatusKeptAction},
};
use crate::autotune::{
    history::AutotuneHistoryEvent,
    planner::{PlannerEvaluationSummary, PlannerSummary},
};

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
pub(crate) fn default_restore_path_display() -> String {
    let path = crate::affinity::default_restore_path();
    restore_path_display(&path)
}

pub(crate) fn restore_path_display(path: &Path) -> String {
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

pub(crate) fn format_last_decision(event: &AutotuneHistoryEvent) -> String {
    let decision = normalized_decision(&event.decision.decision);

    if let Some(improvement) = improvement_percent(event)
        && decision == "candidate_kept"
    {
        return format!("candidate_kept, improvement={:.1}%", improvement);
    }

    if event.rollback_performed {
        return format!("{decision}, rollback performed");
    }

    if !event.reason.clone().trim().is_empty() {
        return format!("{decision}: {}", event.reason.clone());
    }

    decision
}

pub(crate) fn normalized_decision(decision: &str) -> String {
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
pub(crate) fn humanize_decision(decision: &str) -> String {
    let mut out = String::new();

    for (idx, ch) in decision.chars().enumerate() {
        if idx > 0 && ch.is_uppercase() {
            out.push(' ');
        }
        out.push(ch.to_ascii_lowercase());
    }

    out
}
