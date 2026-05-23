use serde::Serialize;

use super::{
    helpers::build_policy_from_daemon_status,
    status::{DaemonStatusOutput, build_status_output_with_recent_decisions},
};
use crate::{
    autotune::planner::{PlannerEvaluationSummary, PlannerSummary},
    daemon::{
        explain::{
            DaemonPolicyExplanation, DaemonStatusExplanation, policy_context_from_daemon_status,
        },
        health::SystemHealthSnapshot,
        policy::DaemonPolicy,
        state::DaemonPhase,
        watchdog::DaemonWatchdogReport,
    },
};

#[derive(Clone, Debug, Serialize)]
pub struct DaemonExplainOutput {
    pub status: DaemonStatusOutput,
    pub policy: DaemonPolicy,
    pub policy_explanation: DaemonPolicyExplanation,
    pub status_explanation: DaemonStatusExplanation,
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonWhyNotOptimizeOutput {
    pub state_path: String,
    pub state_loaded: bool,
    pub mode: crate::daemon::policy::DaemonMode,
    pub phase: DaemonPhase,
    pub health: SystemHealthSnapshot,
    pub watchdog: DaemonWatchdogReport,
    pub why_no_optimize: Vec<String>,
    pub planner: Option<PlannerSummary>,
    pub recent_decisions: Vec<super::status::DaemonRecentDecision>,
    pub manual_restore_command: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonWhatChangedOutput {
    pub state_path: String,
    pub state_loaded: bool,
    pub mode: crate::daemon::policy::DaemonMode,
    pub phase: DaemonPhase,
    pub health: SystemHealthSnapshot,
    pub watchdog: DaemonWatchdogReport,
    pub what_changed: Vec<String>,
    pub recent_decisions: Vec<super::status::DaemonRecentDecision>,
    pub manual_restore_command: String,
}

pub fn run_explain_command(
    input: crate::commands::input::DaemonExplainCommandInput,
) -> anyhow::Result<()> {
    let output = build_explain_output(input.explain_last);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_explain_text(&output));
    }

    Ok(())
}

pub fn run_why_not_optimize_command(
    input: crate::commands::input::DaemonWhyNotOptimizeCommandInput,
) -> anyhow::Result<()> {
    let output = build_why_not_optimize_output(input.explain_last);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_why_not_optimize_text(&output));
    }

    Ok(())
}

pub fn run_what_changed_command(
    input: crate::commands::input::DaemonWhatChangedCommandInput,
) -> anyhow::Result<()> {
    let output = build_what_changed_output(input.explain_last);

    if input.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_what_changed_text(&output));
    }

    Ok(())
}

pub fn build_explain_output(recent_decision_limit: usize) -> DaemonExplainOutput {
    let status = build_status_output_with_recent_decisions(recent_decision_limit);
    let policy = build_policy_from_daemon_status(&status);
    let policy_context = policy_context_from_daemon_status(
        &status.state,
        &status.current_health,
        &status.capabilities,
    );
    let policy_explanation =
        DaemonPolicyExplanation::from_policy_with_context(&policy, &policy_context);
    let status_explanation = DaemonStatusExplanation::from_state_health_watchdog(
        &status.state,
        &status.current_health,
        &status.watchdog,
    );

    DaemonExplainOutput {
        status,
        policy,
        policy_explanation,
        status_explanation,
    }
}

pub fn build_why_not_optimize_output(recent_decision_limit: usize) -> DaemonWhyNotOptimizeOutput {
    let explain = build_explain_output(recent_decision_limit);
    why_not_optimize_output_from_explain(explain)
}

pub fn why_not_optimize_output_from_explain(
    explain: DaemonExplainOutput,
) -> DaemonWhyNotOptimizeOutput {
    DaemonWhyNotOptimizeOutput {
        state_path: explain.status.state_path.clone(),
        state_loaded: explain.status.state_loaded,
        mode: explain.status.state.mode,
        phase: explain.status.state.phase,
        health: explain.status.current_health.clone(),
        watchdog: explain.status.watchdog.clone(),
        why_no_optimize: explain.status_explanation.why_no_optimize.clone(),
        planner: explain
            .status
            .state
            .last_decision
            .as_ref()
            .and_then(|decision| decision.planner.clone()),
        recent_decisions: explain.status.recent_decisions.clone(),
        manual_restore_command: explain.status.manual_restore_command.clone(),
    }
}

pub fn build_what_changed_output(recent_decision_limit: usize) -> DaemonWhatChangedOutput {
    let explain = build_explain_output(recent_decision_limit);
    what_changed_output_from_explain(explain)
}

pub fn what_changed_output_from_explain(explain: DaemonExplainOutput) -> DaemonWhatChangedOutput {
    DaemonWhatChangedOutput {
        state_path: explain.status.state_path.clone(),
        state_loaded: explain.status.state_loaded,
        mode: explain.status.state.mode,
        phase: explain.status.state.phase,
        health: explain.status.current_health.clone(),
        watchdog: explain.status.watchdog.clone(),
        what_changed: explain.status_explanation.what_changed.clone(),
        recent_decisions: explain.status.recent_decisions.clone(),
        manual_restore_command: explain.status.manual_restore_command.clone(),
    }
}

