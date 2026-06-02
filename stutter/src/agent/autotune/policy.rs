//! Remote autotune policy and mode validation helpers.

use super::*;

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
        Some("gnome") => ForegroundSource::Gnome,
        Some("kde") | Some("plasma") => ForegroundSource::Kde,
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
