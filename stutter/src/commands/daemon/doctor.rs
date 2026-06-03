use std::{fs, path::Path};

use serde::Serialize;

use super::status::configured_system_health_snapshot;
use crate::{
    autotune::controller_journal::{
        ControllerJournalState, default_controller_journal_path, read_controller_journal,
    },
    daemon::{
        capabilities::{CapabilityProbe, DaemonCapabilities},
        health::SystemHealthSnapshot,
        policy::DaemonMode,
        state::{DaemonState, default_daemon_state_snapshot_path, load_daemon_state},
        watchdog::{
            DaemonWatchdogConfig, DaemonWatchdogInputs, DaemonWatchdogReport,
            evaluate_daemon_watchdog,
        },
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
    pub rollback_drill: DaemonRollbackDrillReport,
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonDoctorCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DaemonRollbackDrillReport {
    pub dry_run: bool,
    pub passed: bool,
    pub emergency_restore_command: String,
    pub state_path: String,
    pub controller_journal_path: String,
    pub affinity_restore_path: String,
    pub profile_restore_path: String,
    pub pending_rollback_state: String,
    pub privileged_worker_socket_configured: Option<bool>,
    pub checks: Vec<DaemonDoctorCheck>,
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

pub fn run_rollback_drill_command(
    input: crate::commands::input::DaemonRollbackDrillCommandInput,
) -> anyhow::Result<()> {
    if !input.dry_run {
        anyhow::bail!("daemon rollback-drill requires --dry-run");
    }
    let report = build_daemon_rollback_drill_report(input.dry_run);
    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_daemon_rollback_drill_text(&report));
    }
    if !report.passed {
        anyhow::bail!("daemon rollback drill failed one or more checks");
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
    let rollback_drill =
        build_daemon_rollback_drill_report_with_inputs(true, &state, state_load_ok, &capabilities);
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
        rollback_drill,
    }
}

pub fn build_daemon_rollback_drill_report(dry_run: bool) -> DaemonRollbackDrillReport {
    let state_path = default_daemon_state_snapshot_path();
    let state_result = load_daemon_state(&state_path);
    let (state_load_ok, state) = match state_result {
        Ok(state) => (true, state),
        Err(_) => (false, DaemonState::default()),
    };
    let capabilities = CapabilityProbe::default().probe();
    build_daemon_rollback_drill_report_with_inputs(dry_run, &state, state_load_ok, &capabilities)
}

fn build_daemon_rollback_drill_report_with_inputs(
    dry_run: bool,
    state: &DaemonState,
    state_load_ok: bool,
    capabilities: &DaemonCapabilities,
) -> DaemonRollbackDrillReport {
    let state_path = default_daemon_state_snapshot_path();
    let journal_path = default_controller_journal_path();
    let affinity_restore_path = crate::affinity::default_restore_path();
    let profile_restore_path = crate::profile_restore::default_restore_path();
    let emergency_restore_command = "stutter daemon emergency-restore --dry-run".to_owned();
    let mut checks = Vec::new();

    let state_path_exists = state_path.exists();
    checks.push(DaemonDoctorCheck {
        name: "state_snapshot_readable".to_owned(),
        passed: state_load_ok || !state_path_exists,
        message: if state_load_ok {
            format!("state snapshot readable at {}", state_path.display())
        } else if !state_path_exists {
            format!(
                "state snapshot is not present at {}; no pending daemon state",
                state_path.display()
            )
        } else {
            format!(
                "state snapshot missing or unreadable at {}; observe-only recovery required",
                state_path.display()
            )
        },
    });

    let (journal_readable, journal_message, journal_requires_restore, journal_restorable) =
        match read_controller_journal(&journal_path) {
            Ok(record) => (
                true,
                format!(
                    "controller journal readable at {} state={}",
                    journal_path.display(),
                    record.state.as_str()
                ),
                !matches!(
                    record.state,
                    ControllerJournalState::Clean
                        | ControllerJournalState::Reverted
                        | ControllerJournalState::Planned
                        | ControllerJournalState::Preflighted
                ),
                record.rollback_token.is_some() || record.restore_command.is_some(),
            ),
            Err(err) => (
                false,
                format!(
                    "controller journal unreadable at {}: {err}",
                    journal_path.display()
                ),
                true,
                false,
            ),
        };
    checks.push(DaemonDoctorCheck {
        name: "controller_journal_readable".to_owned(),
        passed: journal_readable,
        message: journal_message,
    });

    checks.push(optional_restore_file_check(
        "affinity_restore_file_readable",
        &affinity_restore_path,
    ));
    checks.push(optional_restore_file_check(
        "profile_restore_file_readable",
        &profile_restore_path,
    ));

    let state_has_rollback = state.active_rollback.is_some();
    let state_rollback_restorable = state.active_rollback.as_ref().is_some_and(|rollback| {
        rollback.rollback_available || rollback.manual_restore_command.is_some()
    });
    let pending_rollback_state = if !state_has_rollback && !journal_requires_restore {
        "empty"
    } else if state_rollback_restorable || journal_restorable {
        "restorable"
    } else {
        "blocked"
    }
    .to_owned();
    checks.push(DaemonDoctorCheck {
        name: "pending_rollback_state".to_owned(),
        passed: pending_rollback_state != "blocked",
        message: format!("pending rollback state is {pending_rollback_state}"),
    });

    checks.push(DaemonDoctorCheck {
        name: "emergency_restore_command_path".to_owned(),
        passed: true,
        message: format!("dry-run command: {emergency_restore_command}"),
    });

    let privileged_worker_socket_configured = capabilities.privileged_worker_socket_reachable;
    let privileged_worker_ok = state.mode != DaemonMode::ApplyMediumRisk
        || privileged_worker_socket_configured == Some(true);
    checks.push(DaemonDoctorCheck {
        name: "privileged_worker_socket".to_owned(),
        passed: privileged_worker_ok,
        message: if state.mode == DaemonMode::ApplyMediumRisk {
            format!(
                "apply-medium-risk rollback drill requires privileged worker socket; reachable={}",
                privileged_worker_socket_configured
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_owned())
            )
        } else {
            "privileged worker socket is not required for the current low-risk/observe drill"
                .to_owned()
        },
    });

    let passed = checks.iter().all(|check| check.passed);
    DaemonRollbackDrillReport {
        dry_run,
        passed,
        emergency_restore_command,
        state_path: state_path.display().to_string(),
        controller_journal_path: journal_path.display().to_string(),
        affinity_restore_path: affinity_restore_path.display().to_string(),
        profile_restore_path: profile_restore_path.display().to_string(),
        pending_rollback_state,
        privileged_worker_socket_configured,
        checks,
    }
}

