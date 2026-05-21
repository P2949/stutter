//! Autotune HTTP handler boundary.

use super::{daemon::system_health_monitor_for_agent_state, *};

#[derive(Debug)]
pub(crate) struct AutotuneStartSecurityRejection {
    pub(crate) status: StatusCode,
    pub(crate) audit_message: String,
    pub(crate) response_message: String,
}

pub(crate) fn safety_class_for_daemon_mode(mode: DaemonMode) -> SafetyClass {
    match mode {
        DaemonMode::Observe | DaemonMode::Suggest => SafetyClass::ObserveOnly,
        DaemonMode::ApplyLowRisk => SafetyClass::ReversibleLowRisk,
        DaemonMode::ApplyMediumRisk => SafetyClass::ReversibleMediumRisk,
        DaemonMode::ApplyHighRisk => SafetyClass::HighRisk,
    }
}

pub(crate) fn supported_remote_modes(limits: &AgentAutotuneLimits) -> Vec<DaemonMode> {
    let mut modes = vec![DaemonMode::Observe, DaemonMode::Suggest];

    if limits.max_mode >= DaemonMode::ApplyLowRisk
        && limits.max_safety_class >= SafetyClass::ReversibleLowRisk
    {
        modes.push(DaemonMode::ApplyLowRisk);
    }

    if limits.max_mode >= DaemonMode::ApplyMediumRisk
        && limits.max_safety_class >= SafetyClass::ReversibleMediumRisk
    {
        modes.push(DaemonMode::ApplyMediumRisk);
    }

    if limits.max_mode >= DaemonMode::ApplyHighRisk
        && limits.max_safety_class >= SafetyClass::HighRisk
        && limits.allow_high_risk
    {
        modes.push(DaemonMode::ApplyHighRisk);
    }

    modes
}

pub(crate) fn supported_remote_mode_labels(limits: &AgentAutotuneLimits) -> Vec<String> {
    supported_remote_modes(limits)
        .into_iter()
        .map(|mode| mode.as_str().to_owned())
        .collect()
}

pub(crate) fn remote_mode_supported(limits: &AgentAutotuneLimits, mode: DaemonMode) -> bool {
    supported_remote_modes(limits).contains(&mode)
}

pub(crate) fn daemon_policy_for_remote_mode(
    mode: DaemonMode,
    state: &AgentState,
    request: &AutotuneStartRequest,
    remote_context: RemotePolicyContext,
) -> DaemonPolicy {
    let mut config = daemon_config_for_runtime_mode(
        mode,
        ActionSource::RemoteAgent,
        request.tree_pid,
        request.watch_process.clone(),
    );
    config.remote.allow_remote_apply = mode.supports_apply();
    config.safety.allow_high_risk =
        state.autotune_limits.allow_high_risk && mode == DaemonMode::ApplyHighRisk;
    config.safety.allow_system_wide_suggestions =
        state.autotune_limits.allow_system_wide_suggestions;
    config.safety.allow_system_wide_apply = state.autotune_limits.allow_system_wide_apply;

    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(remote_context),
    })
}

pub(crate) fn remote_policy_context_for_request(
    state: &AgentState,
    request_authorized: bool,
) -> RemotePolicyContext {
    RemotePolicyContext {
        bind_is_loopback: agent_state_is_local(state),
        auth_configured: state.auth.any_token_configured(),
        request_authorized,
        limits: state.autotune_limits.clone(),
    }
}

pub(crate) fn parse_focus_source_or_hybrid(value: Option<&str>) -> FocusSource {
    match value {
        Some("heuristic") => FocusSource::Heuristic,
        Some("foreground") => FocusSource::Foreground,
        Some("hybrid") | None => FocusSource::Hybrid,
        Some(_) => FocusSource::Hybrid,
    }
}

