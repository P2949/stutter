//! Agent response DTOs shared by handlers.

use super::*;

#[derive(Serialize)]
pub(crate) struct ErrorResponse {
    pub(crate) error: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonStatusResponse {
    pub(crate) active_recording: bool,
    pub(crate) active_autotune: bool,
    #[serde(flatten)]
    pub(crate) remote_access: RemoteAccessStatus,
    pub(crate) daemon_state: DaemonState,
    pub(crate) capabilities: DaemonCapabilities,
    pub(crate) health: SystemHealthSnapshot,
    pub(crate) watchdog: DaemonWatchdogReport,
    pub(crate) manual_restore_command: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonHealthResponse {
    pub(crate) ok: bool,
    pub(crate) phase: DaemonPhase,
    pub(crate) health: SystemHealthSnapshot,
    pub(crate) degraded: Vec<crate::daemon::state::DaemonDegradedStatus>,
    pub(crate) faulted: Option<crate::daemon::state::DaemonFaultState>,
    pub(crate) unavailable_features: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonPolicyResponse {
    pub(crate) policy: DaemonPolicy,
    pub(crate) explanation: PolicyExplanation,
    pub(crate) policy_explanation: DaemonPolicyExplanation,
    pub(crate) capabilities: DaemonCapabilities,
    pub(crate) health: SystemHealthSnapshot,
    pub(crate) watchdog: DaemonWatchdogReport,
    pub(crate) manual_restore_command: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonExplainResponse {
    pub(crate) daemon_state: DaemonState,
    pub(crate) policy: DaemonPolicy,
    pub(crate) explanation: PolicyExplanation,
    pub(crate) policy_explanation: DaemonPolicyExplanation,
    pub(crate) capabilities: DaemonCapabilities,
    pub(crate) health: SystemHealthSnapshot,
    pub(crate) watchdog: DaemonWatchdogReport,
    pub(crate) why_no_optimize: Vec<String>,
    pub(crate) what_changed: Vec<String>,
    pub(crate) manual_restore_command: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DaemonControlResponse {
    pub(crate) ok: bool,
    pub(crate) phase: DaemonPhase,
    pub(crate) message: String,
    pub(crate) manual_restore_command: String,
    pub(crate) restore_messages: Vec<String>,
}
