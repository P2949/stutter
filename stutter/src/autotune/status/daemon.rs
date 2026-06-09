use std::path::PathBuf;

use super::{
    history::cooldown_remaining_seconds_from_unix_nanos,
    model::{AutotuneStatus, StatusKeptAction, StatusTarget},
    render::{normalized_decision, restore_path_display},
};
use crate::daemon::{
    policy::DaemonMode,
    state::{DaemonPhase, DaemonState, DaemonTargetState},
};

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
            .and_then(|decision| decision.diagnostic_current_raw_score_total),
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