pub(crate) fn parse_foreground_source_or_auto(value: Option<&str>) -> ForegroundSource {
    match value {
        Some("sway") => ForegroundSource::Sway,
        Some("hyprland") => ForegroundSource::Hyprland,
        Some("x11") => ForegroundSource::X11,
        Some("auto") | None => ForegroundSource::Auto,
        Some(_) => ForegroundSource::Auto,
    }
}

pub(crate) fn validate_autotune_start_limits(
    request: &AutotuneStartRequest,
    state: &AgentState,
) -> anyhow::Result<()> {
    let limits = &state.autotune_limits;

    if limits.max_active_controllers != 1 {
        anyhow::bail!("remote autotune supports exactly one active controller");
    }

    if limits.allow_system_wide_apply {
        anyhow::bail!("remote autotune system-wide apply is disabled by default");
    }

    let requested_target_count =
        usize::from(request.watch_process.is_some()) + usize::from(request.tree_pid.is_some());

    if requested_target_count == 0 && !request.auto_focus {
        anyhow::bail!("autotune start requires watch_process, tree_pid, or auto_focus");
    }

    if let Some(duration_seconds) = request
        .duration_seconds
        .filter(|&d| d > limits.max_candidate_window_seconds)
    {
        anyhow::bail!(
            "autotune duration_seconds {} exceeds max_candidate_window_seconds {}",
            duration_seconds,
            limits.max_candidate_window_seconds
        );
    }

    Ok(())
}

pub(crate) fn remote_autotune_start_descriptor(
    mode: DaemonMode,
    request: &AutotuneStartRequest,
) -> ActionDescriptor {
    ActionDescriptor {
        action_id: ActionId::new(format!("remote-autotune-start:{}", mode.as_str())),
        action_kind: "remote-autotune-start".to_owned(),
        safety_class: safety_class_for_daemon_mode(mode),
        effect_scope: if mode.supports_apply() {
            ActionEffectScope::LocalProcessTree
        } else {
            ActionEffectScope::ObserveOnly
        },
        rollback: if mode.supports_apply() {
            RollbackRequirement::RequiredBeforeApply
        } else {
            RollbackRequirement::NotRequiredForDryRun
        },
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: mode.supports_apply()
            && request.tree_pid.is_none()
            && request.watch_process.is_none()
            && !request.auto_focus,
        confidence: Some(1.0),
    }
}

pub(crate) fn policy_rejection_status(
    rejection: &PolicyRejection,
    auth_error: Option<StatusCode>,
) -> StatusCode {
    match rejection {
        PolicyRejection::RemoteApplyRequiresConfiguredAuth => StatusCode::UNAUTHORIZED,
        PolicyRejection::RemoteApplyRequiresAuthorizedRequest => {
            auth_error.unwrap_or(StatusCode::UNAUTHORIZED)
        }
        PolicyRejection::RemoteApplyRequiresLoopbackBind
        | PolicyRejection::HighRiskRequiresExplicitOptIn => StatusCode::FORBIDDEN,
        PolicyRejection::RemoteApplyDisabled
        | PolicyRejection::RemoteModeNotAllowed { .. }
        | PolicyRejection::RemoteTargetCountTooHigh { .. } => StatusCode::BAD_REQUEST,
        _ => StatusCode::BAD_REQUEST,
    }
}

pub(crate) fn remote_autotune_apply_auth_rejection(
    mode: DaemonMode,
    status: StatusCode,
    response_message: &'static str,
) -> AutotuneStartSecurityRejection {
    AutotuneStartSecurityRejection {
        status,
        audit_message: format!("rejected remote autotune mode={mode}: {response_message}"),
        response_message: response_message.to_owned(),
    }
}

pub(crate) fn remote_autotune_mode_limit_rejection(
    mode: DaemonMode,
) -> AutotuneStartSecurityRejection {
    let reason = PolicyRejection::RemoteModeNotAllowed { mode }.to_string();
    AutotuneStartSecurityRejection {
        status: StatusCode::BAD_REQUEST,
        audit_message: format!("rejected remote autotune mode={mode}: {reason}"),
        response_message: reason,
    }
}