fn optional_restore_file_check(name: &str, path: &Path) -> DaemonDoctorCheck {
    if !path.exists() {
        return DaemonDoctorCheck {
            name: name.to_owned(),
            passed: true,
            message: format!("{} is not present; nothing to restore", path.display()),
        };
    }

    match fs::read(path) {
        Ok(_) => DaemonDoctorCheck {
            name: name.to_owned(),
            passed: true,
            message: format!("{} is readable", path.display()),
        },
        Err(err) => DaemonDoctorCheck {
            name: name.to_owned(),
            passed: false,
            message: format!("{} is not readable: {err}", path.display()),
        },
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
        "rollback_drill: passed={} pending={} command=\"{}\"\n",
        report.rollback_drill.passed,
        report.rollback_drill.pending_rollback_state,
        report.rollback_drill.emergency_restore_command
    ));

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

pub fn render_daemon_rollback_drill_text(report: &DaemonRollbackDrillReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon rollback drill\n");
    text.push_str("=====================\n");
    text.push_str(&format!("dry_run: {}\n", report.dry_run));
    text.push_str(&format!("passed: {}\n", report.passed));
    text.push_str(&format!(
        "emergency_restore_command: {}\n",
        report.emergency_restore_command
    ));
    text.push_str(&format!(
        "pending_rollback_state: {}\n",
        report.pending_rollback_state
    ));
    text.push_str(&format!("state_path: {}\n", report.state_path));
    text.push_str(&format!(
        "controller_journal_path: {}\n",
        report.controller_journal_path
    ));
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
        assert!(text.contains("rollback_drill:"));
        assert!(text.contains("checks:"));
        assert!(text.contains("state_store_load:"));
    }

    #[test]
    fn rollback_drill_text_reports_dry_run_command_and_checks() {
        let report = build_daemon_rollback_drill_report(true);

        let text = render_daemon_rollback_drill_text(&report);

        assert!(text.contains("Daemon rollback drill"));
        assert!(text.contains("dry_run: true"));
        assert!(
            text.contains("emergency_restore_command: stutter daemon emergency-restore --dry-run")
        );
        assert!(text.contains("controller_journal_readable"));
        assert!(text.contains("pending_rollback_state:"));
    }
}
