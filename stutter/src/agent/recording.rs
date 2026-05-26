//! Recording HTTP handler boundary.

use super::{
    autotune::{
        daemon_state_for_agent_decision, mark_agent_daemon_fault, replace_agent_daemon_state,
    },
    *,
};

pub(crate) async fn start_record_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
    Json(request): Json<RemoteMonitorRequest>,
) -> impl IntoResponse {
    if let Err(status) =
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::StartRecording)
    {
        audit_agent_event("remote-record-start", false, 0, "unauthorized".to_owned());
        mark_agent_daemon_fault(
            &state,
            DaemonMode::Observe,
            "record_start_rejected",
            "unauthorized",
        )
        .await;
        return status.into_response();
    }

    if let Err(status) = validate_foreground_title_capture_security(&request, &state, &headers) {
        audit_agent_event(
            "remote-record-start",
            false,
            0,
            "rejected foreground_include_title request: title capture requires loopback bind or valid bearer token".to_owned(),
        );
        return (
            status,
            Json(ErrorResponse {
                error: "foreground_include_title requires a loopback-bound agent or a valid bearer token".to_owned(),
            }),
        )
            .into_response();
    }

    if let Err(e) = validate_remote_request_limits(&request, &state.limits) {
        audit_agent_event(
            "remote-record-start",
            false,
            0,
            format!("limit_rejected reason={e}"),
        );
        mark_agent_daemon_fault(
            &state,
            DaemonMode::Observe,
            "record_limit_rejected",
            format!("limit rejected: {e}"),
        )
        .await;
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("limit rejected: {e}"),
            }),
        )
            .into_response();
    }

    reap_finished_recording(&state).await;

    let mut active = state.active_run.lock().await;
    if active.is_some() {
        audit_agent_event(
            "remote-record-start",
            false,
            0,
            "conflict: recording already active".to_owned(),
        );
        mark_agent_daemon_fault(
            &state,
            DaemonMode::Observe,
            "record_start_conflict",
            "a recording run is already active",
        )
        .await;
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "a recording run is already active".to_owned(),
            }),
        )
            .into_response();
    }

    let run_id = new_run_id();
    let run_dir = state.runs_dir.join(&run_id);

    let monitor_config = match monitor_config_from_remote_request(&request, &run_dir, &state.limits)
    {
        Ok(c) => c,
        Err(e) => {
            audit_agent_event(
                "remote-record-start",
                false,
                0,
                format!("config_error reason={e}"),
            );
            mark_agent_daemon_fault(
                &state,
                DaemonMode::Observe,
                "record_config_error",
                format!("invalid configuration: {e}"),
            )
            .await;
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("invalid configuration: {e}"),
                }),
            )
                .into_response();
        }
    };

    let target_count =
        request.target_pids.len() + request.tree_pids.len() + request.exclude_tree_pids.len();

    audit_agent_event(
        "remote-record-start",
        true,
        target_count,
        format!(
            "run_id={} duration={} targets={} hwmon={} cpu_freq={} faults={} stat_wait={} block_io={} runtime_slices={} irq_latency={} foreground_window={} focus_source={:?} foreground_source={:?} foreground_include_title={}",
            run_id,
            request
                .duration_seconds
                .unwrap_or(state.limits.max_duration_seconds),
            target_count,
            request.hwmon,
            request.cpu_freq,
            request.faults,
            request.stat_wait,
            request.block_io,
            request.runtime_slices,
            request.irq_latency,
            request.foreground_window,
            request.focus_source,
            request.foreground_source,
            request.foreground_include_title
        ),
    );

    let (stop_tx, stop_rx) = oneshot::channel();
    let monitor_config = Arc::new(monitor_config);

    // The agent owns only the cancellation sender and join handle; the monitor session owns its
    // mutable recording state inside the spawned task.
    let join = tokio::spawn(async move {
        crate::session::run_monitor(monitor_config, None, None, Some(stop_rx)).await
    });

    *active = Some(RunHandle {
        id: run_id.clone(),
        stop_tx,
        join,
    });
    replace_agent_daemon_state(
        &state,
        daemon_state_for_record_start(&request, &run_id, target_count),
    )
    .await;

    (
        StatusCode::OK,
        Json(StartRecordResponse {
            run_id,
            status: "started".to_owned(),
        }),
    )
        .into_response()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecordingReapStatus {
    NoActiveRun,
    StillActive,
    Completed,
    Failed,
}