pub(crate) fn rejection_from_policy_explanation(
    mode: DaemonMode,
    explanation: crate::daemon::explain::PolicyExplanation,
    auth_error: Option<StatusCode>,
) -> Option<AutotuneStartSecurityRejection> {
    match explanation.decision {
        PolicyDecisionKind::Allowed => None,
        PolicyDecisionKind::Rejected { rejection } => {
            let status = policy_rejection_status(&rejection, auth_error);
            let reason = explanation.final_reason;
            Some(AutotuneStartSecurityRejection {
                status,
                audit_message: format!("rejected remote autotune mode={mode}: {reason}"),
                response_message: reason,
            })
        }
    }
}

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

#[cfg(test)]
pub(crate) fn policy_for_remote_autotune_start(
    headers: &HeaderMap,
    state: &AgentState,
    request: &AutotuneStartRequest,
) -> Result<DaemonPolicy, AutotuneStartSecurityRejection> {
    policy_for_remote_autotune_start_with_safety_context(
        headers,
        state,
        request,
        &DaemonState::default(),
        &SystemHealthSnapshot::default(),
        &CapabilityProbe::default().probe(),
    )
}

pub(crate) fn policy_for_remote_autotune_start_with_safety_context(
    headers: &HeaderMap,
    state: &AgentState,
    request: &AutotuneStartRequest,
    daemon_state: &DaemonState,
    health: &SystemHealthSnapshot,
    capabilities: &DaemonCapabilities,
) -> Result<DaemonPolicy, AutotuneStartSecurityRejection> {
    let raw_mode = request.mode.trim();
    let mode = raw_mode
        .parse::<DaemonMode>()
        .map_err(|err| AutotuneStartSecurityRejection {
            status: StatusCode::BAD_REQUEST,
            audit_message: format!("rejected remote autotune mode={raw_mode}: {err}"),
            response_message:
                "unsupported remote autotune mode; use observe, suggest, or apply-low-risk"
                    .to_owned(),
        })?;

    let auth_result = authorize_remote_autotune_start(headers, state);
    let auth_error = auth_result.as_ref().err().copied();

    if mode.supports_apply() {
        if !state.auth.apply_token_configured() {
            return Err(remote_autotune_apply_auth_rejection(
                mode,
                StatusCode::UNAUTHORIZED,
                "remote autotune apply mode requires a configured bearer token",
            ));
        }

        if let Err(status) = auth_result {
            return Err(remote_autotune_apply_auth_rejection(
                mode,
                status,
                "remote autotune apply mode requires a valid bearer token",
            ));
        }

        if !remote_mode_supported(&state.autotune_limits, mode) {
            return Err(remote_autotune_mode_limit_rejection(mode));
        }
    } else if let Err(status) = auth_result {
        return Err(AutotuneStartSecurityRejection {
            status,
            audit_message: format!("rejected remote autotune mode={mode}: unauthorized"),
            response_message: "unauthorized".to_owned(),
        });
    }

    let request_authorized = auth_error.is_none();
    let policy = daemon_policy_for_remote_mode(
        mode,
        state,
        request,
        remote_policy_context_for_request(state, request_authorized),
    );

    let descriptor = remote_autotune_start_descriptor(mode, request);
    let intent = match mode {
        DaemonMode::Observe => PolicyIntent::Observe,
        DaemonMode::Suggest => PolicyIntent::Suggest,
        DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk | DaemonMode::ApplyHighRisk => {
            PolicyIntent::Apply
        }
    };
    let explanation = policy.explain_action(intent.clone(), &descriptor);
    if let Some(rejection) = rejection_from_policy_explanation(mode, explanation, auth_error) {
        return Err(rejection);
    }

    let policy_context = policy_context_from_daemon_status(daemon_state, health, capabilities);
    let explanation = policy.explain_action_with_context(intent, &descriptor, &policy_context);
    if let Some(rejection) = rejection_from_policy_explanation(mode, explanation, auth_error) {
        return Err(rejection);
    }

    Ok(policy)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AutotuneReapStatus {
    NoActiveSession,
    StillActive,
    Completed,
    Failed,
}

pub(crate) async fn reap_finished_autotune(state: &AgentState) -> AutotuneReapStatus {
    let handle = {
        let mut active = state.active_autotune.lock().await;
        match active.as_ref() {
            Some(handle) if handle.join.is_finished() => active.take(),
            Some(_) => return AutotuneReapStatus::StillActive,
            None => return AutotuneReapStatus::NoActiveSession,
        }
    };

    let Some(handle) = handle else {
        return AutotuneReapStatus::NoActiveSession;
    };

    let mode = daemon_mode_from_agent_mode(&handle.mode);
    match handle.join.await {
        Ok(Ok(exit)) => {
            audit_agent_event(
                "remote-autotune-reap",
                true,
                0,
                format!("reason={}", exit.reason),
            );
            replace_agent_daemon_state(state, daemon_state_for_autotune_stop(mode, &exit)).await;
            AutotuneReapStatus::Completed
        }
        Ok(Err(err)) => {
            audit_agent_event("remote-autotune-reap", false, 0, format!("error={err:#}"));
            mark_agent_daemon_fault(
                state,
                mode,
                "autotune_controller_failed",
                format!("autotune controller failed: {err:#}"),
            )
            .await;
            AutotuneReapStatus::Failed
        }
        Err(err) => {
            audit_agent_event(
                "remote-autotune-reap",
                false,
                0,
                format!("join_error={err}"),
            );
            mark_agent_daemon_fault(
                state,
                mode,
                "autotune_controller_join_failed",
                format!("autotune controller task join failed: {err}"),
            )
            .await;
            AutotuneReapStatus::Failed
        }
    }
}

pub(crate) async fn autotune_status_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    reap_finished_autotune(&state).await;

    let active = state.active_autotune.lock().await;
    let daemon_state = state.daemon_state.lock().await.clone();
    let disk_status = crate::autotune::status::load_autotune_status(
        &crate::autotune::history::default_autotune_history_path(),
    )
    .ok();

    let response = match active.as_ref() {
        Some(handle) => AutotuneStatusResponse {
            active: true,
            mode: Some(handle.mode.clone()),
            watch_process: handle.watch_process.clone(),
            tree_pid: handle.tree_pid,
            started_unix_nanos: Some(handle.started_unix_nanos),
            focus_group: disk_status
                .as_ref()
                .and_then(|status| status.focus_group.clone()),
            target_root: handle.tree_pid.or_else(|| {
                disk_status
                    .as_ref()
                    .and_then(|status| status.target.as_ref().map(|target| target.pid))
            }),
            current_score: disk_status.as_ref().and_then(|status| status.current_score),
            active_profile: disk_status
                .as_ref()
                .and_then(|status| status.active_profile.clone()),
            last_decision: disk_status
                .as_ref()
                .map(|status| status.last_decision.clone()),
            rollback_available: disk_status
                .as_ref()
                .map(|status| status.rollback_available)
                .unwrap_or(false),
            cooldown_remaining_seconds: disk_status
                .as_ref()
                .and_then(|status| status.cooldown_remaining_seconds),
            data_quality: disk_status
                .as_ref()
                .and_then(|status| status.data_quality.clone()),
            last_fault: disk_status
                .as_ref()
                .and_then(|status| status.last_fault.clone()),
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            daemon_state: daemon_state.clone(),
            message: format!("autotune {} controller active", handle.mode),
        },
        None => AutotuneStatusResponse {
            active: false,
            mode: None,
            watch_process: None,
            tree_pid: None,
            started_unix_nanos: None,
            focus_group: disk_status
                .as_ref()
                .and_then(|status| status.focus_group.clone()),
            target_root: disk_status
                .as_ref()
                .and_then(|status| status.target.as_ref().map(|target| target.pid)),
            current_score: disk_status.as_ref().and_then(|status| status.current_score),
            active_profile: disk_status
                .as_ref()
                .and_then(|status| status.active_profile.clone()),
            last_decision: disk_status
                .as_ref()
                .map(|status| status.last_decision.clone()),
            rollback_available: disk_status
                .as_ref()
                .map(|status| status.rollback_available)
                .unwrap_or(false),
            cooldown_remaining_seconds: disk_status
                .as_ref()
                .and_then(|status| status.cooldown_remaining_seconds),
            data_quality: disk_status
                .as_ref()
                .and_then(|status| status.data_quality.clone()),
            last_fault: disk_status
                .as_ref()
                .and_then(|status| status.last_fault.clone()),
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            daemon_state,
            message: "no autotune session active".to_owned(),
        },
    };

    Json(response).into_response()
}

