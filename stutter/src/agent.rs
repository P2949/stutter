use std::{
    net::SocketAddr,
    path::{Path as StdPath, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::Serialize;
use tokio::{
    sync::{Mutex, oneshot},
    task::JoinHandle,
};

use crate::{
    cli::Config,
    remote::{
        AgentFeatureFlags, CapabilitiesResponse, HealthResponse, RecordStatusResponse,
        RemoteMonitorRequest, RunsResponse, StartRecordResponse, StopRecordResponse,
        VersionResponse,
    },
};

pub const DEFAULT_AGENT_MAX_DURATION_SECONDS: u64 = 300;
pub const DEFAULT_AGENT_MAX_TARGETS: usize = 128;
pub const DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS: usize = 1;

pub(crate) const AGENT_ARTIFACT_ALLOWLIST: &[&str] = &[
    "metadata.json",
    "session.json",
    "interval.json",
    "spike_events.json",
    "tree_events.json",
    "irq_events.json",
    "gpu_samples.json",
    "frame_correlation.json",
    "frame_events.json",
    "migration_events.json",
    "cpu_freq_samples.json",
    "io_events.json",
    "scx_events.json",
];

#[derive(Clone, Debug)]
pub struct AgentConfig {
    pub bind: SocketAddr,
    pub runs_dir: PathBuf,
    pub allow_unsafe_bind: bool,
    pub bearer_token: Option<String>,
    pub max_duration_seconds: u64,
    pub max_targets: usize,
    pub max_concurrent_recordings: usize,
}

#[derive(Clone, Debug)]
pub struct AgentLimits {
    pub max_duration_seconds: u64,
    pub max_targets: usize,
    pub max_concurrent_recordings: usize,
}

#[derive(Clone, Debug)]
pub struct AgentAuth {
    pub bearer_token: Option<String>,
}

pub struct AgentState {
    pub active_run: Mutex<Option<RunHandle>>,
    pub runs_dir: PathBuf,
    pub auth: AgentAuth,
    pub limits: AgentLimits,
}

pub struct RunHandle {
    pub id: String,
    pub stop_tx: oneshot::Sender<()>,
    pub join: JoinHandle<anyhow::Result<String>>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

pub async fn run_agent(config: AgentConfig) -> anyhow::Result<()> {
    if !is_local_bind(&config.bind) && !config.allow_unsafe_bind {
        anyhow::bail!(
            "refusing to bind stutter agent to non-loopback address {}; \
             pass --allow-unsafe-bind only on trusted networks",
            config.bind
        );
    }

    if !config.runs_dir.exists() {
        std::fs::create_dir_all(&config.runs_dir)?;
    }

    if !is_local_bind(&config.bind) {
        if config.bearer_token.is_none() {
            println!(
                "WARNING: stutter agent is listening on a non-loopback address without authentication because --allow-unsafe-bind was passed."
            );
        } else {
            println!(
                "WARNING: stutter agent is listening on a non-loopback address. Authentication is enabled."
            );
        }
    }

    let auth_enabled = config.bearer_token.is_some();

    audit_agent_event(
        "remote-agent-listen",
        true,
        0,
        format!(
            "bind={} auth_enabled={} allow_unsafe_bind={} runs_dir={}",
            config.bind,
            auth_enabled,
            config.allow_unsafe_bind,
            config.runs_dir.display()
        ),
    );

    let state = Arc::new(AgentState {
        active_run: Mutex::new(None),
        runs_dir: config.runs_dir,
        auth: AgentAuth {
            bearer_token: config.bearer_token,
        },
        limits: AgentLimits {
            max_duration_seconds: config.max_duration_seconds,
            max_targets: config.max_targets,
            max_concurrent_recordings: config.max_concurrent_recordings,
        },
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/version", get(version_handler))
        .route("/capabilities", get(capabilities_handler))
        .route("/record/start", post(start_record_handler))
        .route("/record/stop", post(stop_record_handler))
        .route("/record/status", get(status_handler))
        .route("/runs", get(list_runs_handler))
        .route("/runs/:id/session.json", get(get_session_handler))
        .route("/runs/:id/artifact/:name", get(get_artifact_handler))
        .with_state(state);

    println!("stutter agent listening on http://{}", config.bind);
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

pub fn default_runs_dir() -> PathBuf {
    let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    p.push("stutter-runs");
    p
}

pub fn load_bearer_token(
    bearer_token_env: &str,
    bearer_token_file: Option<&StdPath>,
) -> anyhow::Result<Option<String>> {
    if let Some(path) = bearer_token_file {
        let token = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read bearer token file {}", path.display()))?;
        return normalize_bearer_token(token);
    }

    if let Some(value) = std::env::var_os(bearer_token_env) {
        return normalize_bearer_token(value.to_string_lossy().into_owned());
    }

    Ok(None)
}

fn normalize_bearer_token(raw: String) -> anyhow::Result<Option<String>> {
    let token = raw.trim().to_owned();
    if token.is_empty() {
        anyhow::bail!("bearer token is empty");
    }
    Ok(Some(token))
}

fn authorize(headers: &HeaderMap, auth: &AgentAuth) -> Result<(), StatusCode> {
    let Some(expected) = auth.bearer_token.as_deref() else {
        return Ok(());
    };

    let Some(header) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let Ok(value) = header.to_str() else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let Some(token) = value.strip_prefix("Bearer ") else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    if constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn audit_agent_event(
    action_id: &'static str,
    success: bool,
    affected_tasks: usize,
    message: String,
) {
    crate::audit::audit_or_warn(&crate::audit::AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "agent".to_owned(),
        action_id: Some(action_id.to_owned()),
        safety_class: Some(crate::actions::SafetyClass::HighRisk),
        dry_run: false,
        success,
        affected_tasks,
        restore_path: None,
        message,
    });
}

fn is_local_bind(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION").to_owned(),
    })
}

async fn start_record_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
    Json(request): Json<RemoteMonitorRequest>,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        audit_agent_event("remote-record-start", false, 0, "unauthorized".to_owned());
        return status.into_response();
    }

    if let Err(e) = validate_remote_request_limits(&request, &state.limits) {
        audit_agent_event(
            "remote-record-start",
            false,
            0,
            format!("limit_rejected reason={e}"),
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("limit rejected: {e}"),
            }),
        )
            .into_response();
    }

    let mut active = state.active_run.lock().await;
    if active.is_some() {
        audit_agent_event(
            "remote-record-start",
            false,
            0,
            "conflict: recording already active".to_owned(),
        );
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

    let config = match config_from_remote_request(&request, &run_dir, &state.limits) {
        Ok(c) => c,
        Err(e) => {
            audit_agent_event(
                "remote-record-start",
                false,
                0,
                format!("config_error reason={e}"),
            );
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
            "run_id={} duration={} targets={} hwmon={} cpu_freq={} faults={} stat_wait={} block_io={} irq_latency={}",
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
            request.irq_latency
        ),
    );

    let (stop_tx, stop_rx) = oneshot::channel();
    let config_arc = Arc::new(config);

    let join =
        tokio::spawn(
            async move { crate::session::run_monitor(config_arc, None, Some(stop_rx)).await },
        );

    *active = Some(RunHandle {
        id: run_id.clone(),
        stop_tx,
        join,
    });

    (
        StatusCode::OK,
        Json(StartRecordResponse {
            run_id,
            status: "started".to_owned(),
        }),
    )
        .into_response()
}

async fn stop_record_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    let mut active = state.active_run.lock().await;
    let handle = active.take();

    if let Some(handle) = handle {
        let _ = handle.stop_tx.send(());
        let run_id = handle.id.clone();
        match handle.join.await {
            Ok(Ok(_)) => {
                audit_agent_event("remote-record-stop", true, 0, format!("run_id={run_id}"));
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

async fn status_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }
    let active = state.active_run.lock().await;
    Json(RecordStatusResponse {
        active: active.is_some(),
        run_id: active.as_ref().map(|h| h.id.clone()),
    })
    .into_response()
}

async fn list_runs_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }
    let mut runs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&state.runs_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                #[allow(clippy::collapsible_if)]
                if let Some(name) = entry.file_name().to_str() {
                    runs.push(name.to_owned());
                }
            }
        }
    }
    runs.sort_by(|a, b| b.cmp(a)); // Newest first
    Json(RunsResponse { runs }).into_response()
}

