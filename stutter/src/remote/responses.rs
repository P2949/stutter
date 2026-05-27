use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRecordResponse {
    pub run_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopRecordResponse {
    pub run_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordStatusResponse {
    pub active: bool,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub ok: bool,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunsResponse {
    pub runs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionResponse {
    pub name: String,
    pub version: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitiesResponse {
    pub version: String,
    pub auth_required: bool,
    pub max_duration_seconds: u64,
    pub max_targets: usize,
    pub max_concurrent_recordings: usize,
    pub supported_routes: Vec<String>,
    pub supported_artifacts: Vec<String>,
    pub features: AgentFeatureFlags,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFeatureFlags {
    pub record_start_stop: bool,
    pub list_runs: bool,
    pub download_session: bool,
    pub download_artifacts: bool,
    pub hwmon_request: bool,
    pub cpu_freq_request: bool,
    pub faults_request: bool,
    pub stat_wait_request: bool,
    pub block_io_request: bool,
    pub irq_latency_request: bool,
    pub autotune_observe: bool,
    pub foreground_window_request: bool,
    pub autotune_suggest: bool,
    pub autotune_apply_low_risk: bool,
}
