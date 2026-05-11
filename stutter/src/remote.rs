use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize};

use crate::{actions::SafetyClass, cli::Config, daemon_policy::DaemonMode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteMonitorRequest {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub exclude_tree_pids: Vec<u32>,
    pub duration_seconds: Option<u64>,

    pub spike_us: Option<u64>,
    pub summary_ms: Option<u64>,

    pub include_comm: Vec<String>,
    pub exclude_comm: Vec<String>,

    pub hwmon: bool,
    pub cpu_freq: bool,
    pub faults: bool,
    pub stat_wait: bool,
    pub block_io: bool,
    #[serde(default)]
    pub runtime_slices: bool,
    #[serde(default)]
    pub runtime_slices_max_tasks: Option<usize>,

    pub irq_latency: bool,
    pub irqs: Vec<u32>,

    #[serde(default)]
    pub foreground_window: bool,
    #[serde(default)]
    pub focus_source: Option<String>,
    #[serde(default)]
    pub foreground_source: Option<String>,
    #[serde(default)]
    pub foreground_poll_ms: Option<u64>,
    #[serde(default)]
    pub foreground_max_stale_ms: Option<u64>,
    #[serde(default)]
    pub foreground_include_title: bool,

    pub record: bool,
    pub run_name: Option<String>,
}

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentAutotuneLimits {
    pub max_active_controllers: usize,
    pub max_mode: DaemonMode,
    pub max_safety_class: SafetyClass,
    pub allow_high_risk: bool,
    pub max_candidate_window_seconds: u64,
    pub max_targets: usize,
    pub allow_system_wide_actions: bool,
}

impl Default for AgentAutotuneLimits {
    fn default() -> Self {
        Self {
            max_active_controllers: 1,
            max_mode: DaemonMode::ApplyLowRisk,
            max_safety_class: SafetyClass::ReversibleLowRisk,
            allow_high_risk: false,
            max_candidate_window_seconds: 120,
            max_targets: 1,
            allow_system_wide_actions: false,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AgentAutotuneLimitsCompat {
    #[serde(default = "default_max_active_controllers")]
    max_active_controllers: usize,
    #[serde(default)]
    max_mode: Option<DaemonMode>,
    #[serde(default)]
    max_safety_class: Option<String>,
    #[serde(default)]
    allow_high_risk: bool,
    #[serde(default = "default_max_candidate_window_seconds")]
    max_candidate_window_seconds: u64,
    #[serde(default = "default_max_targets")]
    max_targets: usize,
    #[serde(default)]
    allow_system_wide_actions: bool,
}

fn default_max_active_controllers() -> usize {
    1
}

fn default_max_candidate_window_seconds() -> u64 {
    120
}

fn default_max_targets() -> usize {
    1
}

pub fn parse_legacy_safety_class(value: Option<&str>) -> Result<SafetyClass, String> {
    match value {
        Some("ObserveOnly") | Some("observe_only") => Ok(SafetyClass::ObserveOnly),
        Some("ReversibleMediumRisk") | Some("reversible_medium_risk") => {
            Ok(SafetyClass::ReversibleMediumRisk)
        }
        Some("HighRisk") | Some("high_risk") => Ok(SafetyClass::HighRisk),
        Some("ReversibleLowRisk") | Some("reversible_low_risk") | None => {
            Ok(SafetyClass::ReversibleLowRisk)
        }
        Some(other) => Err(format!(
            "invalid safety class {:?}; valid values are ObserveOnly, ReversibleLowRisk, ReversibleMediumRisk, HighRisk",
            other
        )),
    }
}

pub fn mode_for_safety_class(safety_class: SafetyClass) -> DaemonMode {
    match safety_class {
        SafetyClass::ObserveOnly => DaemonMode::Suggest,
        SafetyClass::ReversibleLowRisk => DaemonMode::ApplyLowRisk,
        SafetyClass::ReversibleMediumRisk => DaemonMode::ApplyMediumRisk,
        SafetyClass::HighRisk => DaemonMode::ApplyHighRisk,
    }
}

impl<'de> Deserialize<'de> for AgentAutotuneLimits {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let compat = AgentAutotuneLimitsCompat::deserialize(deserializer)?;
        let max_safety_class = parse_legacy_safety_class(compat.max_safety_class.as_deref())
            .map_err(serde::de::Error::custom)?;
        let max_mode = compat
            .max_mode
            .unwrap_or_else(|| mode_for_safety_class(max_safety_class.clone()));

        Ok(Self {
            max_active_controllers: compat.max_active_controllers,
            max_mode,
            max_safety_class,
            allow_high_risk: compat.allow_high_risk,
            max_candidate_window_seconds: compat.max_candidate_window_seconds,
            max_targets: compat.max_targets,
            allow_system_wide_actions: compat.allow_system_wide_actions,
        })
    }
}

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
    pub allow_system_wide_actions: bool,
    pub minimum_focus_confidence: f32,
    pub required_stable_focus_polls: u32,
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

fn focus_source_label(source: crate::cli::FocusSource) -> String {
    match source {
        crate::cli::FocusSource::Heuristic => "heuristic",
        crate::cli::FocusSource::Foreground => "foreground",
        crate::cli::FocusSource::Hybrid => "hybrid",
    }
    .to_owned()
}

fn foreground_source_label(source: crate::cli::ForegroundSourceArg) -> String {
    match source {
        crate::cli::ForegroundSourceArg::Auto => "auto",
        crate::cli::ForegroundSourceArg::Sway => "sway",
        crate::cli::ForegroundSourceArg::Hyprland => "hyprland",
        crate::cli::ForegroundSourceArg::X11 => "x11",
    }
    .to_owned()
}

pub fn request_from_monitor_config(config: &Config) -> anyhow::Result<RemoteMonitorRequest> {
    Ok(RemoteMonitorRequest {
        target_pids: config.target_pids.clone(),
        tree_pids: config.tree_pids.clone(),
        exclude_tree_pids: config.exclude_tree_pids.clone(),
        duration_seconds: config.max_duration.map(|d| d.as_secs()),
        spike_us: Some(config.spike_threshold_ns / 1000),
        summary_ms: Some(config.summary_period_ms),
        include_comm: config
            .task_filters
            .include_comm
            .iter()
            .map(|p| p.raw().to_owned())
            .collect(),
        exclude_comm: config
            .task_filters
            .exclude_comm
            .iter()
            .map(|p| p.raw().to_owned())
            .collect(),
        hwmon: config.hwmon,
        cpu_freq: config.cpu_freq,
        faults: config.faults,
        stat_wait: config.stat_wait,
        block_io: config.block_io,
        runtime_slices: config.runtime_slices,
        runtime_slices_max_tasks: Some(config.runtime_slices_max_tasks),
        irq_latency: config.irq_latency,
        irqs: config.irqs.clone(),
        foreground_window: config.foreground_window,
        focus_source: Some(focus_source_label(config.focus_source)),
        foreground_source: Some(foreground_source_label(config.foreground_source)),
        foreground_poll_ms: Some(config.foreground_poll_ms),
        foreground_max_stale_ms: Some(config.foreground_max_stale_ms),
        foreground_include_title: config.foreground_include_title,
        record: config.recording.is_some(),
        run_name: config.recording.as_ref().and_then(|r| r.run_name.clone()),
    })
}

pub async fn run_remote_monitor(
    endpoint: &str,
    request: RemoteMonitorRequest,
) -> anyhow::Result<()> {
    let base = endpoint.trim_end_matches('/');
    let client = reqwest::Client::new();

    let start_url = format!("{base}/record/start");
    log::info!("sending remote start request to {start_url}");

    let start: StartRecordResponse = apply_auth(client.post(&start_url))
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    println!("remote recording started: run_id={}", start.run_id);

    let stop_handle = tokio::spawn({
        let client = client.clone();
        let base = base.to_owned();
        async move {
            if let Err(e) = tokio::signal::ctrl_c().await {
                log::error!("failed to wait for ctrl-c: {e}");
            }
            log::info!("ctrl-c detected, sending remote stop request");
            let _ = apply_auth(client.post(format!("{base}/record/stop")))
                .send()
                .await;
        }
    });

    if let Some(seconds) = request.duration_seconds {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(seconds)) => {
                log::info!("duration reached, sending remote stop request");
            }
            _ = stop_handle => {
                return Ok(());
            }
        }
    } else {
        stop_handle.await?;
        return Ok(());
    }

    let stop: StopRecordResponse = apply_auth(client.post(format!("{base}/record/stop")))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    println!("remote recording stopped: run_id={:?}", stop.run_id);

    Ok(())
}

fn maybe_bearer_token_from_env() -> Option<String> {
    std::env::var("STUTTER_AGENT_TOKEN")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn apply_auth(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Some(token) = maybe_bearer_token_from_env() {
        request.bearer_auth(token)
    } else {
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_monitor_request_round_trips_json() {
        let request = RemoteMonitorRequest {
            target_pids: vec![1234],
            tree_pids: vec![],
            exclude_tree_pids: vec![],
            duration_seconds: Some(5),
            spike_us: Some(1000),
            summary_ms: Some(500),
            include_comm: vec!["Game".to_string()],
            exclude_comm: vec![],
            hwmon: true,
            cpu_freq: true,
            faults: true,
            stat_wait: true,
            block_io: false,
            runtime_slices: true,
            runtime_slices_max_tasks: Some(64),
            irq_latency: false,
            irqs: vec![],
            foreground_window: true,
            focus_source: Some("hybrid".to_string()),
            foreground_source: Some("sway".to_string()),
            foreground_poll_ms: Some(1000),
            foreground_max_stale_ms: Some(2500),
            foreground_include_title: false,
            record: true,
            run_name: Some("test".to_string()),
        };

        let json = serde_json::to_string(&request).unwrap();
        let decoded: RemoteMonitorRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.target_pids, vec![1234]);
        assert_eq!(decoded.duration_seconds, Some(5));
        assert!(decoded.hwmon);
        assert!(decoded.runtime_slices);
        assert_eq!(decoded.runtime_slices_max_tasks, Some(64));
        assert!(decoded.foreground_window);
        assert_eq!(decoded.focus_source.as_deref(), Some("hybrid"));
        assert_eq!(decoded.foreground_source.as_deref(), Some("sway"));
        assert_eq!(decoded.foreground_poll_ms, Some(1000));
        assert_eq!(decoded.foreground_max_stale_ms, Some(2500));
        assert!(!decoded.foreground_include_title);
    }

    #[test]
    fn remote_monitor_request_defaults_foreground_fields_for_old_clients() {
        let json = r#"{
         "target_pids": [],
         "tree_pids": [],
         "exclude_tree_pids": [],
         "duration_seconds": null,
         "spike_us": null,
         "summary_ms": null,
         "include_comm": [],
         "exclude_comm": [],
         "hwmon": false,
         "cpu_freq": false,
         "faults": false,
         "stat_wait": false,
         "block_io": false,
         "irq_latency": false,
         "irqs": [],
         "record": false,
         "run_name": null
     }"#;

        let decoded: RemoteMonitorRequest = serde_json::from_str(json).unwrap();

        assert!(!decoded.foreground_window);
        assert!(!decoded.runtime_slices);
        assert_eq!(decoded.runtime_slices_max_tasks, None);
        assert_eq!(decoded.focus_source, None);
        assert_eq!(decoded.foreground_source, None);
        assert_eq!(decoded.foreground_poll_ms, None);
        assert_eq!(decoded.foreground_max_stale_ms, None);
        assert!(!decoded.foreground_include_title);
    }

    #[test]
    fn autotune_limits_default_to_low_risk_policy() {
        let limits = AgentAutotuneLimits::default();

        assert_eq!(limits.max_active_controllers, 1);
        assert_eq!(limits.max_mode, DaemonMode::ApplyLowRisk);
        assert_eq!(limits.max_safety_class, SafetyClass::ReversibleLowRisk);
        assert!(!limits.allow_high_risk);
        assert!(!limits.allow_system_wide_actions);
    }

    #[test]
    fn autotune_limits_accept_legacy_max_safety_class_json() {
        let json = r#"{
               "max_active_controllers": 1,
               "max_safety_class": "ReversibleLowRisk",
               "max_candidate_window_seconds": 120,
               "max_targets": 1,
               "allow_system_wide_actions": false
           }"#;

        let limits: AgentAutotuneLimits = serde_json::from_str(json).unwrap();

        assert_eq!(limits.max_mode, DaemonMode::ApplyLowRisk);
        assert_eq!(limits.max_safety_class, SafetyClass::ReversibleLowRisk);
        assert!(!limits.allow_high_risk);
    }

    #[test]
    fn autotune_limits_accept_typed_max_mode_json() {
        let json = r#"{
               "max_active_controllers": 1,
               "max_mode": "apply-medium-risk",
               "max_safety_class": "ReversibleMediumRisk",
               "allow_high_risk": false,
               "max_candidate_window_seconds": 120,
               "max_targets": 1,
               "allow_system_wide_actions": false
           }"#;

        let limits: AgentAutotuneLimits = serde_json::from_str(json).unwrap();

        assert_eq!(limits.max_mode, DaemonMode::ApplyMediumRisk);
        assert_eq!(limits.max_safety_class, SafetyClass::ReversibleMediumRisk);
    }
}
