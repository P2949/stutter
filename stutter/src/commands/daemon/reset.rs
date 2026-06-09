use std::fs;

use serde::Serialize;

use crate::daemon::state::{
    DaemonPhase, DaemonState, DaemonStateSnapshotWriter, default_daemon_state_snapshot_path,
};

#[derive(Clone, Debug, Serialize)]
pub struct DaemonResetStateReport {
    pub state_path: String,
    pub dry_run: bool,
    pub state_exists: bool,
    pub backup_path: Option<String>,
    pub reset_state: DaemonState,
}

pub fn run_reset_state_command(
    input: crate::commands::input::DaemonResetStateCommandInput,
) -> anyhow::Result<()> {
    let report = reset_daemon_state(input.dry_run)?;

    if input.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_reset_state_text(&report));
    }

    Ok(())
}

pub fn reset_daemon_state(dry_run: bool) -> anyhow::Result<DaemonResetStateReport> {
    let state_path = default_daemon_state_snapshot_path();
    let state_exists = state_path.exists();
    let backup_path = state_exists
        .then(|| state_path.with_extension(format!("json.bak.{}", crate::audit::unix_nanos_now())));
    let reset_state = safe_reset_daemon_state();

    if !dry_run {
        if let Some(parent) = state_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(backup_path) = backup_path.as_ref() {
            fs::copy(&state_path, backup_path)?;
        }
        DaemonStateSnapshotWriter::new(&state_path).write(&reset_state)?;
    }

    Ok(DaemonResetStateReport {
        state_path: state_path.display().to_string(),
        dry_run,
        state_exists,
        backup_path: backup_path.map(|path| path.display().to_string()),
        reset_state,
    })
}

pub fn safe_reset_daemon_state() -> DaemonState {
    DaemonState {
        mode: crate::daemon::policy::DaemonMode::Observe,
        phase: DaemonPhase::Disabled,
        last_decision: Some(crate::daemon::state::DaemonDecisionState {
            decision: "daemon_state_reset".to_owned(),
            reason: "operator reset daemon state to safe observe-only defaults".to_owned(),
            unix_nanos: Some(crate::audit::unix_nanos_now()),
            diagnostic_current_raw_score_total: None,
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        }),
        ..DaemonState::default()
    }
}

pub fn render_reset_state_text(report: &DaemonResetStateReport) -> String {
    let mut text = String::new();

    text.push_str("Daemon reset-state\n");
    text.push_str("==================\n");
    text.push_str(&format!("dry_run: {}\n", report.dry_run));
    text.push_str(&format!("state_path: {}\n", report.state_path));
    text.push_str(&format!("state_exists: {}\n", report.state_exists));
    text.push_str(&format!(
        "backup_path: {}\n",
        report.backup_path.as_deref().unwrap_or("none")
    ));
    text.push_str(&format!("reset_mode: {}\n", report.reset_state.mode));
    text.push_str(&format!(
        "reset_phase: {}\n",
        report.reset_state.phase.lifecycle_label()
    ));
    if let Some(decision) = report.reset_state.last_decision.as_ref() {
        text.push_str(&format!("reset_decision: {}\n", decision.decision));
        text.push_str(&format!("reset_reason: {}\n", decision.reason));
    }
    if report.dry_run {
        text.push_str("result: no changes written\n");
    } else {
        text.push_str("result: daemon state reset written\n");
    }

    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_reset_state_text_reports_dry_run_backup_and_safe_state() {
        let report = DaemonResetStateReport {
            state_path: "state.json".to_owned(),
            dry_run: true,
            state_exists: true,
            backup_path: Some("state.json.bak".to_owned()),
            reset_state: safe_reset_daemon_state(),
        };

        let text = render_reset_state_text(&report);

        assert!(text.contains("Daemon reset-state"));
        assert!(text.contains("dry_run: true"));
        assert!(text.contains("state_path: state.json"));
        assert!(text.contains("backup_path: state.json.bak"));
        assert!(text.contains("reset_mode: observe"));
        assert!(text.contains("result: no changes written"));
    }

    #[test]
    fn safe_reset_daemon_state_clears_active_state_and_disables_apply() {
        let state = safe_reset_daemon_state();

        assert_eq!(state.mode, crate::daemon::policy::DaemonMode::Observe);
        assert_eq!(state.phase, DaemonPhase::Disabled);
        assert!(state.active_experiment.is_none());
        assert!(state.active_target.is_none());
        assert_eq!(
            state.last_decision.as_ref().unwrap().decision,
            "daemon_state_reset"
        );
    }
}