pub(crate) async fn reap_finished_recording(state: &AgentState) -> RecordingReapStatus {
    let handle = {
        let mut active = state.active_run.lock().await;
        match active.as_ref() {
            Some(handle) if handle.join.is_finished() => active.take(),
            Some(_) => return RecordingReapStatus::StillActive,
            None => return RecordingReapStatus::NoActiveRun,
        }
    };

    let Some(handle) = handle else {
        return RecordingReapStatus::NoActiveRun;
    };

    let run_id = handle.id.clone();
    match handle.join.await {
        Ok(Ok(_)) => {
            audit_agent_event(
                "remote-record-reap",
                true,
                0,
                format!("run_id={run_id} completed"),
            );
            replace_agent_daemon_state(
                state,
                daemon_state_for_agent_decision(
                    DaemonMode::Observe,
                    DaemonPhase::Disabled,
                    "record_completed",
                    format!("run_id={run_id}"),
                ),
            )
            .await;
            RecordingReapStatus::Completed
        }
        Ok(Err(err)) => {
            audit_agent_event(
                "remote-record-reap",
                false,
                0,
                format!("run_id={run_id} error={err:#}"),
            );
            mark_agent_daemon_fault(
                state,
                DaemonMode::Observe,
                "record_failed",
                format!("run_id={run_id} error={err:#}"),
            )
            .await;
            RecordingReapStatus::Failed
        }
        Err(err) => {
            audit_agent_event(
                "remote-record-reap",
                false,
                0,
                format!("run_id={run_id} join_error={err}"),
            );
            mark_agent_daemon_fault(
                state,
                DaemonMode::Observe,
                "record_join_failed",
                format!("run_id={run_id} join_error={err}"),
            )
            .await;
            RecordingReapStatus::Failed
        }
    }
}

pub(crate) async fn stop_record_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) =
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::StopRecording)
    {
        return status.into_response();
    }

    let handle = {
        let mut active = state.active_run.lock().await;
        active.take()
    };

    if let Some(handle) = handle {
        let _ = handle.stop_tx.send(());
        let run_id = handle.id.clone();
        match handle.join.await {
            Ok(Ok(_)) => {
                audit_agent_event("remote-record-stop", true, 0, format!("run_id={run_id}"));
                replace_agent_daemon_state(
                    &state,
                    daemon_state_for_agent_decision(
                        DaemonMode::Observe,
                        DaemonPhase::Disabled,
                        "record_stopped",
                        format!("run_id={run_id}"),
                    ),
                )
                .await;

                (
                    StatusCode::OK,
                    Json(StopRecordResponse {
                        run_id: Some(run_id),
                        status: "stopped".to_owned(),
                    }),
                )
                    .into_response()
            }
            Ok(Err(e)) => {
                audit_agent_event(
                    "remote-record-stop",
                    false,
                    0,
                    format!("run_id={run_id} error={e}"),
                );
                mark_agent_daemon_fault(
                    &state,
                    DaemonMode::Observe,
                    "record_failed",
                    format!("run_id={run_id} error={e}"),
                )
                .await;

                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("recording failed: {e}"),
                    }),
                )
                    .into_response()
            }
            Err(e) => {
                audit_agent_event(
                    "remote-record-stop",
                    false,
                    0,
                    format!("run_id={run_id} join_error={e}"),
                );
                mark_agent_daemon_fault(
                    &state,
                    DaemonMode::Observe,
                    "record_join_failed",
                    format!("run_id={run_id} join_error={e}"),
                )
                .await;

                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("task join failed: {e}"),
                    }),
                )
                    .into_response()
            }
        }
    } else {
        replace_agent_daemon_state(
            &state,
            daemon_state_for_agent_decision(
                DaemonMode::Observe,
                DaemonPhase::Disabled,
                "record_stop_no_active_run",
                "no active recording run",
            ),
        )
        .await;

        (
            StatusCode::OK,
            Json(StopRecordResponse {
                run_id: None,
                status: "no_active_run".to_owned(),
            }),
        )
            .into_response()
    }
}