pub(crate) async fn autotune_start_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
    Json(request): Json<AutotuneStartRequest>,
) -> impl IntoResponse {
    let daemon_state = state.daemon_state.lock().await.clone();
    let capabilities = CapabilityProbe::default().probe();
    let health = system_health_monitor_for_agent_state(&state).probe();
    let policy = match policy_for_remote_autotune_start_with_safety_context(
        &headers,
        &state,
        &request,
        &daemon_state,
        &health,
        &capabilities,
    ) {
        Ok(policy) => policy,
        Err(rejection) => {
            audit_agent_event(
                "remote-autotune-start",
                false,
                0,
                rejection.audit_message.clone(),
            );
            mark_agent_daemon_policy_rejection(
                &state,
                daemon_mode_from_agent_mode(&request.mode),
                "autotune_policy_rejected",
                rejection.audit_message,
            )
            .await;
            return (
                rejection.status,
                Json(ErrorResponse {
                    error: rejection.response_message,
                }),
            )
                .into_response();
        }
    };
    let mode = policy.mode.as_str().to_owned();

    if let Err(err) = validate_autotune_start_limits(&request, &state) {
        audit_agent_event(
            "remote-autotune-start",
            false,
            0,
            format!("limit_rejected reason={err:#}"),
        );
        mark_agent_daemon_fault(
            &state,
            daemon_mode_from_agent_mode(&request.mode),
            "autotune_limit_rejected",
            format!("autotune limit rejected: {err}"),
        )
        .await;
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("autotune limit rejected: {err}"),
            }),
        )
            .into_response();
    }

    reap_finished_autotune(&state).await;

    let mut active = state.active_autotune.lock().await;
    if active.is_some() {
        audit_agent_event(
            "remote-autotune-start",
            false,
            0,
            "conflict: autotune session already active".to_owned(),
        );
        mark_agent_daemon_fault(
            &state,
            policy.mode,
            "autotune_start_conflict",
            "an autotune session is already active",
        )
        .await;
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "an autotune session is already active".to_owned(),
            }),
        )
            .into_response();
    }

    let input = crate::autotune::commands::live::AutotuneCommandInput {
        config: request.config.as_deref().map(PathBuf::from),
        watch_process: request.watch_process.clone(),
        tree_pid: request.tree_pid,
        profiles: request.profiles.as_deref().map(PathBuf::from),
        mode: policy.mode.as_str().to_owned(),
        decision_log: request.decision_log.as_deref().map(PathBuf::from),
        duration_seconds: request.duration_seconds,
        summary_ms: request.summary_ms.unwrap_or(1_000),
        preset: request
            .preset
            .clone()
            .unwrap_or_else(|| "diagnosis".to_owned()),
        hwmon: request.hwmon,
        mangohud_log: request.mangohud_log.as_deref().map(PathBuf::from),
        auto_focus: request.auto_focus,
        min_focus_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
        focus_source: parse_focus_source_or_hybrid(request.focus_source.as_deref()),
        foreground_window: false,
        foreground_source: parse_foreground_source_or_auto(request.foreground_source.as_deref()),
        foreground_poll_ms: request.foreground_poll_ms.unwrap_or(1_000),
        foreground_max_stale_ms: request.foreground_max_stale_ms.unwrap_or(2_500),
        washout_seconds: request
            .washout_seconds
            .unwrap_or(crate::autotune::washout::DEFAULT_WASHOUT_SECONDS),
        washout_verify_interval_ms: request
            .washout_verify_interval_ms
            .unwrap_or(crate::autotune::washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS),
        allow_system_wide_suggestions: false,
        allow_medium_risk: policy.mode == DaemonMode::ApplyMediumRisk,
        high_risk_dry_run: false,
        dry_run_all_safe: false,
    };

    let monitor_config = match crate::cli::autotune_monitor_config(&input) {
        Ok(config) => config,
        Err(err) => {
            audit_agent_event(
                "remote-autotune-start",
                false,
                0,
                format!("config_error reason={err:#}"),
            );
            mark_agent_daemon_fault(
                &state,
                policy.mode,
                "autotune_config_error",
                format!("invalid autotune configuration: {err}"),
            )
            .await;
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("invalid autotune configuration: {err}"),
                }),
            )
                .into_response();
        }
    };

    let started_unix_nanos = crate::audit::unix_nanos_now();

    let handle = {
        let profile_list = match input.profiles.as_deref() {
            Some(path) => match crate::profiles::load_profiles(path) {
                Ok(profiles) => profiles,
                Err(err) => {
                    audit_agent_event(
                        "remote-autotune-start",
                        false,
                        0,
                        format!("profile_load_error reason={err:#}"),
                    );
                    mark_agent_daemon_fault(
                        &state,
                        policy.mode,
                        "autotune_profile_load_error",
                        format!("failed to load profiles: {err:#}"),
                    )
                    .await;
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse {
                            error: format!("failed to load profiles: {err:#}"),
                        }),
                    )
                        .into_response();
                }
            },
            None => Vec::new(),
        };

        let mut daemon_config = daemon_config_for_runtime_mode(
            policy.mode,
            policy.source,
            request.tree_pid,
            request.watch_process.clone(),
        );
        daemon_config.remote.allow_remote_apply = policy.mode.supports_apply();
        daemon_config.safety.allow_high_risk = policy.allow_high_risk;
        daemon_config.safety.allow_system_wide_suggestions = policy.allow_system_wide_suggestions;
        daemon_config.safety.allow_system_wide_apply = policy.allow_system_wide_apply;

        let mut runtime_config = AutotuneRuntimeConfig::from_daemon_parts(
            daemon_config,
            policy.clone(),
            request.decision_log.as_deref().map(PathBuf::from),
        )
        .with_profiles(profile_list);

        runtime_config = apply_remote_autotune_runtime_mode_overrides(
            runtime_config,
            policy.mode,
            input.duration_seconds,
        );

        let (stop_tx, stop_rx) = oneshot::channel();
        let duration = remote_autotune_controller_duration(policy.mode, input.duration_seconds);
        let join = tokio::spawn(async move {
            run_autotune_controller_session(monitor_config, runtime_config, Some(stop_rx), duration)
                .await
        });
        AutotuneControllerHandle {
            mode: policy.mode.as_str().to_owned(),
            watch_process: request.watch_process.clone(),
            tree_pid: request.tree_pid,
            started_unix_nanos,
            stop_tx,
            join,
        }
    };

    let daemon_state = daemon_state_for_autotune_start(&policy, &request, started_unix_nanos);
    *active = Some(handle);
    replace_agent_daemon_state(&state, daemon_state).await;

    audit_agent_event_with_safety(
        "remote-autotune-start",
        safety_class_for_daemon_mode(policy.mode),
        true,
        0,
        format!(
            "daemon_mode={} action_source={:?} allow_system_wide_suggestions={} allow_system_wide_apply={} allow_high_risk={} watch_process={:?} tree_pid={:?}",
            policy.mode.as_str(),
            policy.source,
            policy.allow_system_wide_suggestions,
            policy.allow_system_wide_apply,
            policy.allow_high_risk,
            request.watch_process.as_deref().unwrap_or("none"),
            request.tree_pid
        ),
    );

    Json(AutotuneStartResponse {
        status: "started".to_owned(),
        mode,
        message: remote_autotune_start_message(policy.mode).to_owned(),
    })
    .into_response()
}