pub fn render_explain_text(output: &DaemonExplainOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon explain\n");
    text.push_str("==============\n");
    text.push_str(&format!("state_loaded: {}\n", output.status.state_loaded));
    text.push_str(&format!("state_path: {}\n", output.status.state_path));
    text.push_str(&format!("mode: {}\n", output.status.state.mode));
    text.push_str(&format!(
        "phase: {}\n",
        output.status.state.phase.lifecycle_label()
    ));
    text.push_str(&format!(
        "health: {}\n",
        output.status.current_health.state.as_str()
    ));
    text.push_str(&format!("watchdog_ok: {}\n", output.status.watchdog.ok));
    text.push_str(&format!(
        "manual_restore_command: {}\n",
        output.status.manual_restore_command
    ));

    text.push_str("\nWhy no optimize\n");
    text.push_str("----------------\n");
    for reason in &output.status_explanation.why_no_optimize {
        text.push_str(&format!("- {reason}\n"));
    }

    text.push_str("\nWhat changed\n");
    text.push_str("------------\n");
    for change in &output.status_explanation.what_changed {
        text.push_str(&format!("- {change}\n"));
    }

    text.push_str("\nPolicy decisions\n");
    text.push_str("----------------\n");
    for line in &output.policy_explanation.lines {
        text.push_str(&format!(
            "- {}: {} - {}\n",
            line.rule, line.outcome, line.reason
        ));
    }

    if !output.status.recent_decisions.is_empty() {
        text.push_str("\nRecent decisions\n");
        text.push_str("----------------\n");
        for decision in &output.status.recent_decisions {
            text.push_str(&format!(
                "- {} candidate={} action={} rollback={} reason={}\n",
                decision.decision,
                decision.candidate_name.as_deref().unwrap_or("none"),
                decision.action_id.as_deref().unwrap_or("none"),
                decision.rollback_performed,
                decision.reason
            ));
        }
    }

    text
}

pub fn render_why_not_optimize_text(output: &DaemonWhyNotOptimizeOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon why-not-optimize\n");
    text.push_str("=======================\n");
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("mode: {}\n", output.mode));
    text.push_str(&format!("phase: {}\n", output.phase.lifecycle_label()));
    text.push_str(&format!("health: {}\n", output.health.state.as_str()));
    text.push_str(&format!("watchdog_ok: {}\n", output.watchdog.ok));
    text.push_str(&format!(
        "manual_restore_command: {}\n",
        output.manual_restore_command
    ));
    text.push_str("reasons:\n");
    for reason in &output.why_no_optimize {
        text.push_str(&format!("- {reason}\n"));
    }

    append_daemon_planner_text(&mut text, output.planner.as_ref());

    if !output.recent_decisions.is_empty() {
        text.push_str("recent_decisions:\n");
        for decision in &output.recent_decisions {
            text.push_str(&format!(
                "- {} reason={}\n",
                decision.decision, decision.reason
            ));
        }
    }

    text
}

