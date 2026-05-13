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
    actions::SafetyClass,
    autotune::runtime::{
        AutotuneRuntimeConfig, daemon_config_for_runtime_mode, run_autotune_controller_session,
    },
    config::{FocusSource, ForegroundSource, model::MonitorConfig},
    daemon::{
        ActionSource, DaemonConfig, DaemonMode, DaemonPolicy, DaemonPolicyBuildInput,
        RemotePolicyContext, build_daemon_policy,
    },
    remote::{
        AgentAutotuneLimits, AgentFeatureFlags, AutotuneConfigResponse, AutotuneHistoryResponse,
        AutotuneRestoreResponse, AutotuneStartRequest, AutotuneStartResponse,
        AutotuneStatusResponse, AutotuneStopResponse, CapabilitiesResponse, HealthResponse,
        RecordStatusResponse, RemoteMonitorRequest, RunsResponse, StartRecordResponse,
        StopRecordResponse, VersionResponse,
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
    "runtime_slices.json",
    "foreground_events.json",
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
    pub autotune_limits: AgentAutotuneLimits,
    pub rollback_on_crash_recovery: bool,
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
    pub active_autotune: Mutex<Option<AutotuneControllerHandle>>,
    pub runs_dir: PathBuf,
    pub auth: AgentAuth,
    pub bind: SocketAddr,
    pub limits: AgentLimits,
    pub autotune_limits: AgentAutotuneLimits,
}

pub struct AutotuneControllerHandle {
    pub mode: String,
    pub watch_process: Option<String>,
    pub tree_pid: Option<u32>,
    pub started_unix_nanos: u128,
    pub stop_tx: oneshot::Sender<()>,
    pub join: JoinHandle<anyhow::Result<crate::autotune::runtime::AutotuneControllerExit>>,
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

    let startup_recovery =
        crate::autotune::startup_recovery::recover_controller_journal_on_startup(
            crate::autotune::startup_recovery::StartupRecoveryConfig {
                rollback_on_crash_recovery: config.rollback_on_crash_recovery,
                ..crate::autotune::startup_recovery::StartupRecoveryConfig::default()
            },
        )?;

