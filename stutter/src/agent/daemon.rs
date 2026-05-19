//! Daemon HTTP handler boundary.

use super::{autotune::mark_agent_daemon_fault, *};

pub(crate) async fn daemon_status_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    let active_recording = state.active_run.lock().await.is_some();
    let active_autotune = state.active_autotune.lock().await.is_some();
    let daemon_state = state.daemon_state.lock().await.clone();

    Json(daemon_status_response(
        active_recording,
        active_autotune,
        daemon_state,
        CapabilityProbe::default().probe(),
        system_health_monitor_for_agent_state(&state).probe(),
    ))
    .into_response()
}

pub(crate) async fn daemon_health_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    let daemon_state = state.daemon_state.lock().await.clone();
    let capabilities = CapabilityProbe::default().probe();
    let health = system_health_monitor_for_agent_state(&state).probe();

    Json(DaemonHealthResponse {
        ok: daemon_state.faulted.is_none() && health.ok_for_apply,
        phase: daemon_state.phase,
        health,
        degraded: daemon_state.degraded.clone(),
        faulted: daemon_state.faulted.clone(),
        unavailable_features: capabilities.unavailable_features(),
    })
    .into_response()
}

pub(crate) async fn daemon_policy_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    daemon_policy_response_handler(state, headers).await
}

pub(crate) async fn daemon_explain_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    let daemon_state = state.daemon_state.lock().await.clone();
    let capabilities = CapabilityProbe::default().probe();
    let health = system_health_monitor_for_agent_state(&state).probe();

    Json(daemon_explain_response(daemon_state, capabilities, health)).into_response()
}

pub(crate) async fn daemon_pause_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) =
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::ControlDaemon)
    {
        return status.into_response();
    }

    let mut daemon_state = state.daemon_state.lock().await;
    daemon_state.phase = DaemonPhase::Paused;
    daemon_state.faulted = None;
    daemon_state.last_decision = Some(daemon_decision_state(
        "daemon_paused",
        "operator requested daemon pause",
    ));

    Json(DaemonControlResponse {
        ok: true,
        phase: daemon_state.phase,
        message: "daemon paused".to_owned(),
        manual_restore_command: "stutter daemon emergency-restore".to_owned(),
        restore_messages: Vec::new(),
    })
    .into_response()
}

pub(crate) async fn daemon_resume_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) =
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::ControlDaemon)
    {
        return status.into_response();
    }

    let mut daemon_state = state.daemon_state.lock().await;
    if daemon_state.faulted.is_some() {
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "cannot resume a faulted daemon state; restore first".to_owned(),
            }),
        )
            .into_response();
    }

    daemon_state.phase = DaemonPhase::Observe;
    daemon_state.last_decision = Some(daemon_decision_state(
        "daemon_resumed",
        "operator requested daemon resume",
    ));

    Json(DaemonControlResponse {
        ok: true,
        phase: daemon_state.phase,
        message: "daemon resumed".to_owned(),
        manual_restore_command: "stutter daemon emergency-restore".to_owned(),
        restore_messages: Vec::new(),
    })
    .into_response()
}

