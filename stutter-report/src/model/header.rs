use serde::{Deserialize, Serialize};
use stutter_core::ids::Pid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportHeaderSummary {
    pub file_path: String,
    pub schema_version: u32,
    pub expected_schema_version: u32,
    pub run_name: String,
    pub duration_ms: u64,
    pub stop_reason: String,
    pub manual_pids: Vec<Pid>,
    pub tree_roots: Vec<Pid>,
    pub include_comm: Vec<String>,
    pub exclude_comm: Vec<String>,
    pub event_stream_warning: Option<String>,
    pub watch_process: String,
    pub persistent: bool,
    pub csv_stream: String,
    pub active_target_pids_count: u64,
}
