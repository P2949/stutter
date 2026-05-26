//! Agent route construction boundary.

use super::*;

pub(crate) fn build_agent_router(
    state: Arc<AgentState>,
    request_rate_limiter: Arc<AgentRateLimiter>,
) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/version", get(version_handler))
        .route("/capabilities", get(capabilities_handler))
        .route("/record/start", post(recording::start_record_handler))
        .route("/record/stop", post(recording::stop_record_handler))
        .route("/record/status", get(recording::status_handler))
        .route("/autotune/status", get(autotune::autotune_status_handler))
        .route("/autotune/start", post(autotune::autotune_start_handler))
        .route("/autotune/stop", post(autotune::autotune_stop_handler))
        .route(
            "/autotune/restore",
            post(autotune::autotune_restore_handler),
        )
        .route("/autotune/history", get(autotune::autotune_history_handler))
        .route("/autotune/config", get(autotune::autotune_config_handler))
        .route("/daemon/status", get(daemon::daemon_status_handler))
        .route("/daemon/health", get(daemon::daemon_health_handler))
        .route("/daemon/policy", get(daemon::daemon_policy_handler))
        .route("/daemon/explain", get(daemon::daemon_explain_handler))
        .route("/daemon/pause", post(daemon::daemon_pause_handler))
        .route("/daemon/resume", post(daemon::daemon_resume_handler))
        .route("/daemon/restore", post(daemon::daemon_restore_handler))
        .route("/runs", get(recording::list_runs_handler))
        .route(
            "/runs/:id/session.json",
            get(recording::get_session_handler),
        )
        .route(
            "/runs/:id/artifact/:name",
            get(recording::get_artifact_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            request_rate_limiter,
            server::agent_request_guard,
        ))
        .layer(DefaultBodyLimit::max(DEFAULT_AGENT_MAX_REQUEST_BYTES))
        .with_state(state)
}

pub(crate) async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

pub(crate) async fn version_handler() -> impl IntoResponse {
    Json(version_response())
}

pub(crate) async fn capabilities_handler(
    State(state): State<Arc<AgentState>>,
) -> impl IntoResponse {
    Json(capabilities_response(&state))
}

pub(crate) fn version_response() -> VersionResponse {
    VersionResponse {
        name: "stutter".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_version: crate::recorder::SESSION_SCHEMA_VERSION.get(),
    }
}

pub(crate) fn capabilities_response(state: &AgentState) -> CapabilitiesResponse {
    CapabilitiesResponse {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        auth_required: state.auth.any_token_configured(),
        max_duration_seconds: state.limits.max_duration_seconds,
        max_targets: state.limits.max_targets,
        max_concurrent_recordings: state.limits.max_concurrent_recordings,
        supported_routes: vec![
            "/health".to_owned(),
            "/version".to_owned(),
            "/capabilities".to_owned(),
            "/record/start".to_owned(),
            "/record/stop".to_owned(),
            "/record/status".to_owned(),
            "/autotune/status".to_owned(),
            "/autotune/start".to_owned(),
            "/autotune/stop".to_owned(),
            "/autotune/restore".to_owned(),
            "/autotune/history".to_owned(),
            "/autotune/config".to_owned(),
            "/daemon/status".to_owned(),
            "/daemon/health".to_owned(),
            "/daemon/policy".to_owned(),
            "/daemon/explain".to_owned(),
            "/daemon/pause".to_owned(),
            "/daemon/resume".to_owned(),
            "/daemon/restore".to_owned(),
            "/runs".to_owned(),
            "/runs/:id/session.json".to_owned(),
            "/runs/:id/artifact/:name".to_owned(),
        ],
        supported_artifacts: AGENT_ARTIFACT_ALLOWLIST
            .iter()
            .map(|s| s.to_string())
            .collect(),
        features: AgentFeatureFlags {
            record_start_stop: true,
            list_runs: true,
            download_session: true,
            download_artifacts: true,
            hwmon_request: true,
            cpu_freq_request: true,
            faults_request: true,
            stat_wait_request: true,
            block_io_request: true,
            irq_latency_request: true,
            foreground_window_request: true,
            autotune_observe: autotune::remote_mode_supported(
                &state.autotune_limits,
                DaemonMode::Observe,
            ),
            autotune_suggest: autotune::remote_mode_supported(
                &state.autotune_limits,
                DaemonMode::Suggest,
            ),
            autotune_apply_low_risk: autotune::remote_mode_supported(
                &state.autotune_limits,
                DaemonMode::ApplyLowRisk,
            ),
        },
    }
}