async fn get_session_handler(
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

    let path = state.runs_dir.join(&id).join("session.json");
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

async fn get_artifact_handler(
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

async fn version_handler() -> impl IntoResponse {
    Json(version_response())
}

async fn capabilities_handler(State(state): State<Arc<AgentState>>) -> impl IntoResponse {
    Json(capabilities_response(&state))
}

fn version_response() -> VersionResponse {
    VersionResponse {
        name: "stutter".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        schema_version: crate::recorder::SESSION_SCHEMA_VERSION,
    }
}

fn capabilities_response(state: &AgentState) -> CapabilitiesResponse {
    CapabilitiesResponse {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        auth_required: state.auth.bearer_token.is_some(),
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
        },
    }
}

fn validate_artifact_name(name: &str) -> anyhow::Result<()> {
    validate_id(name)?;
    if !AGENT_ARTIFACT_ALLOWLIST.contains(&name) {
        anyhow::bail!("artifact is not downloadable");
    }
    Ok(())
}

fn artifact_content_type(name: &str) -> &'static str {
    if name.ends_with(".json") {
        "application/json"
    } else if name.ends_with("_events.json") || name.ends_with("_samples.json") {
        "application/x-ndjson"
    } else {
        "application/octet-stream"
    }
}

fn validate_remote_request_limits(
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

    if request.spike_us == Some(0) {
        anyhow::bail!("spike_us must be greater than zero");
    }

    if request.irq_latency && request.irqs.is_empty() {
        anyhow::bail!("irq_latency requires explicit IRQ list");
    }

    Ok(())
}