pub(crate) async fn daemon_restore_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) =
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::RollbackAction)
    {
        return status.into_response();
    }

    let outcome = match restore_known_autotune_actions(AutotuneRestoreCommandInput {
        journal_path: None,
        audit_path: None,
        history_path: None,
        dry_run: false,
    }) {
        Ok(outcome) => outcome,
        Err(err) => {
            mark_agent_daemon_fault(
                &state,
                DaemonMode::ApplyLowRisk,
                "daemon_restore_failed",
                format!("daemon restore failed: {err:#}"),
            )
            .await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("daemon restore failed: {err:#}"),
                }),
            )
                .into_response();
        }
    };

    let profile_outcome = match restore::restore_profile_state_default(false) {
        Ok(outcome) => outcome,
        Err(err) => {
            mark_agent_daemon_fault(
                &state,
                DaemonMode::ApplyLowRisk,
                "daemon_restore_failed",
                format!("daemon profile restore failed: {err:#}"),
            )
            .await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("daemon profile restore failed: {err:#}"),
                }),
            )
                .into_response();
        }
    };
    let restore_summary = daemon_restore_summary_fields(&outcome, &profile_outcome);
    let restore_messages = daemon_restore_messages(&outcome, &profile_outcome);

    let mut daemon_state = state.daemon_state.lock().await;
    let ok = matches!(
        outcome.status,
        AutotuneRestoreStatus::Clean | AutotuneRestoreStatus::Restored
    );
    if ok {
        daemon_state.phase = DaemonPhase::Observe;
        daemon_state.active_experiment = None;
        daemon_state.active_rollback = None;
        daemon_state.faulted = None;
        daemon_state.degraded.clear();
        daemon_state.last_decision = Some(daemon_decision_state(
            "daemon_restored",
            format!("daemon restore completed {restore_summary}"),
        ));
    } else {
        daemon_state.phase = DaemonPhase::Faulted;
        daemon_state.faulted = Some(crate::daemon::state::DaemonFaultState {
            reason: format!(
                "daemon restore did not complete safely status={:?}",
                outcome.status
            ),
            manual_restore_command: Some("stutter daemon emergency-restore --dry-run".to_owned()),
        });
        daemon_state.last_decision = Some(daemon_decision_state(
            "daemon_restore_failed",
            format!(
                "daemon restore did not complete safely status={:?} failed_actions={} skipped_actions={}",
                outcome.status, outcome.failed_actions, outcome.skipped_actions
            ),
        ));
    }

    let status = if ok {
        StatusCode::OK
    } else {
        StatusCode::CONFLICT
    };
    (
        status,
        Json(DaemonControlResponse {
            ok,
            phase: daemon_state.phase,
            message: format!("daemon restore status={:?}", outcome.status),
            manual_restore_command: "stutter daemon emergency-restore".to_owned(),
            restore_messages,
        }),
    )
        .into_response()
}

pub(crate) fn daemon_restore_messages(
    outcome: &crate::autotune::emergency_restore::AutotuneRestoreOutcome,
    profile_outcome: &restore::ProfileRestoreCommandOutcome,
) -> Vec<String> {
    let mut messages = outcome.messages.clone();
    messages.extend(profile_outcome.messages.clone());
    messages.push(format!(
        "daemon restore summary: {}",
        daemon_restore_summary_fields(outcome, profile_outcome)
    ));
    messages
}

pub(crate) fn daemon_restore_summary_fields(
    outcome: &crate::autotune::emergency_restore::AutotuneRestoreOutcome,
    profile_outcome: &restore::ProfileRestoreCommandOutcome,
) -> String {
    restore::RestoreSummaryFields::from_profile(
        format!("{:?}", outcome.status),
        outcome.restored_actions,
        outcome.failed_actions,
        outcome.skipped_actions,
        profile_outcome,
    )
    .render_fields()
}

pub(crate) async fn daemon_policy_response_handler(
    state: Arc<AgentState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    let daemon_state = state.daemon_state.lock().await.clone();
    let capabilities = CapabilityProbe::default().probe();
    let health = system_health_monitor_for_agent_state(&state).probe();

    Json(daemon_policy_response(daemon_state, capabilities, health)).into_response()
}

