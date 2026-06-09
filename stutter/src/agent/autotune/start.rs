//! Remote autotune start endpoint.

use super::{
    policy::{
        parse_focus_source_or_hybrid, parse_foreground_source_or_auto,
        policy_for_remote_autotune_start_with_safety_context, safety_class_for_daemon_mode,
        validate_autotune_start_limits,
    },
    reap::reap_finished_autotune,
    state::{
        apply_remote_autotune_runtime_mode_overrides, daemon_mode_from_agent_mode,
        daemon_state_for_autotune_start, mark_agent_daemon_fault,
        mark_agent_daemon_policy_rejection, remote_autotune_controller_duration,
        remote_autotune_start_message, replace_agent_daemon_state,
    },
    *,
};

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
        mode: policy.mode,
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
        // Remote handlers keep the cancellation sender and join handle; the controller task owns
        // runtime state and reports completion through the join result.
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
