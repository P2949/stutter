use serde::Serialize;

use super::helpers::load_recent_daemon_decisions;
use crate::{
    config_file,
    daemon::{
        capabilities::{CapabilityProbe, DaemonCapabilities},
        health::{SystemHealthMonitor, SystemHealthProbeRoot, SystemHealthSnapshot},
        state::{DaemonState, default_daemon_state_snapshot_path, load_daemon_state},
        watchdog::{
            DaemonWatchdogConfig, DaemonWatchdogInputs, DaemonWatchdogReport,
            evaluate_daemon_watchdog,
        },
    },
};

#[derive(Clone, Debug, Serialize)]
pub struct DaemonStatusOutput {
    pub state_path: String,
    pub state_loaded: bool,
    pub state: DaemonState,
    pub capabilities: DaemonCapabilities,
    pub current_health: SystemHealthSnapshot,
    pub watchdog: DaemonWatchdogReport,
    pub manual_restore_command: String,
    pub recent_decisions: Vec<DaemonRecentDecision>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DaemonRecentDecision {
    pub unix_nanos: u128,
    pub phase: String,
    pub mode: String,
    pub decision: String,
    pub candidate_name: Option<String>,
    pub action_id: Option<String>,
    pub rollback_performed: bool,
    pub reason: String,
}

pub fn run_status_command(
    input: crate::commands::input::DaemonStatusCommandInput,
) -> anyhow::Result<()> {
    let output = build_status_output_with_recent_decisions(input.explain_last);

    if input.json {
        println!("{}", render_status_json(&output)?);
    } else {
        print!("{}", render_status_text(&output));
    }

    Ok(())
}

pub fn build_status_output_with_recent_decisions(
    recent_decision_limit: usize,
) -> DaemonStatusOutput {
    let state_path = default_daemon_state_snapshot_path();
    let (state_loaded, state) = match load_daemon_state(&state_path) {
        Ok(state) => (true, state),
        Err(err) => {
            log::debug!(
                "daemon_status_state_load_failed path={} err={err:#}",
                state_path.display()
            );
            (false, DaemonState::default())
        }
    };

    let current_health = configured_system_health_snapshot();
    let watchdog = evaluate_daemon_watchdog(
        DaemonWatchdogInputs::from_state_and_health(&state, &current_health),
        &DaemonWatchdogConfig::default(),
    );

    DaemonStatusOutput {
        state_path: state_path.display().to_string(),
        state_loaded,
        state,
        capabilities: CapabilityProbe::default().probe(),
        current_health,
        watchdog,
        manual_restore_command: "stutter daemon emergency-restore".to_owned(),
        recent_decisions: load_recent_daemon_decisions(recent_decision_limit),
    }
}

pub fn configured_system_health_snapshot() -> SystemHealthSnapshot {
    system_health_snapshot_from_user_config_result(
        config_file::load_user_config(),
        SystemHealthProbeRoot::default(),
    )
}

pub fn system_health_snapshot_from_user_config_result(
    user_config: anyhow::Result<Option<config_file::UserConfigFile>>,
    root: SystemHealthProbeRoot,
) -> SystemHealthSnapshot {
    match user_config {
        Ok(user_config) => {
            match system_health_monitor_from_user_config_with_root(
                user_config.as_ref(),
                root.clone(),
            ) {
                Ok(monitor) => monitor.probe(),
                Err(err) => system_health_snapshot_with_config_error(root, err),
            }
        }
        Err(err) => system_health_snapshot_with_config_error(root, err),
    }
}

pub fn system_health_snapshot_with_config_error(
    root: SystemHealthProbeRoot,
    err: anyhow::Error,
) -> SystemHealthSnapshot {
    log::warn!("daemon_health_config_load_failed err={err:#}; blocking apply");

    let monitor = SystemHealthMonitor::new(root, Default::default());
    let mut inputs = monitor.probe_inputs();
    inputs
        .probe_errors
        .push(format!("daemon_config_load_failed: {err:#}"));
    monitor.evaluate(inputs)
}

pub fn system_health_monitor_from_user_config_with_root(
    user_config: Option<&config_file::UserConfigFile>,
    root: SystemHealthProbeRoot,
) -> anyhow::Result<SystemHealthMonitor> {
    let thresholds = config_file::daemon_health_thresholds_from_user_config(
        user_config,
        None,
        crate::daemon::policy::ActionSource::Cli,
    )?;
    Ok(SystemHealthMonitor::new(root, thresholds))
}

pub fn render_status_json(output: &DaemonStatusOutput) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(output)?)
}

