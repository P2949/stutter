use std::{thread, time::Duration};

use super::status::{
    DaemonStatusOutput, build_status_output_with_recent_decisions, render_status_text,
};
use crate::daemon::{DaemonPhase, default_daemon_state_snapshot_path};

#[derive(Clone, Debug)]
pub struct DaemonWatchSignature {
    pub phase: DaemonPhase,
    pub active_action_id: Option<String>,
    pub rollback_action_id: Option<String>,
    pub rollback_available: bool,
    pub fault_reason: Option<String>,
}

pub fn run_watch_command(
    input: crate::commands::input::DaemonWatchCommandInput,
) -> anyhow::Result<()> {
    let iterations = input.iterations.unwrap_or(u64::MAX);
    let mut previous = None;

    for index in 0..iterations {
        let output = build_status_output_with_recent_decisions(input.explain_last);
        let signature = DaemonWatchSignature::from_output(&output);

        if index == 0 || input.verbose {
            print!("{}", render_watch_line(&output));
        }
        if let Some(notification) = previous
            .as_ref()
            .and_then(|old| render_watch_notification(old, &signature))
        {
            println!("notification: {notification}");
        }
        if input.verbose {
            print!("{}", render_status_text(&output));
        }

        previous = Some(signature);
        if index + 1 < iterations {
            thread::sleep(Duration::from_millis(input.interval_ms));
        }
    }

    Ok(())
}

impl DaemonWatchSignature {
    pub fn from_output(output: &DaemonStatusOutput) -> Self {
        Self {
            phase: output.state.phase,
            active_action_id: output
                .state
                .active_experiment
                .as_ref()
                .map(|experiment| experiment.action_id.clone()),
            rollback_action_id: output
                .state
                .active_rollback
                .as_ref()
                .map(|rollback| rollback.action_id.clone()),
            rollback_available: output
                .state
                .active_rollback
                .as_ref()
                .is_some_and(|rollback| rollback.rollback_available),
            fault_reason: output
                .state
                .faulted
                .as_ref()
                .map(|fault| fault.reason.clone()),
        }
    }
}

pub fn render_watch_line(output: &DaemonStatusOutput) -> String {
    let workload = output
        .state
        .active_target
        .as_ref()
        .map(|target| {
            format!(
                "{}:{}",
                target.comm.as_deref().unwrap_or("unknown"),
                target
                    .root_pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "unknown".to_owned())
            )
        })
        .unwrap_or_else(|| "none".to_owned());
    let action = output
        .state
        .active_experiment
        .as_ref()
        .map(|experiment| experiment.action_id.as_str())
        .unwrap_or("none");
    let rollback = output
        .state
        .active_rollback
        .as_ref()
        .map(|rollback| {
            if rollback.rollback_available {
                "available"
            } else {
                "restore-needed"
            }
        })
        .unwrap_or("none");
    let last = output
        .state
        .last_decision
        .as_ref()
        .map(|decision| decision.decision.as_str())
        .unwrap_or("none");

    format!(
        "daemon mode={} phase={} health={} watchdog_ok={} workload={} action={} rollback={} last_decision={} restore=\"{}\"\n",
        output.state.mode,
        output.state.phase.lifecycle_label(),
        output.current_health.state.as_str(),
        output.watchdog.ok,
        workload,
        action,
        rollback,
        last,
        output.manual_restore_command
    )
}

pub fn render_watch_notification(
    previous: &DaemonWatchSignature,
    current: &DaemonWatchSignature,
) -> Option<String> {
    if current.fault_reason != previous.fault_reason
        && let Some(reason) = current.fault_reason.as_ref()
    {
        return Some(format!("fault: {reason}"));
    }

    if current.rollback_action_id != previous.rollback_action_id
        && let Some(action_id) = current.rollback_action_id.as_ref()
    {
        if current.rollback_available {
            return Some(format!("rollback available for action_id={action_id}"));
        }
        return Some(format!("restore needed for action_id={action_id}"));
    }

    if previous.rollback_available
        && !current.rollback_available
        && let Some(action_id) = current.rollback_action_id.as_ref()
    {
        return Some(format!("restore needed for action_id={action_id}"));
    }

    if current.phase == DaemonPhase::Rollback && previous.phase != DaemonPhase::Rollback {
        return Some("rollback started".to_owned());
    }

    if current.active_action_id != previous.active_action_id
        && let Some(action_id) = current.active_action_id.as_ref()
    {
        return Some(format!("action applied action_id={action_id}"));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{DaemonPhase, SystemHealthSnapshot};

    #[test]
    fn daemon_watch_line_is_compact_and_notification_only_tracks_meaningful_changes() {
        let mut output = build_status_output_with_recent_decisions(0);
        output.state.mode = crate::daemon::DaemonMode::ApplyLowRisk;
        output.state.phase = DaemonPhase::Apply;
        output.current_health = SystemHealthSnapshot {
            state: crate::daemon::SystemHealthState::Healthy,
            ok_for_apply: true,
            reason_code: None,
            unix_nanos: Some(1_000),
            inputs: crate::daemon::health::SystemHealthInputs::default(),
            issues: Vec::new(),
        };

        let line = render_watch_line(&output);
        assert!(line.contains("mode=ApplyLowRisk phase=active health=healthy watchdog_ok=true"));

        let signature = DaemonWatchSignature::from_output(&output);
        let mut next = signature.clone();

        assert_eq!(render_watch_notification(&signature, &next), None);

        next.phase = DaemonPhase::Rollback;
        assert_eq!(
            render_watch_notification(&signature, &next),
            Some("rollback started".to_owned())
        );

        next.phase = DaemonPhase::Apply;
        next.active_action_id = Some("action-a".to_owned());
        assert_eq!(
            render_watch_notification(&signature, &next),
            Some("action applied action_id=action-a".to_owned())
        );

        next.active_action_id = None;
        next.rollback_action_id = Some("action-a".to_owned());
        next.rollback_available = true;
        assert_eq!(
            render_watch_notification(&signature, &next),
            Some("rollback available for action_id=action-a".to_owned())
        );

        next.rollback_available = false;
        assert_eq!(
            render_watch_notification(&signature, &next),
            Some("restore needed for action_id=action-a".to_owned())
        );

        next.fault_reason = Some("critical-error".to_owned());
        assert_eq!(
            render_watch_notification(&signature, &next),
            Some("fault: critical-error".to_owned())
        );
    }
}