pub(crate) async fn status_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    reap_finished_recording(&state).await;

    let active = state.active_run.lock().await;
    Json(RecordStatusResponse {
        active: active.is_some(),
        run_id: active.as_ref().map(|h| h.id.clone()),
    })
    .into_response()
}

pub(crate) async fn list_runs_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }
    let mut runs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&state.runs_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                runs.push(name.to_owned());
            }
        }
    }
    runs.sort_by(|a, b| b.cmp(a)); // Newest first
    Json(RunsResponse { runs }).into_response()
}

pub(crate) async fn get_session_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }
    if let Err(e) = validate_id(&id) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

    let path = crate::artifacts::artifact_path(
        &state.runs_dir.join(&id),
        crate::artifacts::ArtifactKind::Session,
    );
    if !path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }

    match std::fs::read_to_string(path) {
        Ok(json) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            json,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) async fn get_artifact_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }
    if let Err(e) = validate_id(&id) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    if let Err(e) = validate_artifact_name(&name) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

    let path = state.runs_dir.join(&id).join(&name);
    if !path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }

    match std::fs::read(&path) {
        Ok(data) => (
            [(
                axum::http::header::CONTENT_TYPE,
                artifact_content_type(&name),
            )],
            data,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(crate) fn validate_artifact_name(name: &str) -> anyhow::Result<()> {
    validate_id(name)?;
    if !AGENT_ARTIFACT_ALLOWLIST.contains(&name) {
        anyhow::bail!("artifact is not downloadable");
    }
    Ok(())
}

pub(crate) fn artifact_content_type(name: &str) -> &'static str {
    if name.ends_with(".json") {
        "application/json"
    } else if name.ends_with("_events.json") || name.ends_with("_samples.json") {
        "application/x-ndjson"
    } else {
        "application/octet-stream"
    }
}

pub(crate) fn parse_remote_focus_source(value: Option<&str>) -> anyhow::Result<FocusSource> {
    match value
        .unwrap_or("heuristic")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "heuristic" => Ok(FocusSource::Heuristic),
        "foreground" => Ok(FocusSource::Foreground),
        "hybrid" => Ok(FocusSource::Hybrid),
        other => {
            anyhow::bail!("focus_source must be heuristic, foreground, or hybrid, got {other:?}")
        }
    }
}

pub(crate) fn parse_remote_foreground_source(
    value: Option<&str>,
) -> anyhow::Result<ForegroundSource> {
    match value.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(ForegroundSource::Auto),
        "sway" => Ok(ForegroundSource::Sway),
        "hyprland" => Ok(ForegroundSource::Hyprland),
        "x11" => Ok(ForegroundSource::X11),
        other => {
            anyhow::bail!("foreground_source must be auto, sway, hyprland, or x11, got {other:?}")
        }
    }
}

pub(crate) fn remote_foreground_enabled(request: &RemoteMonitorRequest) -> anyhow::Result<bool> {
    let focus_source = parse_remote_focus_source(request.focus_source.as_deref())?;
    Ok(request.foreground_window || focus_source != FocusSource::Heuristic)
}

