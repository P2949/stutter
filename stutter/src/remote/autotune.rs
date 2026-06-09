use serde::{Deserialize, Serialize};

use super::limits::AgentAutotuneLimits;
use crate::daemon::DaemonState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutotuneStartRequest {
    pub mode: String,
    pub watch_process: Option<String>,
    pub tree_pid: Option<u32>,
    pub profiles: Option<String>,
    pub config: Option<String>,
    pub duration_seconds: Option<u64>,
    pub decision_log: Option<String>,
    #[serde(default)]
    pub summary_ms: Option<u64>,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub hwmon: bool,
    #[serde(default)]
    pub mangohud_log: Option<String>,
    #[serde(default)]
    pub auto_focus: bool,
    #[serde(default)]
    pub focus_source: Option<String>,
    #[serde(default)]
    pub foreground_window: bool,
    #[serde(default)]
    pub foreground_source: Option<String>,
    #[serde(default)]
    pub foreground_poll_ms: Option<u64>,
    #[serde(default)]
    pub foreground_max_stale_ms: Option<u64>,
    #[serde(default)]
    pub washout_seconds: Option<u64>,
    #[serde(default)]
    pub washout_verify_interval_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutotuneStatusResponse {
    pub active: bool,
    pub mode: Option<String>,
    pub watch_process: Option<String>,
    pub tree_pid: Option<u32>,
    pub started_unix_nanos: Option<u128>,
    pub focus_group: Option<String>,
    pub target_root: Option<u32>,
    pub current_score: Option<u64>,
    pub active_profile: Option<String>,
    pub last_decision: Option<String>,
    pub rollback_available: bool,
    pub cooldown_remaining_seconds: Option<u64>,
    pub data_quality: Option<String>,
    pub last_fault: Option<String>,
    pub manual_restore_command: Option<String>,
    pub daemon_state: DaemonState,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutotuneStartResponse {
    pub status: String,
    pub mode: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutotuneStopResponse {
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutotuneRestoreResponse {
    pub status: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_actions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_actions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_actions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_records: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_missing: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_identity_mismatch: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_records: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restore_messages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutotuneHistoryResponse {
    pub path: String,
    pub events: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutotuneConfigResponse {
    pub default_mode: String,
    pub supported_modes: Vec<String>,
    pub apply_low_risk_remote_enabled: bool,
    pub local_only_by_default: bool,
    pub history_path: String,
    pub autotune_limits: AgentAutotuneLimits,
    pub daemon_scope: String,
    pub allow_system_wide_suggestions: bool,
    pub allow_system_wide_apply: bool,
    pub minimum_focus_confidence: f32,
    pub required_stable_focus_polls: u32,
}