pub(crate) fn daemon_policy_response(
    daemon_state: DaemonState,
    capabilities: DaemonCapabilities,
    health: SystemHealthSnapshot,
) -> DaemonPolicyResponse {
    let policy = daemon_policy_from_state(&daemon_state);
    let descriptor = daemon_control_plane_descriptor();
    let policy_context = policy_context_from_daemon_status(&daemon_state, &health, &capabilities);
    let explanation =
        policy.explain_action_with_context(PolicyIntent::Observe, &descriptor, &policy_context);
    let policy_explanation =
        DaemonPolicyExplanation::from_policy_with_context(&policy, &policy_context);
    let watchdog = evaluate_daemon_watchdog(
        DaemonWatchdogInputs::from_state_and_health(&daemon_state, &health),
        &DaemonWatchdogConfig::default(),
    );

    DaemonPolicyResponse {
        policy,
        explanation,
        policy_explanation,
        capabilities,
        health,
        watchdog,
        manual_restore_command: "stutter daemon emergency-restore".to_owned(),
    }
}

pub(crate) fn daemon_explain_response(
    daemon_state: DaemonState,
    capabilities: DaemonCapabilities,
    health: SystemHealthSnapshot,
) -> DaemonExplainResponse {
    let policy = daemon_policy_from_state(&daemon_state);
    let descriptor = daemon_control_plane_descriptor();
    let explanation = policy.explain_action(PolicyIntent::Observe, &descriptor);
    let policy_context = policy_context_from_daemon_status(&daemon_state, &health, &capabilities);
    let policy_explanation =
        DaemonPolicyExplanation::from_policy_with_context(&policy, &policy_context);
    let watchdog = evaluate_daemon_watchdog(
        DaemonWatchdogInputs::from_state_and_health(&daemon_state, &health),
        &DaemonWatchdogConfig::default(),
    );
    let status_explanation =
        DaemonStatusExplanation::from_state_health_watchdog(&daemon_state, &health, &watchdog);

    DaemonExplainResponse {
        daemon_state,
        policy,
        explanation,
        policy_explanation,
        capabilities,
        health,
        watchdog,
        why_no_optimize: status_explanation.why_no_optimize,
        what_changed: status_explanation.what_changed,
        manual_restore_command: "stutter daemon emergency-restore".to_owned(),
    }
}

pub(crate) fn daemon_status_response(
    active_recording: bool,
    active_autotune: bool,
    daemon_state: DaemonState,
    capabilities: DaemonCapabilities,
    health: SystemHealthSnapshot,
) -> DaemonStatusResponse {
    let watchdog = evaluate_daemon_watchdog(
        DaemonWatchdogInputs::from_state_and_health(&daemon_state, &health),
        &DaemonWatchdogConfig::default(),
    );
    let message = if daemon_state.phase.is_faulted() {
        "daemon is faulted; run the manual restore command before applying more changes".to_owned()
    } else if active_autotune {
        "autotune controller active".to_owned()
    } else if active_recording {
        "recording active".to_owned()
    } else {
        "daemon control plane idle".to_owned()
    };

    DaemonStatusResponse {
        active_recording,
        active_autotune,
        daemon_state,
        capabilities,
        health,
        watchdog,
        manual_restore_command: "stutter daemon emergency-restore".to_owned(),
        message,
    }
}

pub(crate) fn system_health_monitor_for_agent_state(state: &AgentState) -> SystemHealthMonitor {
    SystemHealthMonitor::new(
        SystemHealthProbeRoot::default(),
        state.health_thresholds.clone(),
    )
}

pub(crate) fn daemon_policy_from_state(state: &DaemonState) -> DaemonPolicy {
    let config = daemon_config_for_runtime_mode(
        state.mode,
        ActionSource::RemoteAgent,
        state
            .active_target
            .as_ref()
            .and_then(|target| target.root_pid),
        None,
    );

    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

pub(crate) fn daemon_control_plane_descriptor() -> ActionDescriptor {
    ActionDescriptor {
        action_id: ActionId::new("daemon-control-plane".to_owned()),
        action_kind: "daemon-control-plane".to_owned(),
        safety_class: SafetyClass::ObserveOnly,
        effect_scope: ActionEffectScope::ObserveOnly,
        rollback: RollbackRequirement::NotRequiredForDryRun,
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: false,
        confidence: None,
    }
}