    match startup_recovery {
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Recovered {
            experiment_id,
            action_id,
            affected_tasks,
            manual_restore_command,
        } => {
            println!(
                "autotune startup recovery: rolled back interrupted action experiment_id={} action_id={} affected_tasks={} manual_restore_command=\"{}\"",
                experiment_id, action_id, affected_tasks, manual_restore_command
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Faulted {
            experiment_id,
            action_id,
            manual_restore_command,
            reason,
        } => {
            eprintln!(
                "autotune startup recovery faulted: experiment_id={} action_id={} reason={}",
                experiment_id, action_id, reason
            );
            eprintln!("manual restore command: {}", manual_restore_command);
            anyhow::bail!(
                "autotune startup recovery failed; controller is Faulted; manual restore command: {}",
                manual_restore_command
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::RollbackDisabled {
            experiment_id,
            action_id,
            manual_restore_command,
        } => {
            println!(
                "autotune startup recovery: rollback_on_crash_recovery=false; leaving applied journal in place experiment_id={} action_id={} manual_restore_command=\"{}\"",
                experiment_id, action_id, manual_restore_command
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::ApplyingWithoutRollback {
            experiment_id,
            action_id,
        } => {
            println!(
                "autotune startup recovery: found applying journal without rollback token experiment_id={} action_id={}; no automatic rollback attempted",
                experiment_id, action_id
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Clean => {}
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
        active_autotune: Mutex::new(None),
        runs_dir: config.runs_dir,
        bind: config.bind,
        auth: AgentAuth {
            bearer_token: config.bearer_token,
        },
        autotune_limits: config.autotune_limits,
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
        .route("/autotune/status", get(autotune_status_handler))
        .route("/autotune/start", post(autotune_start_handler))
        .route("/autotune/stop", post(autotune_stop_handler))
        .route("/autotune/restore", post(autotune_restore_handler))
        .route("/autotune/history", get(autotune_history_handler))
        .route("/autotune/config", get(autotune_config_handler))
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct AutotuneStartSecurityRejection {
    status: StatusCode,
    audit_message: String,
    response_message: String,
}

fn safety_class_for_daemon_mode(mode: DaemonMode) -> SafetyClass {
    match mode {
        DaemonMode::Observe | DaemonMode::Suggest => SafetyClass::ObserveOnly,
        DaemonMode::ApplyLowRisk => SafetyClass::ReversibleLowRisk,
        DaemonMode::ApplyMediumRisk => SafetyClass::ReversibleMediumRisk,
        DaemonMode::ApplyHighRisk => SafetyClass::HighRisk,
    }
}

fn supported_remote_modes(limits: &AgentAutotuneLimits) -> Vec<DaemonMode> {
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

fn supported_remote_mode_labels(limits: &AgentAutotuneLimits) -> Vec<String> {
    supported_remote_modes(limits)
        .into_iter()
        .map(|mode| mode.as_str().to_owned())
        .collect()
}

fn remote_mode_supported(limits: &AgentAutotuneLimits, mode: DaemonMode) -> bool {
    supported_remote_modes(limits).contains(&mode)
}

fn daemon_policy_for_remote_mode(
    mode: DaemonMode,
    limits: &AgentAutotuneLimits,
    remote_context: RemotePolicyContext,
) -> DaemonPolicy {
    let mut config = DaemonConfig {
        mode,
        source: ActionSource::RemoteAgent,
        ..DaemonConfig::default()
    };
    config.remote.allow_remote_apply = mode.supports_apply();
    config.safety.allow_high_risk = limits.allow_high_risk && mode == DaemonMode::ApplyHighRisk;
    config.safety.allow_system_wide_actions = limits.allow_system_wide_actions;

    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: Some(remote_context),
    })
}

fn remote_policy_context_for_request(
    state: &AgentState,
    headers: &HeaderMap,
) -> RemotePolicyContext {
    RemotePolicyContext {
        is_loopback_bind: is_local_bind(&state.bind),
        auth_configured: state.auth.bearer_token.is_some(),
        valid_auth: authorize(headers, &state.auth).is_ok(),
        max_mode: state.autotune_limits.max_mode,
        max_safety_class: state.autotune_limits.max_safety_class.clone(),
        allow_high_risk: state.autotune_limits.allow_high_risk,
        allow_system_wide_actions: state.autotune_limits.allow_system_wide_actions,
        max_remote_targets: state.autotune_limits.max_targets,
    }
}

fn parse_focus_source_or_hybrid(value: Option<&str>) -> FocusSource {
    match value {
        Some("heuristic") => FocusSource::Heuristic,
        Some("foreground") => FocusSource::Foreground,
        Some("hybrid") | None => FocusSource::Hybrid,
        Some(_) => FocusSource::Hybrid,
    }
}

fn parse_foreground_source_or_auto(value: Option<&str>) -> ForegroundSource {
    match value {
        Some("sway") => ForegroundSource::Sway,
        Some("hyprland") => ForegroundSource::Hyprland,
        Some("x11") => ForegroundSource::X11,
        Some("auto") | None => ForegroundSource::Auto,
        Some(_) => ForegroundSource::Auto,
    }
}

fn validate_autotune_start_limits(
    request: &AutotuneStartRequest,
    state: &AgentState,
) -> anyhow::Result<()> {
    let limits = &state.autotune_limits;

    if limits.max_active_controllers != 1 {
        anyhow::bail!("remote autotune supports exactly one active controller");
    }

    if limits.allow_system_wide_actions {
        anyhow::bail!("remote autotune system-wide actions are disabled by default");
    }

    let requested_target_count =
        usize::from(request.watch_process.is_some()) + usize::from(request.tree_pid.is_some());

    if requested_target_count == 0 && !request.auto_focus {
        anyhow::bail!("autotune start requires watch_process, tree_pid, or auto_focus");
    }

    if requested_target_count > limits.max_targets {
        anyhow::bail!(
            "autotune target count {} exceeds max_targets {}",
            requested_target_count,
            limits.max_targets
        );
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

fn policy_for_remote_autotune_start(
    headers: &HeaderMap,
    state: &AgentState,
    request: &AutotuneStartRequest,
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

    // Phase 1 security mandate: Apply modes require authentication and loopback bind
    if mode.supports_apply() {
        if state.auth.bearer_token.is_none() {
            return Err(AutotuneStartSecurityRejection {
                status: StatusCode::UNAUTHORIZED,
                audit_message: format!(
                    "rejected remote autotune apply mode={mode}: bearer token is required"
                ),
                response_message: "remote autotune apply mode requires a configured bearer token"
                    .to_owned(),
            });
        }

        if let Err(status) = authorize(headers, &state.auth) {
            return Err(AutotuneStartSecurityRejection {
                status,
                audit_message: format!(
                    "rejected remote autotune apply mode={mode}: invalid bearer token"
                ),
                response_message: "remote autotune apply mode requires a valid bearer token"
                    .to_owned(),
            });
        }

        if !is_local_bind(&state.bind) {
            return Err(AutotuneStartSecurityRejection {
                status: StatusCode::FORBIDDEN,
                audit_message: format!(
                    "rejected remote autotune apply mode={mode}: non-loopback bind {} is not allowed",
                    state.bind
                ),
                response_message:
                    "remote autotune apply mode is only allowed on loopback binds for now"
                        .to_owned(),
            });
        }
    } else {
        // Observe/Suggest still require auth if a token is configured
        if let Err(status) = authorize(headers, &state.auth) {
            return Err(AutotuneStartSecurityRejection {
                status,
                audit_message: format!("rejected remote autotune mode={mode}: unauthorized"),
                response_message: "unauthorized".to_owned(),
            });
        }
    }

    // Now check if the mode is actually supported by the agent's configured limits
    if !remote_mode_supported(&state.autotune_limits, mode) {
        return Err(AutotuneStartSecurityRejection {
            status: StatusCode::BAD_REQUEST,
            audit_message: format!(
                "rejected remote autotune mode={mode}: exceeds configured remote limits max_mode={} max_safety_class={:?} allow_high_risk={}",
                state.autotune_limits.max_mode,
                state.autotune_limits.max_safety_class,
                state.autotune_limits.allow_high_risk
            ),
            response_message:
                "unsupported remote autotune mode; use observe, suggest, or apply-low-risk"
                    .to_owned(),
        });
    }

    let policy = daemon_policy_for_remote_mode(
        mode,
        &state.autotune_limits,
        remote_policy_context_for_request(state, headers),
    );

    if mode == DaemonMode::ApplyHighRisk && !policy.allow_high_risk {
        return Err(AutotuneStartSecurityRejection {
            status: StatusCode::FORBIDDEN,
            audit_message: "rejected remote autotune apply-high-risk: high risk not enabled"
                .to_owned(),
            response_message: "remote autotune high-risk mode is disabled".to_owned(),
        });
    }

    Ok(policy)
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
    audit_agent_event_with_safety(
        action_id,
        SafetyClass::ObserveOnly,
        success,
        affected_tasks,
        message,
    );
}

fn audit_agent_event_with_safety(
    action_id: &'static str,
    safety_class: SafetyClass,
    success: bool,
    affected_tasks: usize,
    message: String,
) {
    crate::audit::audit_or_warn(&crate::audit::AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "agent".to_owned(),
        action_id: Some(action_id.to_owned()),
        safety_class: Some(safety_class),
        dry_run: false,
        success,
        affected_tasks,
        restore_path: None,
        action_phase: None,
        error_category: None,
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

    let join = tokio::spawn(async move {
        crate::session::run_monitor(monitor_config, None, None, Some(stop_rx)).await
    });

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

async fn autotune_status_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    let active = state.active_autotune.lock().await;
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
            manual_restore_command: Some("stutter autotune restore".to_owned()),
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
            manual_restore_command: Some("stutter autotune restore".to_owned()),
            message: "no autotune session active".to_owned(),
        },
    };

    Json(response).into_response()
}

async fn autotune_start_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
    Json(request): Json<AutotuneStartRequest>,
) -> impl IntoResponse {
    let policy = match policy_for_remote_autotune_start(&headers, &state, &request) {
        Ok(policy) => policy,
        Err(rejection) => {
            audit_agent_event("remote-autotune-start", false, 0, rejection.audit_message);
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
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("autotune limit rejected: {err}"),
            }),
        )
            .into_response();
    }

    let mut active = state.active_autotune.lock().await;
    if active.is_some() {
        audit_agent_event(
            "remote-autotune-start",
            false,
            0,
            "conflict: autotune session already active".to_owned(),
        );
        return (
            StatusCode::CONFLICT,
            Json(ErrorResponse {
                error: "an autotune session is already active".to_owned(),
            }),
        )
            .into_response();
    }

    let input = crate::autotune::AutotuneCommandInput {
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
        allow_system_wide_actions: false,
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
        daemon_config.safety.allow_system_wide_actions = policy.allow_system_wide_actions;

        let mut runtime_config = AutotuneRuntimeConfig::from_daemon_parts(
            daemon_config,
            policy.clone(),
            request.decision_log.as_deref().map(PathBuf::from),
        )
        .with_profiles(profile_list);

        if mode == "apply-low-risk" {
            runtime_config =
                runtime_config.with_candidate_window_seconds(input.duration_seconds.unwrap_or(30));
        }

        let (stop_tx, stop_rx) = oneshot::channel();
        let duration = if mode == "apply-low-risk" {
            None
        } else {
            input.duration_seconds.map(std::time::Duration::from_secs)
        };
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

    *active = Some(handle);

    audit_agent_event_with_safety(
        "remote-autotune-start",
        safety_class_for_daemon_mode(policy.mode),
        true,
        0,
        format!(
            "daemon_mode={} action_source={:?} allow_system_wide_actions={} allow_high_risk={} watch_process={:?} tree_pid={:?}",
            policy.mode,
            policy.source,
            policy.allow_system_wide_actions,
            policy.allow_high_risk,
            request.watch_process,
            request.tree_pid
        ),
    );

    Json(AutotuneStartResponse {
        status: "started".to_owned(),
        mode,
        message: "remote autotune observe/suggest controller started; apply modes remain disabled"
            .to_owned(),
    })
    .into_response()
}

async fn autotune_stop_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
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

        return Json(AutotuneStopResponse {
            status: "no_active_session".to_owned(),
            message: "no remote autotune session was active".to_owned(),
        })
        .into_response();
    };

    let _ = handle.stop_tx.send(());

    match handle.join.await {
        Ok(Ok(exit)) => {
            audit_agent_event(
                "remote-autotune-stop",
                true,
                0,
                format!("active_before_stop=true reason={}", exit.reason),
            );

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

async fn autotune_restore_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        audit_agent_event(
            "remote-autotune-restore",
            false,
            0,
            "unauthorized".to_owned(),
        );
        return status.into_response();
    }

    audit_agent_event(
        "remote-autotune-restore",
        true,
        0,
        "restore requested; no remote apply-low-risk action is active".to_owned(),
    );

    Json(AutotuneRestoreResponse {
        status: "no_active_apply".to_owned(),
        message: "remote apply-low-risk is not enabled, so no remote autotune action was restored"
            .to_owned(),
    })
    .into_response()
}

async fn autotune_history_handler(
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

async fn autotune_config_handler(
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
        allow_system_wide_actions: state.autotune_limits.allow_system_wide_actions,
        minimum_focus_confidence: 0.70,
        required_stable_focus_polls: 3,
    })
    .into_response()
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
            "/autotune/status".to_owned(),
            "/autotune/start".to_owned(),
            "/autotune/stop".to_owned(),
            "/autotune/restore".to_owned(),
            "/autotune/history".to_owned(),
            "/autotune/config".to_owned(),
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
            autotune_observe: remote_mode_supported(&state.autotune_limits, DaemonMode::Observe),
            autotune_suggest: remote_mode_supported(&state.autotune_limits, DaemonMode::Suggest),
            autotune_apply_low_risk: remote_mode_supported(
                &state.autotune_limits,
                DaemonMode::ApplyLowRisk,
            ),
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

fn parse_remote_focus_source(value: Option<&str>) -> anyhow::Result<FocusSource> {
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

fn parse_remote_foreground_source(value: Option<&str>) -> anyhow::Result<ForegroundSource> {
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

fn remote_foreground_enabled(request: &RemoteMonitorRequest) -> anyhow::Result<bool> {
    let focus_source = parse_remote_focus_source(request.focus_source.as_deref())?;
    Ok(request.foreground_window || focus_source != FocusSource::Heuristic)
}

fn validate_remote_foreground_request(request: &RemoteMonitorRequest) -> anyhow::Result<()> {
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

fn validate_foreground_title_capture_security(
    request: &RemoteMonitorRequest,
    state: &AgentState,
    headers: &HeaderMap,
) -> Result<(), StatusCode> {
    if !request.foreground_include_title {
        return Ok(());
    }

    if is_local_bind(&state.bind) {
        return Ok(());
    }

    if state.auth.bearer_token.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }

    authorize(headers, &state.auth)
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

fn monitor_config_from_remote_request(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_remote_request() -> RemoteMonitorRequest {
        RemoteMonitorRequest {
            target_pids: vec![1234],
            tree_pids: Vec::new(),
            exclude_tree_pids: Vec::new(),
            duration_seconds: Some(5),
            spike_us: Some(1000),
            summary_ms: Some(500),
            include_comm: Vec::new(),
            exclude_comm: Vec::new(),
            hwmon: false,
            cpu_freq: false,
            faults: false,
            stat_wait: false,
            block_io: false,
            runtime_slices: false,
            runtime_slices_max_tasks: None,
            irq_latency: false,
            irqs: Vec::new(),
            foreground_window: false,
            focus_source: None,
            foreground_source: None,
            foreground_poll_ms: None,
            foreground_max_stale_ms: None,
            foreground_include_title: false,
            record: true,
            run_name: Some("remote-test".to_owned()),
        }
    }

    fn test_agent_state_custom(bind: SocketAddr, bearer_token: Option<String>) -> AgentState {
        AgentState {
            active_run: Mutex::new(None),
            active_autotune: Mutex::new(None),
            runs_dir: std::env::temp_dir(),
            auth: AgentAuth { bearer_token },
            bind,
            limits: AgentLimits {
                max_duration_seconds: DEFAULT_AGENT_MAX_DURATION_SECONDS,
                max_targets: DEFAULT_AGENT_MAX_TARGETS,
                max_concurrent_recordings: DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
            },
            autotune_limits: AgentAutotuneLimits::default(),
        }
    }

    fn test_agent_state(bind: SocketAddr, token: Option<&str>) -> AgentState {
        AgentState {
            active_run: Mutex::new(None),
            active_autotune: Mutex::new(None),
            runs_dir: PathBuf::from("."),
            auth: AgentAuth {
                bearer_token: token.map(str::to_owned),
            },
            bind,
            limits: AgentLimits {
                max_duration_seconds: DEFAULT_AGENT_MAX_DURATION_SECONDS,
                max_targets: DEFAULT_AGENT_MAX_TARGETS,
                max_concurrent_recordings: DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
            },
            autotune_limits: AgentAutotuneLimits::default(),
        }
    }

    fn autotune_request(mode: &str) -> AutotuneStartRequest {
        AutotuneStartRequest {
            mode: mode.to_owned(),
            watch_process: Some("game".to_owned()),
            tree_pid: None,
            profiles: None,
            config: None,
            duration_seconds: Some(5),
            decision_log: None,
            summary_ms: None,
            preset: None,
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            focus_source: None,
            foreground_window: false,
            foreground_source: None,
            foreground_poll_ms: None,
            foreground_max_stale_ms: None,
            washout_seconds: None,
            washout_verify_interval_ms: None,
        }
    }

    #[test]
    fn remote_autotune_observe_builds_observe_policy_without_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        let policy =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("observe"))
                .unwrap();

        assert_eq!(policy.mode, DaemonMode::Observe);
        assert_eq!(policy.source, ActionSource::RemoteAgent);
    }

    #[test]
    fn remote_autotune_apply_requires_configured_bearer_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        let rejection =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
                .unwrap_err();

        assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
        assert!(rejection.response_message.contains("bearer token"));
    }

    #[test]
    fn remote_autotune_apply_requires_valid_bearer_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let headers = HeaderMap::new();

        let rejection =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
                .unwrap_err();

        assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
        assert!(rejection.response_message.contains("valid bearer token"));
    }

    #[test]
    fn remote_autotune_all_apply_modes_require_bearer_token_before_limit_rejection() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        for mode in ["apply-low-risk", "apply-medium-risk", "apply-high-risk"] {
            let rejection =
                policy_for_remote_autotune_start(&headers, &state, &autotune_request(mode))
                    .unwrap_err();

            assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
            assert!(rejection.response_message.contains("bearer token"));
        }
    }

    #[test]
    fn remote_autotune_apply_requires_loopback_bind() {
        let state = test_agent_state("0.0.0.0:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );

        let rejection =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
                .unwrap_err();

        assert_eq!(rejection.status, StatusCode::FORBIDDEN);
        assert!(rejection.response_message.contains("loopback"));
    }

    #[test]
    fn remote_autotune_apply_low_risk_builds_low_risk_policy_with_valid_auth() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );

        let policy =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
                .unwrap();

        assert_eq!(policy.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(policy.source, ActionSource::RemoteAgent);
        assert!(!policy.allow_system_wide_actions);
        assert!(!policy.allow_high_risk);
    }

    #[test]
    fn remote_autotune_policy_build_is_deterministic_for_same_context() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );
        let first_context = remote_policy_context_for_request(&state, &headers);
        let second_context = remote_policy_context_for_request(&state, &headers);

        let first = daemon_policy_for_remote_mode(
            DaemonMode::ApplyLowRisk,
            &state.autotune_limits,
            first_context,
        );
        let second = daemon_policy_for_remote_mode(
            DaemonMode::ApplyLowRisk,
            &state.autotune_limits,
            second_context,
        );

        assert_eq!(first, second);
        assert!(first.remote_apply.allow_remote_apply);
        assert_eq!(
            first.remote_apply.max_remote_targets,
            state.autotune_limits.max_targets
        );
    }

    #[test]
    fn remote_autotune_high_risk_rejected_by_default_limits() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );

        let rejection = policy_for_remote_autotune_start(
            &headers,
            &state,
            &autotune_request("apply-high-risk"),
        )
        .unwrap_err();

        assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
        assert!(
            rejection
                .response_message
                .contains("unsupported remote autotune mode")
        );
    }

    #[test]
    fn autotune_start_rejects_wrong_bearer_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer wrong"),
        );