pub(crate) fn validate_remote_foreground_request(
    request: &RemoteMonitorRequest,
) -> anyhow::Result<()> {
    let _foreground_enabled = remote_foreground_enabled(request)?;
    let _foreground_source = parse_remote_foreground_source(request.foreground_source.as_deref())?;

    if let Some(foreground_poll_ms) = request.foreground_poll_ms
        && foreground_poll_ms < 100
    {
        anyhow::bail!("foreground_poll_ms must be >= 100");
    }

    if let (Some(max_stale), Some(poll)) =
        (request.foreground_max_stale_ms, request.foreground_poll_ms)
        && max_stale < poll
    {
        log::warn!(
            "remote foreground max stale is lower than poll interval; provider errors may clear focus quickly"
        );
    }

    Ok(())
}

pub(crate) fn validate_foreground_title_capture_security(
    request: &RemoteMonitorRequest,
    state: &AgentState,
    headers: &HeaderMap,
) -> Result<(), StatusCode> {
    if !request.foreground_include_title {
        return Ok(());
    }

    if agent_state_is_local(state) {
        return Ok(());
    }

    if !state.auth.apply_token_configured() {
        return Err(StatusCode::FORBIDDEN);
    }

    authorize_apply(headers, &state.auth)
}

pub(crate) fn validate_remote_request_limits(
    request: &RemoteMonitorRequest,
    limits: &AgentLimits,
) -> anyhow::Result<()> {
    let duration = request
        .duration_seconds
        .unwrap_or(limits.max_duration_seconds);
    if duration == 0 {
        anyhow::bail!("duration_seconds must be greater than zero");
    }
    if duration > limits.max_duration_seconds {
        anyhow::bail!(
            "duration_seconds {} exceeds agent max_duration_seconds {}",
            duration,
            limits.max_duration_seconds
        );
    }

    let target_count =
        request.target_pids.len() + request.tree_pids.len() + request.exclude_tree_pids.len();

    if target_count == 0 {
        anyhow::bail!("remote recording requires at least one target pid or tree pid");
    }

    if target_count > limits.max_targets {
        anyhow::bail!(
            "target count {} exceeds agent max_targets {}",
            target_count,
            limits.max_targets
        );
    }

    if request.summary_ms == Some(0) {
        anyhow::bail!("summary_ms must be greater than zero");
    }
    if request.runtime_slices_max_tasks == Some(0) {
        anyhow::bail!("runtime_slices_max_tasks must be greater than zero");
    }

    if request.spike_us == Some(0) {
        anyhow::bail!("spike_us must be greater than zero");
    }

    if request.irq_latency && request.irqs.is_empty() {
        anyhow::bail!("irq_latency requires explicit IRQ list");
    }

    validate_remote_foreground_request(request)?;

    Ok(())
}

pub(crate) fn validate_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        anyhow::bail!("invalid id");
    }
    Ok(())
}

pub(crate) fn new_run_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("run-{}", now.as_nanos())
}

pub(crate) fn monitor_config_from_remote_request(
    request: &RemoteMonitorRequest,
    run_dir: &StdPath,
    limits: &AgentLimits,
) -> anyhow::Result<MonitorConfig> {
    use crate::config::{
        TARGET_PIDS_MAX,
        effective::resolve_monitor_config_sources,
        merge::{ApiOverrides, ConfigSources, DefaultConfig},
    };

    let duration_seconds = request
        .duration_seconds
        .unwrap_or(limits.max_duration_seconds);

    let mut api_layer = request.clone().into_monitor_config_layer()?;
    api_layer.max_duration = Some(Some(std::time::Duration::from_secs(duration_seconds)));
    api_layer.max_tasks = (limits.max_targets != TARGET_PIDS_MAX).then_some(limits.max_targets);

    if request.record {
        api_layer.output_dir = Some(Some(run_dir.to_path_buf()));
    }

    let user_file =
        crate::config_file::load_user_config().map_err(crate::error::ConfigError::UserConfig)?;

    let resolved = resolve_monitor_config_sources(ConfigSources {
        defaults: DefaultConfig {
            config: MonitorConfig::default(),
        },
        user_file,
        preset: None,
        overrides: ApiOverrides { layer: api_layer }.into(),
    })?;

    Ok(resolved.config)
}
