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

pub(crate) use std::{
    collections::VecDeque,
    net::SocketAddr,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path as StdPath, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) use anyhow::Context;
pub(crate) use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
pub(crate) use hyper::body::Incoming;
pub(crate) use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as HyperConnectionBuilder,
    service::TowerToHyperService,
};
pub(crate) use serde::Serialize;
pub(crate) use tokio::{
    sync::{Mutex, Semaphore, oneshot},
    task::JoinHandle,
};
pub(crate) use tower::{Service, ServiceExt as _};

pub use crate::error::AgentError;
pub(crate) use crate::{
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
        RecordStatusResponse, RemoteAccessStatus, RemoteMonitorRequest, RunsResponse,
        StartRecordResponse, StopRecordResponse, VersionResponse,
    },
};

pub(crate) mod artifacts;
pub(crate) mod audit;
pub(crate) mod auth;
pub(crate) mod autotune;
pub(crate) mod bind;
pub(crate) mod config;
pub(crate) mod daemon;
pub(crate) mod rate_limit;
pub(crate) mod recording;
pub(crate) mod responses;
pub(crate) mod routes;
pub(crate) mod server;
pub(crate) mod startup;
pub(crate) mod state;

pub(crate) use artifacts::AGENT_ARTIFACT_ALLOWLIST;
pub(crate) use audit::{audit_agent_event, audit_agent_event_with_safety};
pub use auth::AgentAuth;
#[cfg(test)]
pub(crate) use auth::agent_privilege_transport;
#[cfg(test)]
pub(crate) use auth::authorize_apply;
#[cfg(test)]
pub(crate) use auth::authorize_state_change;
pub(crate) use auth::{
    agent_state_is_local, authorize, authorize_agent_privileged_operation,
    authorize_remote_autotune_start, remote_access_status,
};
#[cfg(test)]
pub(crate) use bind::agent_listen_audit_message;
pub(crate) use bind::is_local_bind;
pub use bind::run_agent;
#[cfg(test)]
pub(crate) use config::normalize_bearer_token;
pub use config::{
    AgentConfig, AgentLimits, default_agent_unix_socket_path, default_runs_dir, load_bearer_token,
};
pub(crate) use rate_limit::AgentRateLimiter;
pub(crate) use responses::{
    DaemonControlResponse, DaemonExplainResponse, DaemonHealthResponse, DaemonPolicyResponse,
    DaemonStatusResponse, ErrorResponse,
};
pub use state::{AgentState, AutotuneControllerHandle, RunHandle};

pub const DEFAULT_AGENT_MAX_DURATION_SECONDS: u64 = 300;
pub const DEFAULT_AGENT_MAX_TARGETS: usize = 128;
pub const DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS: usize = 1;
pub const DEFAULT_AGENT_UNIX_CONNECTION_LIMIT: usize = 128;
pub const DEFAULT_AGENT_UNIX_CONNECTION_TIMEOUT_MS: u64 = 60_000;
pub const DEFAULT_AGENT_UNIX_CONNECTION_TIMEOUT: Duration =
    Duration::from_millis(DEFAULT_AGENT_UNIX_CONNECTION_TIMEOUT_MS);
pub const DEFAULT_AGENT_MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const DEFAULT_AGENT_RATE_LIMIT_REQUESTS: usize = 120;
pub const DEFAULT_AGENT_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

#[cfg(test)]
mod tests;
