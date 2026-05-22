//! Remote autotune daemon-state transitions.

use super::{policy::safety_class_for_daemon_mode, *};

pub(crate) fn daemon_state_for_agent_decision(
    mode: DaemonMode,
    phase: DaemonPhase,
    decision: &str,
    reason: impl Into<String>,
) -> DaemonState {
    DaemonState {
        mode,
        phase,
        last_decision: Some(daemon_decision_state(decision, reason)),
        ..DaemonState::default()
    }
}

pub(crate) fn daemon_mode_from_agent_mode(value: &str) -> DaemonMode {
    match value {
        "suggest" => DaemonMode::Suggest,
        "apply-low-risk" => DaemonMode::ApplyLowRisk,
        "apply-medium-risk" => DaemonMode::ApplyMediumRisk,
        "apply-high-risk" => DaemonMode::ApplyHighRisk,
        _ => DaemonMode::Observe,
    }
}

pub(crate) fn daemon_target_from_autotune_request(
    request: &AutotuneStartRequest,
) -> Option<DaemonTargetState> {
    let active_targets = usize::from(request.tree_pid.is_some())
        + usize::from(request.watch_process.is_some())
        + usize::from(request.auto_focus);
    let comm = request.watch_process.clone();

    if request.tree_pid.is_none() && active_targets == 0 && comm.is_none() {
        return None;
    }

    Some(DaemonTargetState {
        root_pid: request.tree_pid,
        active_targets,
        comm,
    })
}

pub(crate) fn daemon_phase_for_started_mode(mode: DaemonMode) -> DaemonPhase {
    match mode {
        DaemonMode::Observe => DaemonPhase::Observe,
        DaemonMode::Suggest => DaemonPhase::Decide,
        DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk | DaemonMode::ApplyHighRisk => {
            DaemonPhase::Apply
        }
    }
}

pub(crate) fn daemon_state_for_autotune_start(
    policy: &DaemonPolicy,
    request: &AutotuneStartRequest,
    started_unix_nanos: u128,
) -> DaemonState {
    let action_id = format!("remote-autotune-start:{}", policy.mode.as_str());

    DaemonState {
        mode: policy.mode,
        phase: daemon_phase_for_started_mode(policy.mode),
        active_target: daemon_target_from_autotune_request(request),
        active_experiment: Some(DaemonExperimentState {
            experiment_id: format!("remote-autotune-{started_unix_nanos}"),
            action_id: action_id.clone(),
            candidate_name: request
                .watch_process
                .clone()
                .or_else(|| Some(policy.mode.as_str().to_owned())),
            mode: policy.mode,
            safety_class: safety_class_for_daemon_mode(policy.mode),
            started_unix_nanos: Some(started_unix_nanos),
        }),
        active_rollback: policy.mode.supports_apply().then(|| DaemonRollbackState {
            action_id,
            mode: policy.mode,
            safety_class: safety_class_for_daemon_mode(policy.mode),
            rollback_available: false,
            token: None,
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
        }),
        last_decision: Some(daemon_decision_state(
            "autotune_started",
            format!("remote autotune {} controller started", policy.mode),
        )),
        ..DaemonState::default()
    }
}

pub(crate) fn remote_autotune_controller_duration(
    mode: DaemonMode,
    duration_seconds: Option<u64>,
) -> Option<std::time::Duration> {
    if mode == DaemonMode::ApplyLowRisk {
        None
    } else {
        duration_seconds.map(std::time::Duration::from_secs)
    }
}

pub(crate) fn apply_remote_autotune_runtime_mode_overrides(
    runtime_config: AutotuneRuntimeConfig,
    mode: DaemonMode,
    duration_seconds: Option<u64>,
) -> AutotuneRuntimeConfig {
    if mode == DaemonMode::ApplyLowRisk {
        runtime_config.with_candidate_window_seconds(duration_seconds.unwrap_or(30))
    } else {
        runtime_config
    }
}

pub(crate) fn remote_autotune_start_message(mode: DaemonMode) -> &'static str {
    if mode == DaemonMode::ApplyLowRisk {
        "remote autotune apply-low-risk controller started; apply modes are enabled"
    } else {
        "remote autotune observe/suggest controller started; apply modes remain disabled"
    }
}

pub(crate) fn daemon_state_for_autotune_stop(
    mode: DaemonMode,
    exit: &crate::autotune::runtime::AutotuneControllerExit,
) -> DaemonState {
    let last_decision = exit
        .last_decision
        .as_ref()
        .map(|decision| DaemonDecisionState {
            decision: decision.decision.clone(),
            reason: format!("stopped: {}; last_reason={}", exit.reason, decision.reason),
            unix_nanos: Some(decision.unix_nanos),
            score_total: Some(decision.score_total),
            candidate_count: Some(decision.candidate_count),
            top_denied_reason: decision.top_denied_reason.clone(),
            planner: decision.planner.clone(),
            situation: Some(decision.situation.clone()),
            focus_kind: decision.focus_kind.clone(),
        })
        .unwrap_or_else(|| daemon_decision_state("autotune_stopped", exit.reason.clone()));

    DaemonState {
        mode,
        phase: DaemonPhase::Disabled,
        last_decision: Some(last_decision),
        ..DaemonState::default()
    }
}

pub(crate) async fn replace_agent_daemon_state(state: &AgentState, daemon_state: DaemonState) {
    *state.daemon_state.lock().await = daemon_state;
}

pub(crate) async fn mark_agent_daemon_fault(
    state: &AgentState,
    mode: DaemonMode,
    decision: &str,
    reason: impl Into<String>,
) {
    replace_agent_daemon_state(state, daemon_state_for_agent_fault(mode, decision, reason)).await;
}

pub(crate) async fn mark_agent_daemon_policy_rejection(
    state: &AgentState,
    mode: DaemonMode,
    decision: &str,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    let mut daemon_state = state.daemon_state.lock().await;
    daemon_state.mode = mode;
    daemon_state.phase = DaemonPhase::Faulted;
    daemon_state.last_decision = Some(daemon_decision_state(decision, reason.clone()));
    daemon_state
        .degraded
        .push(crate::daemon::state::DaemonDegradedStatus {
            category: "agent".to_owned(),
            message: reason.clone(),
        });
    daemon_state.faulted = Some(crate::daemon::state::DaemonFaultState {
        reason,
        manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
    });
}