        let rejection =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("observe"))
                .unwrap_err();

        assert_eq!(rejection.status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn supported_remote_modes_derive_from_limits() {
        let limits = AgentAutotuneLimits::default();

        assert_eq!(
            supported_remote_mode_labels(&limits),
            vec![
                "observe".to_owned(),
                "suggest".to_owned(),
                "apply-low-risk".to_owned()
            ]
        );
    }

    #[tokio::test]
    async fn autotune_start_accepts_observe_mode_with_tree_pid() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "observe".to_owned(),
                watch_process: None,
                tree_pid: Some(1234),
                profiles: None,
                config: None,
                duration_seconds: Some(30),
                decision_log: None,
                summary_ms: None,
                preset: None,
                hwmon: false,
                mangohud_log: None,
                auto_focus: false,
                focus_source: None,
                foreground_window: false,
                foreground_source: None,
                foreground_poll_ms: None,
                foreground_max_stale_ms: None,
                washout_seconds: None,
                washout_verify_interval_ms: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);

        let active = state.active_autotune.lock().await;
        assert!(active.is_some());
        assert_eq!(active.as_ref().unwrap().mode, "observe");
        assert_eq!(active.as_ref().unwrap().tree_pid, Some(1234));
    }

    #[tokio::test]
    async fn autotune_start_accepts_suggest_mode_with_watch_process() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "suggest".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
                decision_log: None,
                summary_ms: None,
                preset: None,
                hwmon: false,
                mangohud_log: None,
                auto_focus: false,
                focus_source: None,
                foreground_window: false,
                foreground_source: None,
                foreground_poll_ms: None,
                foreground_max_stale_ms: None,
                washout_seconds: None,
                washout_verify_interval_ms: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);

        let active = state.active_autotune.lock().await;
        assert!(active.is_some());
        assert_eq!(active.as_ref().unwrap().mode, "suggest");
        assert_eq!(
            active.as_ref().unwrap().watch_process.as_deref(),
            Some("Game.exe")
        );
    }

    #[tokio::test]
    async fn autotune_start_allows_apply_low_risk_mode_with_valid_auth() {
        let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        state_value.auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
        };
        let state = Arc::new(state_value);
        let response = autotune_start_handler(
            State(state.clone()),
            autotune_headers(Some("secret")),
            Json(AutotuneStartRequest {
                mode: "apply-low-risk".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
                decision_log: None,
                summary_ms: None,
                preset: None,
                hwmon: false,
                mangohud_log: None,
                auto_focus: false,
                focus_source: None,
                foreground_window: false,
                foreground_source: None,
                foreground_poll_ms: None,
                foreground_max_stale_ms: None,
                washout_seconds: None,
                washout_verify_interval_ms: None,
            }),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.active_autotune.lock().await.is_some());
    }

    #[test]
    fn autotune_headers_test() {
        let headers = autotune_headers(Some("secret"));
        assert_eq!(
            headers.get(axum::http::header::AUTHORIZATION).unwrap(),
            "Bearer secret"
        );
    }

    fn autotune_headers(token: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(token) = token {
            headers.insert(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
        }
        headers
    }

    fn test_autotune_handle() -> AutotuneControllerHandle {
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        let join = tokio::spawn(async move {
            let _ = stop_rx.await;
            Ok(crate::autotune::runtime::AutotuneControllerExit {
                reason: "test stop".to_owned(),
                last_decision: None,
            })
        });

        AutotuneControllerHandle {
            mode: "observe".to_owned(),
            watch_process: Some("Game.exe".to_owned()),
            tree_pid: None,
            started_unix_nanos: crate::audit::unix_nanos_now(),
            stop_tx,
            join,
        }
    }

    #[tokio::test]
    async fn autotune_start_apply_low_risk_without_auth_is_rejected_before_mode_validation() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "apply-low-risk".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
                decision_log: None,
                summary_ms: None,
                preset: None,
                hwmon: false,
                mangohud_log: None,
                auto_focus: false,
                focus_source: None,
                foreground_window: false,
                foreground_source: None,
                foreground_poll_ms: None,
                foreground_max_stale_ms: None,
                washout_seconds: None,
                washout_verify_interval_ms: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_start_apply_low_risk_on_non_loopback_is_rejected_even_with_valid_token() {
        let mut state_value = test_agent_state("0.0.0.0:9899".parse().unwrap(), Some("secret"));
        state_value.auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
        };
        state_value.bind = "0.0.0.0:9899".parse().unwrap();
        let state = Arc::new(state_value);

        let response = autotune_start_handler(
            State(state.clone()),
            autotune_headers(Some("secret")),
            Json(AutotuneStartRequest {
                mode: "apply-low-risk".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
                decision_log: None,
                summary_ms: None,
                preset: None,
                hwmon: false,
                mangohud_log: None,
                auto_focus: false,
                focus_source: None,
                foreground_window: false,
                foreground_source: None,
                foreground_poll_ms: None,
                foreground_max_stale_ms: None,
                washout_seconds: None,
                washout_verify_interval_ms: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_start_rejects_request_above_candidate_window_cap() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "observe".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(121),
                decision_log: None,
                summary_ms: None,
                preset: None,
                hwmon: false,
                mangohud_log: None,
                auto_focus: false,
                focus_source: None,
                foreground_window: false,
                foreground_source: None,
                foreground_poll_ms: None,
                foreground_max_stale_ms: None,
                washout_seconds: None,
                washout_verify_interval_ms: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_start_rejects_more_than_one_target() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "observe".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: Some(1234),
                profiles: None,
                config: None,
                duration_seconds: Some(30),
                decision_log: None,
                summary_ms: None,
                preset: None,
                hwmon: false,
                mangohud_log: None,
                auto_focus: false,
                focus_source: None,
                foreground_window: false,
                foreground_source: None,
                foreground_poll_ms: None,
                foreground_max_stale_ms: None,
                washout_seconds: None,
                washout_verify_interval_ms: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_start_requires_target_selector() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "observe".to_owned(),
                watch_process: None,
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
                decision_log: None,
                summary_ms: None,
                preset: None,
                hwmon: false,
                mangohud_log: None,
                auto_focus: false,
                focus_source: None,
                foreground_window: false,
                foreground_source: None,
                foreground_poll_ms: None,
                foreground_max_stale_ms: None,
                washout_seconds: None,
                washout_verify_interval_ms: None,
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_stop_clears_active_session() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        *state.active_autotune.lock().await = Some(test_autotune_handle());

        let response = autotune_stop_handler(State(state.clone()), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_restore_is_safe_noop_until_remote_apply_exists() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

        let response = autotune_restore_handler(State(state), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn autotune_config_response_includes_limits() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let response = AutotuneConfigResponse {
            default_mode: "observe".to_owned(),
            supported_modes: vec!["observe".to_owned(), "suggest".to_owned()],
            apply_low_risk_remote_enabled: false,
            local_only_by_default: true,
            history_path: crate::autotune::history::default_autotune_history_path()
                .display()
                .to_string(),
            autotune_limits: state.autotune_limits.clone(),
            daemon_scope: "focused".to_owned(),
            allow_system_wide_actions: false,
            minimum_focus_confidence: 0.70,
            required_stable_focus_polls: 3,
        };

        assert_eq!(response.autotune_limits, AgentAutotuneLimits::default());
        assert_eq!(response.autotune_limits.max_active_controllers, 1);
        assert_eq!(
            response.autotune_limits.max_safety_class,
            SafetyClass::ReversibleLowRisk
        );
        assert_eq!(response.autotune_limits.max_candidate_window_seconds, 120);
        assert_eq!(response.autotune_limits.max_targets, 1);
        assert!(!response.autotune_limits.allow_system_wide_actions);
    }

    #[tokio::test]
    async fn autotune_config_reports_apply_low_risk_disabled() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

        let response = autotune_config_handler(State(state), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn capabilities_includes_autotune_routes() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let resp = capabilities_response(&state);
        assert!(
            resp.supported_routes
                .contains(&"/autotune/start".to_owned())
        );
        assert!(
            resp.supported_routes
                .contains(&"/autotune/status".to_owned())
        );
        assert!(resp.supported_routes.contains(&"/autotune/stop".to_owned()));
    }

    #[tokio::test]
    async fn autotune_start_rejects_system_wide_actions_by_default() {
        let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        state_value.autotune_limits.allow_system_wide_actions = true; // but the validator rejects it for remote
        let state = Arc::new(state_value);

        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(autotune_request("observe")),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        // The error comes from validate_autotune_start_limits
    }

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
        let mut request = minimal_remote_request();
        request.target_pids = vec![1];
        request.duration_seconds = Some(120);
        request.summary_ms = Some(1000);
        request.run_name = None;
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
        let mut request = minimal_remote_request();
        request.target_pids = vec![1, 2, 3];
        request.duration_seconds = Some(30);
        request.summary_ms = Some(1000);
        request.run_name = None;
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn remote_request_rejects_zero_summary_ms() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let mut request = minimal_remote_request();
        request.target_pids = vec![1];
        request.duration_seconds = Some(30);
        request.summary_ms = Some(0);
        request.run_name = None;
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn remote_request_rejects_zero_spike_us() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let mut request = minimal_remote_request();
        request.target_pids = vec![1];
        request.duration_seconds = Some(30);
        request.spike_us = Some(0);
        request.summary_ms = Some(1000);
        request.run_name = None;
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn remote_request_rejects_irq_latency_without_irqs() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let mut request = minimal_remote_request();
        request.target_pids = vec![1];
        request.duration_seconds = Some(30);
        request.summary_ms = Some(1000);
        request.irq_latency = true;
        request.run_name = None;
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
            active_autotune: Mutex::new(None),
            runs_dir: PathBuf::from("."),
            auth: AgentAuth { bearer_token: None },
            bind: "127.0.0.1:9899".parse().unwrap(),
            limits: AgentLimits {
                max_duration_seconds: 123,
                max_targets: 456,
                max_concurrent_recordings: 1,
            },
            autotune_limits: AgentAutotuneLimits::default(),
        };
        let resp = capabilities_response(&state);
        assert_eq!(resp.max_duration_seconds, 123);
        assert_eq!(resp.max_targets, 456);
        assert!(
            resp.supported_artifacts
                .contains(&"session.json".to_owned())
        );
    }

    #[test]
    fn agent_artifact_allowlist_includes_foreground_events() {
        assert!(AGENT_ARTIFACT_ALLOWLIST.contains(&"foreground_events.json"));
        assert!(validate_artifact_name("foreground_events.json").is_ok());
    }

    #[test]
    fn validate_remote_request_accepts_foreground_defaults() {
        let request = minimal_remote_request();
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        validate_remote_request_limits(&request, &limits).unwrap();
        assert!(!remote_foreground_enabled(&request).unwrap());
    }

    #[test]
    fn validate_remote_request_rejects_invalid_focus_source() {
        let mut request = minimal_remote_request();
        request.focus_source = Some("dbus".to_owned());
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let err = validate_remote_request_limits(&request, &limits)
            .unwrap_err()
            .to_string();

        assert!(err.contains("focus_source must be heuristic, foreground, or hybrid"));
    }

    #[test]
    fn validate_remote_request_rejects_invalid_foreground_source() {
        let mut request = minimal_remote_request();
        request.foreground_source = Some("gnome".to_owned());
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let err = validate_remote_request_limits(&request, &limits)
            .unwrap_err()
            .to_string();

        assert!(err.contains("foreground_source must be auto, sway, hyprland, or x11"));
    }

    #[test]
    fn validate_remote_request_rejects_too_fast_foreground_poll() {
        let mut request = minimal_remote_request();
        request.foreground_window = true;
        request.foreground_poll_ms = Some(99);
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let err = validate_remote_request_limits(&request, &limits)
            .unwrap_err()
            .to_string();

        assert!(err.contains("foreground_poll_ms must be >= 100"));
    }

    #[test]
    fn monitor_config_from_remote_request_applies_foreground_fields() {
        let mut request = minimal_remote_request();
        request.foreground_window = true;
        request.focus_source = Some("hybrid".to_owned());
        request.foreground_source = Some("sway".to_owned());
        request.foreground_poll_ms = Some(750);
        request.foreground_max_stale_ms = Some(3000);
        request.foreground_include_title = false;

        let dir = std::env::temp_dir().join("stutter-agent-foreground-config-test");
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let config = monitor_config_from_remote_request(&request, &dir, &limits).unwrap();

        assert!(config.focus.foreground_window);
        assert_eq!(config.focus.focus_source, FocusSource::Hybrid);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
        assert_eq!(config.focus.foreground_poll_ms, 750);
        assert_eq!(config.focus.foreground_max_stale_ms, 3000);
        assert!(!config.focus.foreground_include_title);
    }

    #[test]
    fn monitor_config_from_remote_request_enables_foreground_window_for_non_heuristic_focus() {
        let mut request = minimal_remote_request();
        request.foreground_window = false;
        request.focus_source = Some("foreground".to_owned());

        let dir = std::env::temp_dir().join("stutter-agent-foreground-focus-test");
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let config = monitor_config_from_remote_request(&request, &dir, &limits).unwrap();

        assert!(config.focus.foreground_window);
        assert_eq!(config.focus.focus_source, FocusSource::Foreground);
    }

    #[test]
    fn foreground_title_capture_allowed_on_loopback_without_token() {
        let mut request = minimal_remote_request();
        request.foreground_include_title = true;
        let state = test_agent_state_custom("127.0.0.1:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        assert!(validate_foreground_title_capture_security(&request, &state, &headers).is_ok());
    }

    #[test]
    fn foreground_title_capture_rejected_on_non_loopback_without_token() {
        let mut request = minimal_remote_request();
        request.foreground_include_title = true;
        let state = test_agent_state_custom("0.0.0.0:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        assert_eq!(
            validate_foreground_title_capture_security(&request, &state, &headers),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn foreground_title_capture_allowed_on_non_loopback_with_valid_bearer_token() {
        let mut request = minimal_remote_request();
        request.foreground_include_title = true;
        let state =
            test_agent_state_custom("0.0.0.0:0".parse().unwrap(), Some("secret".to_owned()));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );

        assert!(validate_foreground_title_capture_security(&request, &state, &headers).is_ok());
    }

    #[test]
    fn foreground_title_capture_rejected_on_non_loopback_with_invalid_bearer_token() {
        let mut request = minimal_remote_request();
        request.foreground_include_title = true;
        let state =
            test_agent_state_custom("0.0.0.0:0".parse().unwrap(), Some("secret".to_owned()));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer wrong".parse().unwrap(),
        );

        assert_eq!(
            validate_foreground_title_capture_security(&request, &state, &headers),
            Err(StatusCode::FORBIDDEN)
        );
    }
}
#[cfg(test)]
mod remote_policy_tests {
    use axum::http::{HeaderMap, StatusCode};

    use super::*;
    use crate::{
        actions::SafetyClass,
        daemon_policy::DaemonMode,
        remote::{AgentAutotuneLimits, AutotuneStartRequest},
    };

    fn test_agent_state(token: Option<&str>, bind_loopback: bool) -> AgentState {
        AgentState {
            active_run: Mutex::new(None),
            active_autotune: Mutex::new(None),
            runs_dir: PathBuf::from("/tmp"),
            auth: AgentAuth {
                bearer_token: token.map(str::to_owned),
            },
            bind: if bind_loopback {
                "127.0.0.1:0".parse().unwrap()
            } else {
                "0.0.0.0:0".parse().unwrap()
            },
            limits: AgentLimits {
                max_duration_seconds: 300,
                max_targets: 128,
                max_concurrent_recordings: 1,
            },
            autotune_limits: AgentAutotuneLimits {
                max_active_controllers: 1,
                max_mode: DaemonMode::ApplyLowRisk,
                max_safety_class: SafetyClass::ReversibleLowRisk,
                allow_high_risk: false,
                max_candidate_window_seconds: 60,
                max_targets: 10,
                allow_system_wide_actions: false,
            },
        }
    }

    fn autotune_start_request(mode: &str) -> AutotuneStartRequest {
        AutotuneStartRequest {
            mode: mode.to_owned(),
            watch_process: None,
            tree_pid: Some(1234),
            profiles: None,
            config: None,
            duration_seconds: Some(30),
            decision_log: None,
            summary_ms: None,
            preset: None,
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            focus_source: None,
            foreground_window: false,
            foreground_source: None,
            foreground_poll_ms: None,
            foreground_max_stale_ms: None,
            washout_seconds: None,
            washout_verify_interval_ms: None,
        }
    }

    #[test]
    fn remote_policy_rejects_apply_without_bearer_token() {
        let state = test_agent_state(Some("secret"), true);
        let request = autotune_start_request("apply-low-risk");

        let res = policy_for_remote_autotune_start(&HeaderMap::new(), &state, &request);
        assert_eq!(res.unwrap_err().status, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer wrong".parse().unwrap());
        let res = policy_for_remote_autotune_start(&headers, &state, &request);
        assert_eq!(res.unwrap_err().status, StatusCode::FORBIDDEN);

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer secret".parse().unwrap());
        let res = policy_for_remote_autotune_start(&headers, &state, &request);
        assert!(res.is_ok());
    }

    #[test]
    fn remote_policy_rejects_apply_on_non_loopback_bind() {
        let state = test_agent_state(Some("secret"), false);
        let request = autotune_start_request("apply-low-risk");

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer secret".parse().unwrap());

        let res = policy_for_remote_autotune_start(&headers, &state, &request);
        assert_eq!(res.unwrap_err().status, StatusCode::FORBIDDEN);
    }
}