fn validate_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
        anyhow::bail!("invalid id");
    }
    Ok(())
}

fn new_run_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("run-{}", now.as_nanos())
}

fn config_from_remote_request(
    request: &RemoteMonitorRequest,
    run_dir: &StdPath,
    limits: &AgentLimits,
) -> anyhow::Result<Config> {
    use crate::{
        cli::RecordingConfig,
        process_tree::{CompiledPattern, TaskFilters},
    };

    let mut include_comm = Vec::new();
    for p in &request.include_comm {
        include_comm.push(CompiledPattern::new(p.clone())?);
    }
    let mut exclude_comm = Vec::new();
    for p in &request.exclude_comm {
        exclude_comm.push(CompiledPattern::new(p.clone())?);
    }

    Ok(Config {
        target_pids: request.target_pids.clone(),
        tree_pids: request.tree_pids.clone(),
        exclude_tree_pids: request.exclude_tree_pids.clone(),
        summary_period_ms: request.summary_ms.unwrap_or(1000),
        epoch_period_ms: None,
        spike_threshold_ns: request.spike_us.unwrap_or(1000) * 1000,
        alert_threshold_ns: None,
        alert_webhook_url: None,
        verbose: false,
        task_filters: TaskFilters {
            include_comm,
            exclude_comm,
        },
        keep_missing_pid: false,
        watch_process: None,
        persistent: false,
        watch_poll_ms: 2000,
        watch_timeout: None,
        max_tasks: limits.max_targets,
        csv_stream: None,
        irq_latency: request.irq_latency,
        irqs: request.irqs.clone(),
        hwmon: request.hwmon,
        hwmon_root: None,
        hwmon_drm_card: None,
        hwmon_render_node: None,
        mangohud_log: None,
        mangohud_log_live: false,
        tui: false,
        retain_intervals: None,
        recording: if request.record {
            Some(RecordingConfig {
                run_name: request.run_name.clone().or(Some("remote-run".to_owned())),
                out_dir: Some(run_dir.to_path_buf()),
            })
        } else {
            None
        },
        max_duration: Some(std::time::Duration::from_secs(
            request
                .duration_seconds
                .unwrap_or(limits.max_duration_seconds),
        )),
        cpu_freq: request.cpu_freq,
        cgroupv2: None,
        native_cgroup_filter: false,
        follow_exec: true,
        faults: request.faults,
        cpu_perf: false,
        cpu_perf_kernel: false,
        cpu_perf_max_tasks: 128,
        cpu_perf_cache_refs: false,
        block_io: request.block_io,
        stat_wait: request.stat_wait,
        json_stream: false,
        metrics_port: None,
        ringbuf_size_kb: None,
        wakeup_map_factor: None,
        otlp_endpoint: None,
        otel_service_name: "stutter".to_owned(),
        remote: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_id_rejects_path_traversal() {
        assert!(validate_id("../evil").is_err());
        assert!(validate_id("foo/bar").is_err());
        assert!(validate_id("run-123").is_ok());
    }

    #[test]
    fn artifact_allowlist_rejects_unknown_file() {
        assert!(validate_artifact_name("shadow").is_err());
        assert!(validate_artifact_name("session.json.bak").is_err());
    }

    #[test]
    fn artifact_allowlist_rejects_path_traversal() {
        assert!(validate_artifact_name("../session.json").is_err());
    }

    #[test]
    fn artifact_allowlist_accepts_known_artifacts() {
        assert!(validate_artifact_name("session.json").is_ok());
        assert!(validate_artifact_name("metadata.json").is_ok());
        assert!(validate_artifact_name("spike_events.json").is_ok());
    }

    #[test]
    fn local_bind_allowed_without_unsafe_flag() {
        let addr: SocketAddr = "127.0.0.1:9899".parse().unwrap();
        assert!(validate_agent_bind_policy(addr, false).is_ok());
    }

    #[test]
    fn non_loopback_bind_rejected_without_unsafe_flag() {
        let addr: SocketAddr = "0.0.0.0:9899".parse().unwrap();
        assert!(validate_agent_bind_policy(addr, false).is_err());
    }

    #[test]
    fn non_loopback_bind_allowed_with_unsafe_flag() {
        let addr: SocketAddr = "0.0.0.0:9899".parse().unwrap();
        assert!(validate_agent_bind_policy(addr, true).is_ok());
    }

    fn validate_agent_bind_policy(bind: SocketAddr, allow_unsafe_bind: bool) -> anyhow::Result<()> {
        if !is_local_bind(&bind) && !allow_unsafe_bind {
            anyhow::bail!("refusing to bind");
        }
        Ok(())
    }

    #[test]
    fn normalize_bearer_token_trims_newline() {
        assert_eq!(
            normalize_bearer_token("  secret\n  ".to_owned()).unwrap(),
            Some("secret".to_owned())
        );
    }

    #[test]
    fn normalize_bearer_token_rejects_empty() {
        assert!(normalize_bearer_token("  ".to_owned()).is_err());
    }

    #[test]
    fn authorize_allows_without_configured_token() {
        let auth = AgentAuth { bearer_token: None };
        let headers = HeaderMap::new();
        assert!(authorize(&headers, &auth).is_ok());
    }

    #[test]
    fn authorize_rejects_missing_bearer_token() {
        let auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
        };
        let headers = HeaderMap::new();
        assert_eq!(authorize(&headers, &auth), Err(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn authorize_rejects_wrong_bearer_token() {
        let auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer wrong".parse().unwrap(),
        );
        assert_eq!(authorize(&headers, &auth), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn authorize_accepts_correct_bearer_token() {
        let auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );
        assert!(authorize(&headers, &auth).is_ok());
    }

    #[test]
    fn remote_request_rejects_duration_above_limit() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let mut request = RemoteMonitorRequest {
            target_pids: vec![1],
            tree_pids: vec![],
            exclude_tree_pids: vec![],
            duration_seconds: Some(120),
            spike_us: Some(1000),
            summary_ms: Some(1000),
            include_comm: vec![],
            exclude_comm: vec![],
            hwmon: false,
            cpu_freq: false,
            faults: false,
            stat_wait: false,
            block_io: false,
            irq_latency: false,
            irqs: vec![],
            record: true,
            run_name: None,
        };
        assert!(validate_remote_request_limits(&request, &limits).is_err());

        request.duration_seconds = Some(30);
        assert!(validate_remote_request_limits(&request, &limits).is_ok());
    }

    #[test]
    fn remote_request_rejects_too_many_targets() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 2,
            max_concurrent_recordings: 1,
        };
        let request = RemoteMonitorRequest {
            target_pids: vec![1, 2, 3],
            tree_pids: vec![],
            exclude_tree_pids: vec![],
            duration_seconds: Some(30),
            spike_us: Some(1000),
            summary_ms: Some(1000),
            include_comm: vec![],
            exclude_comm: vec![],
            hwmon: false,
            cpu_freq: false,
            faults: false,
            stat_wait: false,
            block_io: false,
            irq_latency: false,
            irqs: vec![],
            record: true,
            run_name: None,
        };
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn remote_request_rejects_zero_summary_ms() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let request = RemoteMonitorRequest {
            target_pids: vec![1],
            tree_pids: vec![],
            exclude_tree_pids: vec![],
            duration_seconds: Some(30),
            spike_us: Some(1000),
            summary_ms: Some(0),
            include_comm: vec![],
            exclude_comm: vec![],
            hwmon: false,
            cpu_freq: false,
            faults: false,
            stat_wait: false,
            block_io: false,
            irq_latency: false,
            irqs: vec![],
            record: true,
            run_name: None,
        };
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn remote_request_rejects_zero_spike_us() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let request = RemoteMonitorRequest {
            target_pids: vec![1],
            tree_pids: vec![],
            exclude_tree_pids: vec![],
            duration_seconds: Some(30),
            spike_us: Some(0),
            summary_ms: Some(1000),
            include_comm: vec![],
            exclude_comm: vec![],
            hwmon: false,
            cpu_freq: false,
            faults: false,
            stat_wait: false,
            block_io: false,
            irq_latency: false,
            irqs: vec![],
            record: true,
            run_name: None,
        };
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn remote_request_rejects_irq_latency_without_irqs() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let request = RemoteMonitorRequest {
            target_pids: vec![1],
            tree_pids: vec![],
            exclude_tree_pids: vec![],
            duration_seconds: Some(30),
            spike_us: Some(1000),
            summary_ms: Some(1000),
            include_comm: vec![],
            exclude_comm: vec![],
            hwmon: false,
            cpu_freq: false,
            faults: false,
            stat_wait: false,
            block_io: false,
            irq_latency: true,
            irqs: vec![],
            record: true,
            run_name: None,
        };
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn version_response_reports_schema_version() {
        let resp = version_response();
        assert_eq!(resp.name, "stutter");
        assert!(resp.schema_version > 0);
    }

    #[test]
    fn capabilities_report_limits_and_artifacts() {
        let state = AgentState {
            active_run: Mutex::new(None),
            runs_dir: PathBuf::from("."),
            auth: AgentAuth { bearer_token: None },
            limits: AgentLimits {
                max_duration_seconds: 123,
                max_targets: 456,
                max_concurrent_recordings: 1,
            },
        };
        let resp = capabilities_response(&state);
        assert_eq!(resp.max_duration_seconds, 123);
        assert_eq!(resp.max_targets, 456);
        assert!(
            resp.supported_artifacts
                .contains(&"session.json".to_owned())
        );
    }
}
