//! Agent HTTP service, remote command endpoints, and session controllers.
//!
//! Owns:
//! - Axum router registration, rate limiting, request token auth, startup journal recovery, runs
//!   directory queries, active recorder handles, and remote autotune controllers.
//!
//! Does not own:
//! - low-level system/PSI probes, eBPF attachment, CLI parsing, rendering local HTML/text reports, or
//!   applying autotune actions directly (actions are delegated to the daemon state).
//!
//! Allowed dependencies:
//! - actions safety classes, autotune controllers, config models, daemon policy logic, remote API
//!   data structures, and standard network/tokio utilities.
//!
//! Main entry points:
//! - `AgentConfig`, `AgentState`, `run_agent`, version/capabilities endpoints, recording start/stop
//!   handlers, autotune start/stop/restore endpoints, and daemon status endpoints.
//!
//! Safety, mutation, and persistence invariants:
//! - binding to non-loopback addresses requires explicit config authorization or bearer auth;
//! - all remote-triggered tuning must route through startup recovery validation and daemon policy;
//! - rate limiters and maximum request sizes must protect endpoints from memory exhaustion;
//! - inactive or aborted session resources must be cleaned up and their background joins polled.

use std::{
    collections::VecDeque,
    net::SocketAddr,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path as StdPath, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use hyper::body::Incoming;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as HyperConnectionBuilder,
    service::TowerToHyperService,
};
use serde::Serialize;
use tokio::{
    sync::{Mutex, Semaphore, oneshot},
    task::JoinHandle,
};
use tower::{Service, ServiceExt as _};

pub use crate::error::AgentError;
use crate::{
    actions::{ActionId, SafetyClass},
    autotune::{
        emergency_restore::{
            AutotuneRestoreCommandInput, AutotuneRestoreStatus, restore_known_autotune_actions,
        },
        runtime::{
            AutotuneRuntimeConfig, daemon_config_for_runtime_mode, run_autotune_controller_session,
        },
    },
    commands::restore,
    config::{FocusSource, ForegroundSource, model::MonitorConfig},
    daemon::{
        DaemonPhase, DaemonPolicy, DaemonState,
        capabilities::{CapabilityProbe, DaemonCapabilities},
        explain::{
            DaemonPolicyExplanation, DaemonStatusExplanation, PolicyDecisionKind,
            PolicyExplanation, policy_context_from_daemon_status,
        },
        health::{
            SystemHealthMonitor, SystemHealthProbeRoot, SystemHealthSnapshot,
            SystemHealthThresholds,
        },
        policy::{
            ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicyBuildInput,
            PolicyIntent, PolicyRejection, RemotePolicyContext, RollbackRequirement,
            build_daemon_policy,
        },
        privilege::{
            PrivilegeCommandAllowlist, PrivilegeCommandRequest, PrivilegeProcessRole,
            PrivilegeTransport, PrivilegedOperation,
        },
        state::{
            DaemonDecisionState, DaemonExperimentState, DaemonRollbackState, DaemonTargetState,
        },
        state_builders::{
            daemon_decision_state, daemon_state_for_agent_fault, daemon_state_for_record_start,
            daemon_state_from_startup_recovery,
        },
        watchdog::{
            DaemonWatchdogConfig, DaemonWatchdogInputs, DaemonWatchdogReport,
            evaluate_daemon_watchdog,
        },
    },
    remote::{
        AgentAutotuneLimits, AgentFeatureFlags, AutotuneConfigResponse, AutotuneHistoryResponse,
        AutotuneRestoreResponse, AutotuneStartRequest, AutotuneStartResponse,
        AutotuneStatusResponse, AutotuneStopResponse, CapabilitiesResponse, HealthResponse,
        RecordStatusResponse, RemoteMonitorRequest, RunsResponse, StartRecordResponse,
        StopRecordResponse, VersionResponse,
    },
};

pub(crate) mod artifacts;
pub(crate) mod auth;
pub(crate) mod autotune;
pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod rate_limit;
pub(crate) mod recording;
pub(crate) mod routes;
pub(crate) mod server;
pub(crate) mod startup;
pub(crate) mod state;