pub fn append_daemon_planner_text(text: &mut String, planner: Option<&PlannerSummary>) {
    let Some(planner) = planner else {
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
            format_daemon_planner_evidence(&selected.evidence)
        ));
    } else {
        text.push_str("planner_selected: none\n");
    }

    if planner.eligible_candidates.is_empty() {
        text.push_str("planner_eligible: none\n");
    } else {
        for candidate in &planner.eligible_candidates {
            append_daemon_planner_candidate_text(text, "planner_eligible", candidate);
        }
    }

    if planner.top_denied_candidates.is_empty() {
        text.push_str("planner_denied: none\n");
    } else {
        for candidate in &planner.top_denied_candidates {
            append_daemon_planner_candidate_text(text, "planner_denied", candidate);
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

pub fn append_daemon_planner_candidate_text(
    text: &mut String,
    prefix: &str,
    candidate: &PlannerEvaluationSummary,
) {
    text.push_str(&format!(
        "{prefix}: candidate={} action_kind={} objective={:?} confidence={:.3} eligible={} reasons={} evidence={}\n",
        candidate.candidate_name,
        candidate.action_kind,
        candidate.objective,
        candidate.confidence,
        candidate.eligible,
        if candidate.deny_reason_codes.is_empty() {
            "none".to_owned()
        } else {
            candidate.deny_reason_codes.join(",")
        },
        format_daemon_planner_evidence(&candidate.evidence)
    ));
}

pub fn format_daemon_planner_evidence(evidence: &[String]) -> String {
    if evidence.is_empty() {
        "none".to_owned()
    } else {
        evidence.join("|")
    }
}

pub fn render_what_changed_text(output: &DaemonWhatChangedOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon what-changed\n");
    text.push_str("===================\n");
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("mode: {}\n", output.mode));
    text.push_str(&format!("phase: {}\n", output.phase.lifecycle_label()));
    text.push_str(&format!("health: {}\n", output.health.state.as_str()));
    text.push_str(&format!("watchdog_ok: {}\n", output.watchdog.ok));
    text.push_str("changes:\n");
    for change in &output.what_changed {
        text.push_str(&format!("- {change}\n"));
    }

    if !output.recent_decisions.is_empty() {
        text.push_str("recent_decisions:\n");
        for decision in &output.recent_decisions {
            text.push_str(&format!(
                "- {} reason={}\n",
                decision.decision, decision.reason
            ));
        }
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::planner::PlannerSelectedSummary;

    #[test]
    fn daemon_explain_text_contains_why_and_change_sections() {
        let output = build_explain_output(0);

        let text = render_explain_text(&output);

        assert!(text.contains("Daemon explain"));
        assert!(text.contains("Why no optimize"));
        assert!(text.contains("What changed"));
        assert!(text.contains("Policy decisions"));
    }

    #[test]
    fn daemon_why_and_what_changed_commands_render_focused_outputs() {
        let output = build_explain_output(0);

        let why = why_not_optimize_output_from_explain(output.clone());
        let what = what_changed_output_from_explain(output);

        let why_text = render_why_not_optimize_text(&why);
        let what_text = render_what_changed_text(&what);

        assert!(why_text.contains("Daemon why-not-optimize"));
        assert!(why_text.contains("reasons:"));
        assert!(what_text.contains("Daemon what-changed"));
        assert!(what_text.contains("changes:"));
    }

    #[test]
    fn daemon_status_and_why_text_render_planner_summary() {
        let mut output = build_status_output_with_recent_decisions(0);
        output.state.last_decision = Some(crate::daemon::state::DaemonDecisionState {
            decision: "optimize".to_owned(),
            reason: "found better affinity".to_owned(),
            planner: Some(PlannerSummary {
                total_proposals: 10,
                eligible_proposals: 2,
                selected: Some(PlannerSelectedSummary {
                    candidate_name: "game-main".to_owned(),
                    action_kind: "cpu_affinity_profile".to_owned(),
                    objective: crate::autotune::objective::ObjectiveKind::CompileThroughputWithForegroundProtection,
                    safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
                    confidence: 0.95,
                    rank: Some(1),
                    evidence: vec!["high_throughput".to_owned()],
                }),
                eligible_candidates: Vec::new(),
                top_denied_candidates: vec![PlannerEvaluationSummary {
                    candidate_name: "risky-one".to_owned(),
                    action_kind: "sysctl".to_owned(),
                    provider: "sysctl".to_owned(),
                    objective: crate::autotune::objective::ObjectiveKind::StutterScore,
                    safety_class: crate::actions::SafetyClass::HighRisk,
                    effect_scope: crate::daemon::policy::ActionEffectScope::SystemWide,
                    confidence: 0.0,
                    eligible: false,
                    rank: None,
                    deny_reasons: Vec::new(),
                    deny_reason_codes: vec!["high_risk".to_owned()],
                    deny_messages: Vec::new(),
                    dry_run_affected_tasks: None,
                    manual_only_reason: None,
                    evidence: Vec::new(),
                }],
                missing_capabilities: vec!["perf_event".to_owned()],
                workload_blocked: vec!["systemd".to_owned()],
                grouped_denials: vec![crate::autotune::planner::PlannerDenySummary {
                    reason: crate::autotune::planner::CandidateDenyReason::SafetyClassTooHigh,
                    reason_code: "high_risk".to_owned(),
                    count: 1,
                }],
                manual_only_suggestions: vec!["reboot".to_owned()],
                no_action: None,
            }),
            unix_nanos: Some(1),
            diagnostic_current_raw_score_total: Some(900),
            candidate_count: Some(10),
            top_denied_reason: Some("high_risk".to_owned()),
            situation: Some("GameFocused".to_owned()),
            focus_kind: Some("Game".to_owned()),
        });

        let text = crate::commands::daemon::status::render_status_text(&output);

        assert!(text.contains("planner: total=10 eligible=2"));
        assert!(text.contains(
            "planner_selected: candidate=game-main action_kind=cpu_affinity_profile objective=CompileThroughputWithForegroundProtection confidence=0.950 evidence=high_throughput"
        ));
        assert!(text.contains(
            "planner_denied: candidate=risky-one action_kind=sysctl objective=StutterScore confidence=0.000 eligible=false reasons=high_risk evidence=none"
        ));
        assert!(text.contains("planner_missing_capabilities: perf_event"));
        assert!(text.contains("planner_workload_blocked: systemd"));
        assert!(text.contains("planner_grouped_denials: high_risk=1"));
        assert!(text.contains("planner_manual_only: reboot"));
    }
}
