//! Agent runtime state contracts.

use super::*;

pub struct AgentState {
    // Shared by Axum handlers through `Arc<AgentState>`. Each mutex protects a short-lived
    // request coordination slot; long-running work is owned by the stored task handle.
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
    // Stop uses a one-shot cancellation signal; completion and cleanup are observed through join.
    pub stop_tx: oneshot::Sender<()>,
    pub join: JoinHandle<anyhow::Result<crate::autotune::runtime::AutotuneControllerExit>>,
}

pub struct RunHandle {
    pub id: String,
    // Recording tasks follow the same cancel-then-join ownership model as autotune controllers.
    pub stop_tx: oneshot::Sender<()>,
    pub join: JoinHandle<anyhow::Result<String>>,
}