pub fn render_status_text(output: &DaemonStatusOutput) -> String {
    let mut text = String::new();

    text.push_str("Daemon status\n");
    text.push_str("=============\n");
    text.push_str(&format!("state_loaded: {}\n", output.state_loaded));
    text.push_str(&format!("state_path: {}\n", output.state_path));
    text.push_str(&format!("mode: {}\n", output.state.mode));
    text.push_str(&format!(
        "phase: {}\n",
        output.state.phase.lifecycle_label()
    ));
    text.push_str(&format!(
        "manual_restore_command: {}\n",
        output.manual_restore_command
    ));
    if let Some(target) = output.state.active_target.as_ref() {
        text.push_str(&format!(
            "active_workload: root_pid={} comm={} targets={}\n",
            target
                .root_pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            target.comm.as_deref().unwrap_or("unknown"),
            target.active_targets
        ));
    } else {
        text.push_str("active_workload: none\n");
    }
    if let Some(experiment) = output.state.active_experiment.as_ref() {
        text.push_str(&format!(
            "active_action: action_id={} candidate={} mode={} safety_class={:?}\n",
            experiment.action_id,
            experiment.candidate_name.as_deref().unwrap_or("unknown"),
            experiment.mode,
            experiment.safety_class
        ));
    } else {
        text.push_str("active_action: none\n");
    }
    if let Some(rollback) = output.state.active_rollback.as_ref() {
        text.push_str(&format!(
            "rollback_status: action_id={} mode={} safety_class={:?} available={} manual_restore_command={}\n",
            rollback.action_id,
            rollback.mode,
            rollback.safety_class,
            rollback.rollback_available,
            rollback
                .manual_restore_command
                .as_deref()
                .unwrap_or("unknown")
        ));
    } else {
        text.push_str("rollback_status: none\n");
    }
    if let Some(cooldown_until) = output.state.cooldown_until_unix_nanos {
        text.push_str(&format!("cooldown_until_unix_nanos: {cooldown_until}\n"));
    }
    text.push_str(&format!(
        "health: {}\n",
        output.current_health.state.as_str()
    ));
    text.push_str(&format!(
        "health_ok_for_apply: {}\n",
        output.current_health.ok_for_apply
    ));
    if let Some(reason) = output.current_health.reason_code.as_ref() {
        text.push_str(&format!("health_reason: {reason}\n"));
    }
    for issue in &output.current_health.issues {
        text.push_str(&format!(
            "health_issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }
    text.push_str(&format!("watchdog_ok: {}\n", output.watchdog.ok));
    if output.watchdog.recommended_actions.is_empty() {
        text.push_str("watchdog_actions: none\n");
    } else {
        text.push_str(&format!(
            "watchdog_actions: {}\n",
            output
                .watchdog
                .recommended_actions
                .iter()
                .map(|action| action.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for issue in &output.watchdog.issues {
        text.push_str(&format!(
            "watchdog_issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }
    if let Some(decision) = output.state.last_decision.as_ref() {
        text.push_str(&format!("last_decision: {}\n", decision.decision));
        text.push_str(&format!("last_reason: {}\n", decision.reason));
        if let Some(score) = decision.score_total {
            text.push_str(&format!("current_score: {score}\n"));
        }
        super::explain::append_daemon_planner_text(&mut text, decision.planner.as_ref());
    }
    if let Some(fault) = output.state.faulted.as_ref() {
        text.push_str(&format!("fault: {}\n", fault.reason));
    }
    if output.state.phase == crate::daemon::state::DaemonPhase::Paused {
        text.push_str("pause_state: operator_paused\n");
    }
    let unavailable = output.capabilities.unavailable_features();
    if let Some(reachable) = output.capabilities.privileged_worker_socket_reachable {
        text.push_str(&format!(
            "privileged_worker_socket_reachable: {reachable}\n"
        ));
    }
    if unavailable.is_empty() {
        text.push_str("unavailable_features: none\n");
    } else {
        text.push_str(&format!(
            "unavailable_features: {}\n",
            unavailable.join(", ")
        ));
    }

    if output.recent_decisions.is_empty() {
        text.push_str("recent_decisions: none\n");
    } else {
        text.push_str("recent_decisions:\n");
        for decision in &output.recent_decisions {
            text.push_str(&format!(
                "  - unix_nanos={} mode={} phase={} decision={} action={} candidate={} rollback_performed={} reason={}\n",
                decision.unix_nanos,
                decision.mode,
                decision.phase,
                decision.decision,
                decision.action_id.as_deref().unwrap_or("none"),
                decision.candidate_name.as_deref().unwrap_or("none"),
                decision.rollback_performed,
                decision.reason
            ));
        }
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::planner::PlannerSummary;

    #[test]
    fn daemon_status_health_monitor_uses_configured_guardrails() {
        let user_config = config_file::UserConfigFile {
            daemon_max_cpu_temp_celsius: Some(77),
            daemon_max_gpu_temp_celsius: Some(78),
            ..Default::default()
        };

        let monitor = system_health_monitor_from_user_config_with_root(
            Some(&user_config),
            Default::default(),
        )
        .unwrap();

        assert_eq!(monitor.thresholds().max_cpu_temp_millidegrees, 77_000);
        assert_eq!(monitor.thresholds().max_gpu_temp_millidegrees, 78_000);
    }

    #[test]
    fn daemon_status_health_blocks_apply_when_config_is_invalid() {
        let user_config = config_file::UserConfigFile {
            daemon_preset: Some("invalid-preset".to_owned()),
            ..Default::default()
        };

        let snapshot = system_health_snapshot_from_user_config_result(
            config_file::daemon_config_from_user_config(
                Some(&user_config),
                None,
                crate::daemon::policy::ActionSource::Cli,
            )
            .map(|_| Some(user_config)),
            Default::default(),
        );

        println!("ISSUES: {:#?}", snapshot.issues);
        assert!(!snapshot.ok_for_apply);
        assert!(
            snapshot
                .inputs
                .probe_errors
                .iter()
                .any(|err| { err.contains("daemon_config_load_failed") && err.contains("preset") })
        );
    }

    #[test]
    fn daemon_status_text_contains_state_and_restore_command() {
        let output = build_status_output_with_recent_decisions(0);

        let text = render_status_text(&output);

        assert!(text.contains("Daemon status"));
        assert!(text.contains("state_loaded:"));
        assert!(text.contains("mode: observe"));
        assert!(text.contains("manual_restore_command: stutter daemon emergency-restore"));
    }

    #[test]
    fn daemon_status_text_contains_active_workload_action_score_and_recent_decisions() {
        let mut output = build_status_output_with_recent_decisions(0);
        output.state.active_target = Some(crate::daemon::state::DaemonTargetState {
            root_pid: Some(1234),
            comm: Some("game".to_owned()),
            active_targets: 1,
        });
        output.state.active_experiment = Some(crate::daemon::state::DaemonExperimentState {
            experiment_id: "test-experiment".to_owned(),
            action_id: "cpu-affinity:game".to_owned(),
            candidate_name: Some("game-main".to_owned()),
            mode: crate::daemon::policy::DaemonMode::ApplyLowRisk,
            safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
            started_unix_nanos: Some(1_000),
        });
        output.state.last_decision = Some(crate::daemon::state::DaemonDecisionState {
            decision: "optimize".to_owned(),
            reason: "found better affinity".to_owned(),
            unix_nanos: Some(1_000),
            score_total: Some(850),
            candidate_count: None,
            top_denied_reason: None,
            planner: Some(PlannerSummary {
                total_proposals: 10,
                eligible_proposals: 2,
                selected: None,
                eligible_candidates: Vec::new(),
                top_denied_candidates: Vec::new(),
                missing_capabilities: Vec::new(),
                workload_blocked: Vec::new(),
                grouped_denials: Vec::new(),
                manual_only_suggestions: Vec::new(),
                no_action: None,
            }),
            situation: None,
            focus_kind: None,
        });
        output.recent_decisions = vec![DaemonRecentDecision {
            unix_nanos: 1_000,
            phase: "Optimize".to_owned(),
            mode: "ApplyLowRisk".to_owned(),
            decision: "optimize".to_owned(),
            candidate_name: Some("game-main".to_owned()),
            action_id: Some("cpu-affinity:game".to_owned()),
            rollback_performed: false,
            reason: "found better affinity".to_owned(),
        }];

        let text = render_status_text(&output);
        println!("TEXT:\n{text}");

        assert!(text.contains("active_workload: root_pid=1234 comm=game targets=1"));
        assert!(text.contains("active_action: action_id=cpu-affinity:game candidate=game-main"));
        assert!(text.contains("current_score: 850"));
        assert!(text.contains("recent_decisions:"));
        assert!(
            text.contains(
                "decision=optimize action=cpu-affinity:game candidate=game-main rollback_performed=false reason=found better affinity"
            )
        );
    }

    #[test]
    fn daemon_status_json_contains_state_capabilities_and_manual_restore() {
        let output = build_status_output_with_recent_decisions(0);

        let json = render_status_json(&output).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["state"]["mode"], "observe");
        assert!(value["capabilities"]["kernel_release"].is_string());
        assert_eq!(
            value["manual_restore_command"],
            "stutter daemon emergency-restore"
        );
    }
}