pub(crate) async fn autotune_stop_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) =
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::ControlDaemon)
    {
        return status.into_response();
    }

    let handle = {
        let mut active = state.active_autotune.lock().await;
        active.take()
    };

    let Some(handle) = handle else {
        audit_agent_event(
            "remote-autotune-stop",
            true,
            0,
            "active_before_stop=false".to_owned(),
        );

        replace_agent_daemon_state(
            &state,
            daemon_state_for_agent_decision(
                DaemonMode::Observe,
                DaemonPhase::Disabled,
                "autotune_stop_no_active_session",
                "no remote autotune session was active",
            ),
        )
        .await;

        return Json(AutotuneStopResponse {
            status: "no_active_session".to_owned(),
            message: "no remote autotune session was active".to_owned(),
        })
        .into_response();
    };

    let stop_mode = daemon_mode_from_agent_mode(&handle.mode);
    let _ = handle.stop_tx.send(());

    match handle.join.await {
        Ok(Ok(exit)) => {
            audit_agent_event(
                "remote-autotune-stop",
                true,
                0,
                format!("active_before_stop=true reason={}", exit.reason),
            );

            replace_agent_daemon_state(&state, daemon_state_for_autotune_stop(stop_mode, &exit))
                .await;

            Json(AutotuneStopResponse {
                status: "stopped".to_owned(),
                message: format!("remote autotune session stopped: {}", exit.reason),
            })
            .into_response()
        }
        Ok(Err(err)) => {
            audit_agent_event(
                "remote-autotune-stop",
                false,
                0,
                format!("active_before_stop=true error={err:#}"),
            );

            mark_agent_daemon_fault(
                &state,
                stop_mode,
                "autotune_controller_failed",
                format!("autotune controller failed: {err:#}"),
            )
            .await;

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("autotune controller failed: {err:#}"),
                }),
            )
                .into_response()
        }
        Err(err) => {
            audit_agent_event(
                "remote-autotune-stop",
                false,
                0,
                format!("active_before_stop=true join_error={err}"),
            );

            mark_agent_daemon_fault(
                &state,
                stop_mode,
                "autotune_controller_join_failed",
                format!("autotune controller task join failed: {err}"),
            )
            .await;

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("autotune controller task join failed: {err}"),
                }),
            )
                .into_response()
        }
    }
}

