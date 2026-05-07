use std::{
    net::SocketAddr,
    path::{Path as StdPath, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
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
        HealthResponse, RecordStatusResponse, RemoteMonitorRequest, RunsResponse,
        StartRecordResponse, StopRecordResponse,
    },
};

pub struct AgentState {
    pub active_run: Mutex<Option<RunHandle>>,
    pub runs_dir: PathBuf,
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

pub async fn run_agent(bind: SocketAddr, runs_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let runs_dir = runs_dir.unwrap_or_else(|| {
        let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        p.push("stutter-runs");
        p
    });

    if !runs_dir.exists() {
        std::fs::create_dir_all(&runs_dir)?;
    }

    if !is_local_bind(&bind) {
        println!(
            "WARNING: stutter agent is listening on a non-local address without authentication."
        );
    }

    let state = Arc::new(AgentState {
        active_run: Mutex::new(None),
        runs_dir,
    });

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/record/start", post(start_record_handler))
        .route("/record/stop", post(stop_record_handler))
        .route("/record/status", get(status_handler))
        .route("/runs", get(list_runs_handler))
        .route("/runs/:id/session.json", get(get_session_handler))
        .route("/runs/:id/artifact/:name", get(get_artifact_handler))
        .with_state(state);

    println!("stutter agent listening on http://{}", bind);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;

    Ok(())
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
    Json(request): Json<RemoteMonitorRequest>,
) -> impl IntoResponse {
    let mut active = state.active_run.lock().await;
    if active.is_some() {
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

    let config = match config_from_remote_request(&request, &run_dir) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("invalid configuration: {e}"),
                }),
            )
                .into_response();
        }
    };

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

async fn stop_record_handler(State(state): State<Arc<AgentState>>) -> impl IntoResponse {
    let mut active = state.active_run.lock().await;
    let handle = active.take();

    if let Some(handle) = handle {
        let _ = handle.stop_tx.send(());
        let run_id = handle.id.clone();
        match handle.join.await {
            Ok(Ok(_)) => (
                StatusCode::OK,
                Json(StopRecordResponse {
                    run_id: Some(run_id),
                    status: "stopped".to_owned(),
                }),
            )
                .into_response(),
            Ok(Err(e)) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("recording failed: {e}"),
                }),
            )
                .into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("task join failed: {e}"),
                }),
            )
                .into_response(),
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

async fn status_handler(State(state): State<Arc<AgentState>>) -> impl IntoResponse {
    let active = state.active_run.lock().await;
    Json(RecordStatusResponse {
        active: active.is_some(),
        run_id: active.as_ref().map(|h| h.id.clone()),
    })
}

async fn list_runs_handler(State(state): State<Arc<AgentState>>) -> impl IntoResponse {
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
    Json(RunsResponse { runs })
}

async fn get_session_handler(
    State(state): State<Arc<AgentState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
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
    Path((id, name)): Path<(String, String)>,
) -> impl IntoResponse {
    if let Err(e) = validate_id(&id) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    if let Err(e) = validate_id(&name) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }

    let path = state.runs_dir.join(&id).join(&name);
    if !path.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }

    match std::fs::read(path) {
        Ok(data) => data.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
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
        max_tasks: 1024,
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
        max_duration: request.duration_seconds.map(std::time::Duration::from_secs),
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
    fn test_is_local_bind() {
        assert!(is_local_bind(&"127.0.0.1:9899".parse().unwrap()));
        assert!(is_local_bind(&"[::1]:9899".parse().unwrap()));
        assert!(!is_local_bind(&"0.0.0.0:9899".parse().unwrap()));
        assert!(!is_local_bind(&"192.168.1.1:9899".parse().unwrap()));
    }
}