pub const DEFAULT_AGENT_MAX_DURATION_SECONDS: u64 = 300;
pub const DEFAULT_AGENT_MAX_TARGETS: usize = 128;
pub const DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS: usize = 1;
pub const DEFAULT_AGENT_MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const DEFAULT_AGENT_RATE_LIMIT_REQUESTS: usize = 120;
pub const DEFAULT_AGENT_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

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
    pub unix_socket: Option<PathBuf>,
    pub runs_dir: PathBuf,
    pub allow_unsafe_bind: bool,
    pub bearer_token: Option<String>,
    pub read_token: Option<String>,
    pub apply_token: Option<String>,
    pub max_duration_seconds: u64,
    pub max_targets: usize,
    pub max_concurrent_recordings: usize,
    pub autotune_limits: AgentAutotuneLimits,
    pub health_thresholds: SystemHealthThresholds,
    pub rollback_on_crash_recovery: bool,
}

#[derive(Clone, Debug)]
pub struct AgentLimits {
    pub max_duration_seconds: u64,
    pub max_targets: usize,
    pub max_concurrent_recordings: usize,
}

#[derive(Clone, Debug, Default)]
pub struct AgentAuth {
    pub bearer_token: Option<String>,
    pub read_token: Option<String>,
    pub apply_token: Option<String>,
}

pub struct AgentState {
    pub active_run: Mutex<Option<RunHandle>>,
    pub active_autotune: Mutex<Option<AutotuneControllerHandle>>,
    pub daemon_state: Mutex<DaemonState>,
    pub runs_dir: PathBuf,
    pub auth: AgentAuth,
    pub bind: SocketAddr,
    pub unix_socket: Option<PathBuf>,
    pub limits: AgentLimits,
    pub autotune_limits: AgentAutotuneLimits,
    pub health_thresholds: SystemHealthThresholds,
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

#[derive(Debug)]
pub(crate) struct AgentRateLimiter {
    max_requests: usize,
    window: Duration,
    accepted: Mutex<VecDeque<Instant>>,
}

impl Default for AgentRateLimiter {
    fn default() -> Self {
        Self {
            max_requests: DEFAULT_AGENT_RATE_LIMIT_REQUESTS,
            window: DEFAULT_AGENT_RATE_LIMIT_WINDOW,
            accepted: Mutex::new(VecDeque::new()),
        }
    }
}

impl AgentRateLimiter {
    #[cfg(test)]
    pub(crate) fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            accepted: Mutex::new(VecDeque::new()),
        }
    }

    pub(crate) async fn accept(&self, now: Instant) -> bool {
        let mut accepted = self.accepted.lock().await;
        while accepted
            .front()
            .is_some_and(|previous| now.duration_since(*previous) >= self.window)
        {
            accepted.pop_front();
        }

        if accepted.len() >= self.max_requests {
            return false;
        }

        accepted.push_back(now);
        true
    }
}

