use serde::Serialize;

use super::status::configured_system_health_snapshot;
use crate::daemon::{
    capabilities::{CapabilityProbe, DaemonCapabilities},
    health::SystemHealthSnapshot,
    state::{DaemonState, default_daemon_state_snapshot_path, load_daemon_state},
    watchdog::{
        DaemonWatchdogConfig, DaemonWatchdogInputs, DaemonWatchdogReport, evaluate_daemon_watchdog,
    },
};

#[derive(Clone, Debug, Serialize)]
pub struct DaemonDoctorReport {
    pub state_path: String,
    pub state_load_ok: bool,
    pub state_uncertain: bool,
    pub safe_observe_only_required: bool,
    pub manual_restore_command: String,
    pub checks: Vec<DaemonDoctorCheck>,
    pub capabilities: DaemonCapabilities,
    pub current_health: SystemHealthSnapshot,
    pub watchdog: DaemonWatchdogReport,
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonDoctorCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

pub fn run_doctor_command(
    input: crate::commands::input::DaemonDoctorCommandInput,
) -> anyhow::Result<()> {
    let report = build_daemon_doctor_report();

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_daemon_doctor_text(&report));
    }

    Ok(())
}

pub fn build_daemon_doctor_report() -> DaemonDoctorReport {
    let state_path = default_daemon_state_snapshot_path();
    let state_result = load_daemon_state(&state_path);
    let (state_load_ok, state) = match state_result {
        Ok(state) => (true, state),
        Err(_) => (false, DaemonState::default()),
    };
    let current_health = configured_system_health_snapshot();
    let capabilities = CapabilityProbe::default().probe();
    let watchdog = evaluate_daemon_watchdog(
        DaemonWatchdogInputs::from_state_and_health(&state, &current_health),
        &DaemonWatchdogConfig::default(),
    );
    let state_uncertain = !state_load_ok;
    let safe_observe_only_required =
        state_uncertain || !current_health.ok_for_apply || !watchdog.ok;
    let mut checks = Vec::new();

    checks.push(DaemonDoctorCheck {
        name: "state_store_load".to_owned(),
        passed: state_load_ok,
        message: if state_load_ok {
            "daemon state snapshot loaded".to_owned()
        } else {
            "daemon state is missing or corrupt; apply should remain disabled until reset or recovery"
                .to_owned()
        },
    });
    checks.push(DaemonDoctorCheck {
        name: "health_ok_for_apply".to_owned(),
        passed: current_health.ok_for_apply,
        message: current_health
            .reason_code
            .clone()
            .unwrap_or_else(|| "health model permits apply".to_owned()),
    });
    checks.push(DaemonDoctorCheck {
        name: "watchdog_ok".to_owned(),
        passed: watchdog.ok,
        message: if watchdog.ok {
            "watchdog has no active safety issue".to_owned()
        } else {
            watchdog
                .issues
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "watchdog reported an unsafe state".to_owned())
        },
    });
    checks.push(DaemonDoctorCheck {
        name: "rollback_state".to_owned(),
        passed: state.active_rollback.is_none() || state.active_experiment.is_some(),
        message: if state.active_rollback.is_some() && state.active_experiment.is_none() {
            "rollback record exists without an active experiment".to_owned()
        } else {
            "rollback state is clean or intentionally active".to_owned()
        },
    });

    DaemonDoctorReport {
        state_path: state_path.display().to_string(),
        state_load_ok,
        state_uncertain,
        safe_observe_only_required,
        manual_restore_command: "stutter daemon emergency-restore".to_owned(),
        checks,
        capabilities,
        current_health,
        watchdog,
    }
}

pub fn render_daemon_doctor_text(report: &DaemonDoctorReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon doctor\n");
    text.push_str("=============\n");
    text.push_str(&format!("state_path: {}\n", report.state_path));
    text.push_str(&format!("state_load_ok: {}\n", report.state_load_ok));
    text.push_str(&format!("state_uncertain: {}\n", report.state_uncertain));
    text.push_str(&format!(
        "safe_observe_only_required: {}\n",
        report.safe_observe_only_required
    ));
    text.push_str(&format!(
        "manual_restore_command: {}\n",
        report.manual_restore_command
    ));
    text.push_str(&format!(
        "health: {}\n",
        report.current_health.state.as_str()
    ));
    text.push_str(&format!(
        "health_ok_for_apply: {}\n",
        report.current_health.ok_for_apply
    ));
    if let Some(reason) = report.current_health.reason_code.as_ref() {
        text.push_str(&format!("health_reason: {reason}\n"));
    }
    for issue in &report.current_health.issues {
        text.push_str(&format!(
            "health_issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }
    text.push_str(&format!("watchdog_ok: {}\n", report.watchdog.ok));
    if report.watchdog.recommended_actions.is_empty() {
        text.push_str("watchdog_actions: none\n");
    } else {
        text.push_str(&format!(
            "watchdog_actions: {}\n",
            report
                .watchdog
                .recommended_actions
                .iter()
                .map(|action| action.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for issue in &report.watchdog.issues {
        text.push_str(&format!(
            "watchdog_issue: {} - {}\n",
            issue.reason_code, issue.message
        ));
    }

    text.push_str(&format!(
        "kernel_release: {}\n",
        report
            .capabilities
            .kernel_release
            .as_deref()
            .unwrap_or("unknown")
    ));
    let unavailable = report.capabilities.unavailable_features();
    if let Some(reachable) = report.capabilities.privileged_worker_socket_reachable {
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

    text.push_str("checks:\n");
    for check in &report.checks {
        let status = if check.passed { "passed" } else { "failed" };
        text.push_str(&format!(
            "  - {}: {} - {}\n",
            check.name, status, check.message
        ));
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_doctor_text_reports_state_health_watchdog_and_checks() {
        let report = build_daemon_doctor_report();

        let text = render_daemon_doctor_text(&report);

        assert!(text.contains("Daemon doctor"));
        assert!(text.contains("state_path:"));
        assert!(text.contains("state_load_ok:"));
        assert!(text.contains("health:"));
        assert!(text.contains("watchdog_ok:"));
        assert!(text.contains("checks:"));
        assert!(text.contains("state_store_load:"));
    }
}