pub(crate) async fn autotune_restore_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) =
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::RollbackAction)
    {
        audit_agent_event(
            "remote-autotune-restore",
            false,
            0,
            "unauthorized".to_owned(),
        );
        return status.into_response();
    }

    autotune_restore_authorized(
        state,
        AutotuneRestoreCommandInput {
            journal_path: None,
            audit_path: None,
            history_path: None,
            dry_run: false,
        },
    )
    .await
}

pub(crate) async fn autotune_restore_authorized(
    state: Arc<AgentState>,
    input: AutotuneRestoreCommandInput,
) -> axum::response::Response {
    let outcome = match restore_known_autotune_actions(input) {
        Ok(outcome) => outcome,
        Err(err) => {
            audit_agent_event(
                "remote-autotune-restore",
                false,
                0,
                format!("restore_failed error={err:#}"),
            );
            mark_agent_daemon_fault(
                &state,
                DaemonMode::ApplyLowRisk,
                "remote_autotune_restore_failed",
                format!("remote autotune restore failed: {err:#}"),
            )
            .await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("remote autotune restore failed: {err:#}"),
                }),
            )
                .into_response();
        }
    };

    let ok = matches!(
        &outcome.status,
        AutotuneRestoreStatus::Clean | AutotuneRestoreStatus::Restored
    );
    let status = match &outcome.status {
        AutotuneRestoreStatus::Clean => "nothing_to_restore",
        AutotuneRestoreStatus::Restored => "restored",
        AutotuneRestoreStatus::DryRun => "dry_run",
        AutotuneRestoreStatus::ApplyingWithoutRollbackToken | AutotuneRestoreStatus::Faulted => {
            "restore_failed"
        }
    };
    let message = format!(
        "remote autotune restore status={:?} restored_actions={} skipped_actions={} failed_actions={}",
        outcome.status, outcome.restored_actions, outcome.skipped_actions, outcome.failed_actions
    );

    audit_agent_event(
        "remote-autotune-restore",
        ok,
        outcome.restored_actions,
        message.clone(),
    );

    if ok {
        let decision = match outcome.status {
            AutotuneRestoreStatus::Clean => "remote_autotune_nothing_to_restore",
            _ => "remote_autotune_restored",
        };
        let mut daemon_state = state.daemon_state.lock().await;
        daemon_state.phase = DaemonPhase::Observe;
        daemon_state.active_experiment = None;
        daemon_state.active_rollback = None;
        daemon_state.faulted = None;
        daemon_state.degraded.clear();
        daemon_state.last_decision = Some(daemon_decision_state(decision, message.clone()));
    } else {
        mark_agent_daemon_fault(
            &state,
            DaemonMode::ApplyLowRisk,
            "remote_autotune_restore_incomplete",
            message.clone(),
        )
        .await;
    }

    let http_status = if ok {
        StatusCode::OK
    } else {
        StatusCode::CONFLICT
    };
    (
        http_status,
        Json(AutotuneRestoreResponse {
            status: status.to_owned(),
            message,
            restored_actions: Some(outcome.restored_actions),
            skipped_actions: Some(outcome.skipped_actions),
            failed_actions: Some(outcome.failed_actions),
            restored_records: Some(outcome.restored_records),
            skipped_missing: Some(outcome.skipped_missing),
            skipped_identity_mismatch: Some(outcome.skipped_identity_mismatch),
            failed_records: Some(outcome.failed_records),
            restore_messages: outcome.messages,
        }),
    )
        .into_response()
}