#[derive(Serialize)]
pub(crate) struct ErrorResponse {
    error: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonStatusResponse {
    active_recording: bool,
    active_autotune: bool,
    daemon_state: DaemonState,
    capabilities: DaemonCapabilities,
    health: SystemHealthSnapshot,
    watchdog: DaemonWatchdogReport,
    manual_restore_command: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonHealthResponse {
    ok: bool,
    phase: DaemonPhase,
    health: SystemHealthSnapshot,
    degraded: Vec<crate::daemon::state::DaemonDegradedStatus>,
    faulted: Option<crate::daemon::state::DaemonFaultState>,
    unavailable_features: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonPolicyResponse {
    policy: DaemonPolicy,
    explanation: PolicyExplanation,
    policy_explanation: DaemonPolicyExplanation,
    capabilities: DaemonCapabilities,
    health: SystemHealthSnapshot,
    watchdog: DaemonWatchdogReport,
    manual_restore_command: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonExplainResponse {
    daemon_state: DaemonState,
    policy: DaemonPolicy,
    explanation: PolicyExplanation,
    policy_explanation: DaemonPolicyExplanation,
    capabilities: DaemonCapabilities,
    health: SystemHealthSnapshot,
    watchdog: DaemonWatchdogReport,
    why_no_optimize: Vec<String>,
    what_changed: Vec<String>,
    manual_restore_command: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonControlResponse {
    ok: bool,
    phase: DaemonPhase,
    message: String,
    manual_restore_command: String,
    restore_messages: Vec<String>,
}

pub async fn run_agent(config: AgentConfig) -> anyhow::Result<()> {
    if config.unix_socket.is_none() && !is_local_bind(&config.bind) && !config.allow_unsafe_bind {
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

    let initial_daemon_state = daemon_state_from_startup_recovery(&startup_recovery);

    match startup_recovery {
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Recovered {
            experiment_id,
            action_id,
            affected_tasks,
            manual_restore_command,
        } => {
            log::info!(
                "autotune startup recovery: rolled back interrupted action experiment_id={} action_id={} affected_tasks={} manual_restore_command=\"{}\"",
                experiment_id,
                action_id,
                affected_tasks,
                manual_restore_command
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Faulted {
            experiment_id,
            action_id,
            manual_restore_command,
            reason,
        } => {
            log::error!(
                "autotune startup recovery faulted: experiment_id={} action_id={} reason={}",
                experiment_id,
                action_id,
                reason
            );
            log::error!("manual restore command: {}", manual_restore_command);
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
            log::warn!(
                "autotune startup recovery: rollback_on_crash_recovery=false; leaving applied journal in place experiment_id={} action_id={} manual_restore_command=\"{}\"",
                experiment_id,
                action_id,
                manual_restore_command
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::ApplyingWithoutRollback {
            experiment_id,
            action_id,
        } => {
            log::warn!(
                "autotune startup recovery: found applying journal without rollback token experiment_id={} action_id={}; no automatic rollback attempted",
                experiment_id,
                action_id
            );
        }
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Clean => {}
    }

    if config.unix_socket.is_none() && !is_local_bind(&config.bind) {
        if config.bearer_token.is_none()
            && config.read_token.is_none()
            && config.apply_token.is_none()
        {
            log::warn!(
                "WARNING: stutter agent is listening on a non-loopback address without authentication because --allow-unsafe-bind was passed."
            );
        } else {
            log::warn!(
                "WARNING: stutter agent is listening on a non-loopback address. Authentication is enabled."
            );
        }
    }

    let auth = AgentAuth {
        bearer_token: config.bearer_token.clone(),
        read_token: config.read_token.clone(),
        apply_token: config.apply_token.clone(),
    };
    let auth_enabled = auth.any_token_configured();

    audit_agent_event(
        "remote-agent-listen",
        true,
        0,
        agent_listen_audit_message(&config, auth_enabled),
    );

    let listen_bind = config.bind;
    let listen_unix_socket = config.unix_socket.clone();
    let state = Arc::new(AgentState {
        active_run: Mutex::new(None),
        active_autotune: Mutex::new(None),
        daemon_state: Mutex::new(initial_daemon_state),
        runs_dir: config.runs_dir,
        bind: listen_bind,
        unix_socket: listen_unix_socket.clone(),
        auth,
        autotune_limits: config.autotune_limits,
        health_thresholds: config.health_thresholds,
        limits: AgentLimits {
            max_duration_seconds: config.max_duration_seconds,
            max_targets: config.max_targets,
            max_concurrent_recordings: config.max_concurrent_recordings,
        },
    });

    let request_rate_limiter = Arc::new(AgentRateLimiter::default());
    let app = routes::build_agent_router(state, request_rate_limiter);

    if let Some(path) = listen_unix_socket {
        log::info!("stutter agent listening on unix://{}", path.display());
        server::serve_unix_socket(path, app).await?;
    } else {
        log::info!("stutter agent listening on http://{}", listen_bind);
        let listener = tokio::net::TcpListener::bind(listen_bind).await?;
        axum::serve(listener, app).await?;
    }

    Ok(())
}

fn agent_listen_audit_message(config: &AgentConfig, auth_enabled: bool) -> String {
    match config.unix_socket.as_ref() {
        Some(path) => format!(
            "bind=unix:{} auth_enabled={} allow_unsafe_bind={} runs_dir={}",
            path.display(),
            auth_enabled,
            config.allow_unsafe_bind,
            config.runs_dir.display()
        ),
        None => format!(
            "bind={} auth_enabled={} allow_unsafe_bind={} runs_dir={}",
            config.bind,
            auth_enabled,
            config.allow_unsafe_bind,
            config.runs_dir.display()
        ),
    }
}

pub fn default_runs_dir() -> PathBuf {
    let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    p.push("stutter-runs");
    p
}

pub fn default_agent_unix_socket_path() -> anyhow::Result<PathBuf> {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(runtime_dir).join("stutter-agent.sock"));
    }

    let Some(home) = std::env::var_os("HOME") else {
        anyhow::bail!(
            "cannot choose default agent unix socket path without XDG_RUNTIME_DIR or HOME"
        );
    };

    Ok(PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("stutter")
        .join("agent.sock"))
}

pub fn load_bearer_token(
    bearer_token_env: &str,
    bearer_token_file: Option<&StdPath>,
) -> anyhow::Result<Option<String>> {
    if let Some(path) = bearer_token_file {
        let token =
            std::fs::read_to_string(path).map_err(|source| AgentError::BearerTokenFile {
                path: path.to_path_buf(),
                source,
            })?;
        return normalize_bearer_token(token).map_err(anyhow::Error::new);
    }

    if let Some(value) = std::env::var_os(bearer_token_env) {
        return normalize_bearer_token(value.to_string_lossy().into_owned())
            .map_err(anyhow::Error::new);
    }

    Ok(None)
}

fn normalize_bearer_token(raw: String) -> Result<Option<String>, AgentError> {
    let token = raw.trim().to_owned();
    if token.is_empty() {
        return Err(AgentError::EmptyBearerToken);
    }
    Ok(Some(token))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AgentAuthScope {
    Read,
    Apply,
}

impl AgentAuth {
    fn any_token_configured(&self) -> bool {
        self.bearer_token.is_some() || self.read_token.is_some() || self.apply_token.is_some()
    }

    fn apply_token_configured(&self) -> bool {
        self.bearer_token.is_some() || self.apply_token.is_some()
    }
}

fn authorize(headers: &HeaderMap, auth: &AgentAuth) -> Result<(), StatusCode> {
    authorize_with_scope(headers, auth, AgentAuthScope::Read)
}

fn authorize_apply(headers: &HeaderMap, auth: &AgentAuth) -> Result<(), StatusCode> {
    authorize_with_scope(headers, auth, AgentAuthScope::Apply)
}

#[cfg(test)]
fn authorize_state_change(headers: &HeaderMap, state: &AgentState) -> Result<(), StatusCode> {
    authorize_agent_privileged_operation(headers, state, PrivilegedOperation::ControlDaemon)
}

fn authorize_remote_autotune_start(
    headers: &HeaderMap,
    state: &AgentState,
) -> Result<(), StatusCode> {
    if state.unix_socket.is_none() && !state.auth.apply_token_configured() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    authorize_apply(headers, &state.auth)
}

fn authorize_agent_privileged_operation(
    headers: &HeaderMap,
    state: &AgentState,
    operation: PrivilegedOperation,
) -> Result<(), StatusCode> {
    let transport = agent_privilege_transport(state);
    let socket_authorized =
        matches!(transport, PrivilegeTransport::UnixSocket) && !state.auth.any_token_configured();

    if !socket_authorized
        && operation.requires_apply_authorization()
        && !state.auth.apply_token_configured()
    {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let apply_result = if socket_authorized {
        Ok(())
    } else {
        authorize_apply(headers, &state.auth)
    };

    if operation.requires_apply_authorization()
        && let Err(status) = apply_result
    {
        return Err(status);
    }

    let request = PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::ControlPlane,
        operation,
        transport,
        authenticated: socket_authorized || authorize(headers, &state.auth).is_ok(),
        apply_authorized: socket_authorized || apply_result.is_ok(),
    };
    let decision = PrivilegeCommandAllowlist.check(&request);
    if decision.allowed {
        Ok(())
    } else {
        Err(status_for_privilege_decision(&decision.reason_code))
    }
}

fn agent_privilege_transport(state: &AgentState) -> PrivilegeTransport {
    if state.unix_socket.is_some() {
        PrivilegeTransport::UnixSocket
    } else if state.bind.ip().is_loopback() {
        PrivilegeTransport::LoopbackTcp
    } else {
        PrivilegeTransport::RemoteTcp
    }
}

fn status_for_privilege_decision(reason_code: &str) -> StatusCode {
    match reason_code {
        "missing_authentication" | "missing_apply_authorization" => StatusCode::UNAUTHORIZED,
        _ => StatusCode::FORBIDDEN,
    }
}

fn authorize_with_scope(
    headers: &HeaderMap,
    auth: &AgentAuth,
    scope: AgentAuthScope,
) -> Result<(), StatusCode> {
    if !auth.any_token_configured() {
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

    let legacy_match = token_matches(token, auth.bearer_token.as_deref());
    let apply_match = token_matches(token, auth.apply_token.as_deref());
    let read_match = token_matches(token, auth.read_token.as_deref());

    match scope {
        AgentAuthScope::Read if legacy_match || apply_match || read_match => Ok(()),
        AgentAuthScope::Apply if legacy_match || apply_match => Ok(()),
        _ => Err(StatusCode::FORBIDDEN),
    }
}

fn token_matches(token: &str, expected: Option<&str>) -> bool {
    expected.is_some_and(|expected| constant_time_eq(token.as_bytes(), expected.as_bytes()))
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

fn agent_state_is_local(state: &AgentState) -> bool {
    state.unix_socket.is_some() || is_local_bind(&state.bind)
}

#[cfg(test)]
mod tests;