pub(crate) async fn autotune_history_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    let path = crate::autotune::history::default_autotune_history_path();
    let events = match crate::autotune::history::read_autotune_history_events(&path) {
        Ok(events) => events,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to read autotune history: {err:#}"),
                }),
            )
                .into_response();
        }
    };

    let values = events
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>();

    let events = match values {
        Ok(values) => values,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("failed to encode autotune history: {err}"),
                }),
            )
                .into_response();
        }
    };

    Json(AutotuneHistoryResponse {
        path: path.display().to_string(),
        events,
    })
    .into_response()
}

pub(crate) async fn autotune_config_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    Json(AutotuneConfigResponse {
        default_mode: DaemonMode::Observe.as_str().to_owned(),
        supported_modes: supported_remote_mode_labels(&state.autotune_limits),
        apply_low_risk_remote_enabled: remote_mode_supported(
            &state.autotune_limits,
            DaemonMode::ApplyLowRisk,
        ),
        local_only_by_default: true,
        history_path: crate::autotune::history::default_autotune_history_path()
            .display()
            .to_string(),
        autotune_limits: state.autotune_limits.clone(),
        daemon_scope: "remote-agent".to_owned(),
        allow_system_wide_suggestions: state.autotune_limits.allow_system_wide_suggestions,
        allow_system_wide_apply: state.autotune_limits.allow_system_wide_apply,
        minimum_focus_confidence: 0.70,
        required_stable_focus_polls: 3,
    })
    .into_response()
}
