use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    config::{FocusSource, ForegroundSource, TARGET_PIDS_MAX},
    ebpf_loader::DropCountersSnapshot,
    metadata::SystemMetadata,
    metrics::CpuPerfRecord,
    process_tree::TaskClass,
};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionMetadataCore {
    pub schema_version: u32,
    pub run_name: Option<String>,
    pub started_at: RecordedTime,
    pub ended_at: RecordedTime,
    pub monotonic_start_ns: Option<u64>,
    pub monotonic_end_ns: Option<u64>,
    pub duration_ms: u64,
    #[serde(default)]
    pub mangohud_start_offset: Option<u64>,
    #[serde(default)]
    pub mangohud_first_frame_monotonic_ns: Option<u64>,
    #[serde(default)]
    pub mangohud_first_frame_raw_elapsed_ms: Option<u64>,
    pub metadata: SystemMetadata,
    pub target_pids_max: u64,
    pub active_target_pids_count: u64,
    pub active_expanded_tasks: Vec<u32>,
    #[serde(default)]
    pub focus_mode: Option<String>,
    #[serde(default)]
    pub final_focus_kind: Option<String>,
    #[serde(default)]
    pub focus_switch_count: u64,
    #[serde(default)]
    pub focus_event_count: u64,
    #[serde(default)]
    pub foreground_event_count: u64,
    #[serde(default)]
    pub kms_flip_event_count: u64,
    #[serde(default)]
    pub drm_fence_event_count: u64,
    #[serde(default)]
    pub wayland_presentation_event_count: u64,
    #[serde(default)]
    pub display_path: Option<DisplayPathMetadata>,
    #[serde(default)]
    pub foreground_source: Option<String>,
    #[serde(default)]
    pub final_foreground_pid: Option<u32>,
    #[serde(default)]
    pub final_foreground_app_id: Option<String>,
    #[serde(default)]
    pub final_foreground_class: Option<String>,
    #[serde(default)]
    pub final_foreground_status: Option<String>,
    #[serde(default)]
    pub final_foreground_window_id: Option<String>,
    #[serde(default)]
    pub final_foreground_workspace: Option<String>,
    #[serde(default)]
    pub final_foreground_confidence: Option<f32>,
    #[serde(default)]
    pub final_foreground_stale_ms: Option<u64>,
    #[serde(default)]
    pub final_foreground_reason: Option<String>,
    #[serde(default)]
    pub interval_record_count: u64,
    #[serde(default)]
    pub intervals_dropped: u64,
    #[serde(default)]
    pub spike_events_retained_count: u64,
    #[serde(default)]
    pub spike_events_dropped_count: u64,
    #[serde(default)]
    pub spike_events_truncated: bool,
    #[serde(default)]
    pub scx_event_count: u64,
    #[serde(default)]
    pub irq_event_count: u64,
    #[serde(default)]
    pub migration_event_count: Option<u64>,
    #[serde(default)]
    pub cpu_freq_sample_count: Option<u64>,
    #[serde(default)]
    pub gpu_sample_count: u64,
    #[serde(default)]
    pub frame_event_count: u64,
    #[serde(default)]
    pub block_io_event_count: u64,
    #[serde(default)]
    pub runtime_slice_count: u64,
    #[serde(default)]
    pub runtime_slice_read_errors: u64,
    #[serde(default)]
    pub runtime_slice_skipped_tasks: u64,
    #[serde(default)]
    pub runtime_slice_source: Option<String>,
    #[serde(default)]
    pub event_stream_write_errors: u64,
    #[serde(default)]
    pub alert_events_dropped_count: u64,
    #[serde(default)]
    pub alert_channel_closed_count: u64,
    #[serde(default)]
    pub first_event_stream_write_error: Option<String>,
    #[serde(default = "super::event_types::default_block_io_correlation_basis_string")]
    pub block_io_correlation_basis: String,
    #[serde(default = "super::event_types::default_block_io_correlation_confidence_string")]
    pub block_io_correlation_confidence: String,
    #[serde(default)]
    pub drop_counters: DropCountersSnapshot,
    #[serde(default)]
    pub cpu_perf_sample_count: u64,
    #[serde(default)]
    pub cpu_perf_open_errors: u64,
    #[serde(default)]
    pub cpu_perf_read_errors: u64,
    #[serde(default)]
    pub cpu_perf_skipped_tasks: u64,
    #[serde(default)]
    pub cpu_perf_last_error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionFile {
    #[serde(flatten)]
    pub core: SessionMetadataCore,
    pub stop_reason: String,
    pub config: RecordedConfig,
    pub tasks: Vec<SessionTask>,
    pub top_spikes: Vec<SessionSpike>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayPathMetadata {
    pub label: Option<String>,
    pub render_gpu: Option<String>,
    pub scanout_gpu: Option<String>,
    pub connector: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct MetadataFile {
    #[serde(flatten)]
    pub core: SessionMetadataCore,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecordedConfig {
    pub manual_pids: Vec<u32>,
    pub tree_roots: Vec<u32>,
    #[serde(default)]
    pub cgroupv2: Option<PathBuf>,
    #[serde(default)]
    pub exclude_tree_pids: Vec<u32>,
    #[serde(default)]
    pub include_comm: Vec<String>,
    #[serde(default)]
    pub exclude_comm: Vec<String>,
    #[serde(default)]
    pub watch_process: Option<String>,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default)]
    pub keep_missing_pid: bool,
    #[serde(default)]
    pub watch_poll_ms: u64,
    #[serde(default)]
    pub watch_timeout_ms: Option<u64>,
    #[serde(default)]
    pub csv_stream: Option<crate::config::CsvStreamTarget>,
    #[serde(default)]
    pub irq_latency: bool,
    #[serde(default)]
    pub irqs: Vec<u32>,
    #[serde(default)]
    pub hwmon: bool,
    #[serde(default)]
    pub hwmon_root: Option<PathBuf>,
    #[serde(default)]
    pub hwmon_drm_card: Option<String>,
    #[serde(default)]
    pub hwmon_render_node: Option<PathBuf>,
    #[serde(default)]
    pub mangohud_log: Option<PathBuf>,
    #[serde(default)]
    pub mangohud_log_live: bool,
    #[serde(default)]
    pub tui: bool,
    pub summary_period_ms: u64,
    #[serde(default)]
    pub epoch_period_ms: Option<u64>,
    #[serde(default)]
    pub retain_intervals: Option<usize>,
    #[serde(default = "default_recorded_max_tasks")]
    pub max_tasks: usize,
    pub spike_threshold_ns: u64,
    #[serde(default)]
    pub alert_threshold_ns: Option<u64>,
    #[serde(default)]
    pub alert_webhook_url: Option<String>,
    #[serde(default = "default_recorded_follow_exec")]
    pub follow_exec: bool,
    pub verbose: bool,
    #[serde(default)]
    pub faults: bool,
    #[serde(default)]
    pub cpu_perf: bool,
    #[serde(default)]
    pub cpu_perf_kernel: bool,
    #[serde(default = "default_recorded_cpu_perf_max_tasks")]
    pub cpu_perf_max_tasks: usize,
    #[serde(default)]
    pub cpu_perf_cache_refs: bool,
    #[serde(default)]
    pub block_io: bool,
    #[serde(default)]
    pub stat_wait: bool,
    #[serde(default)]
    pub runtime_slices: bool,
    #[serde(default)]
    pub runtime_slices_max_tasks: usize,
    #[serde(default)]
    pub kms_timing: bool,
    #[serde(default)]
    pub kms_card: Option<String>,
    #[serde(default)]
    pub kms_connector: Option<String>,
    #[serde(default)]
    pub kms_crtc: Option<u32>,
    #[serde(default)]
    pub drm_fence_latency: bool,
    #[serde(default)]
    pub drm_fence_render_card: Option<String>,
    #[serde(default)]
    pub drm_fence_display_card: Option<String>,
    #[serde(default)]
    pub drm_fence_driver: Option<String>,
    #[serde(default)]
    pub wayland_presentation: bool,
    #[serde(default)]
    pub wayland_presentation_log: Option<PathBuf>,
    #[serde(default)]
    pub wayland_presentation_source: String,
    #[serde(default)]
    pub display_path_label: Option<String>,
    #[serde(default)]
    pub display_render_gpu: Option<String>,
    #[serde(default)]
    pub display_scanout_gpu: Option<String>,
    #[serde(default)]
    pub display_connector: Option<String>,
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
    #[serde(default)]
    pub otel_service_name: String,
    #[serde(default)]
    pub auto_focus: bool,
    #[serde(default)]
    pub foreground_window: bool,
    #[serde(default)]
    pub focus_source: String,
    #[serde(default)]
    pub foreground_source: String,
    #[serde(default)]
    pub foreground_poll_ms: u64,
    #[serde(default)]
    pub foreground_max_stale_ms: u64,
    #[serde(default)]
    pub foreground_include_title: bool,
    #[serde(default)]
    pub auto_focus_poll_ms: u64,
    #[serde(default)]
    pub auto_focus_min_confidence: f32,
    #[serde(default)]
    pub auto_focus_switch_cooldown_ms: u64,
    #[serde(default)]
    pub auto_focus_switch_margin: f32,
    #[serde(default)]
    pub auto_focus_required_polls: u32,
    #[serde(default)]
    pub auto_focus_max_roots: usize,
}

fn default_recorded_max_tasks() -> usize {
    TARGET_PIDS_MAX
}

fn default_recorded_follow_exec() -> bool {
    true
}

pub(crate) fn focus_source_label(source: FocusSource) -> String {
    match source {
        FocusSource::Heuristic => "heuristic",
        FocusSource::Foreground => "foreground",
        FocusSource::Hybrid => "hybrid",
    }
    .to_owned()
}

pub(crate) fn foreground_source_arg_label(source: ForegroundSource) -> String {
    match source {
        ForegroundSource::Auto => "auto",
        ForegroundSource::Sway => "sway",
        ForegroundSource::Hyprland => "hyprland",
        ForegroundSource::X11 => "x11",
    }
    .to_owned()
}

pub(crate) fn foreground_source_label(source: crate::foreground::ForegroundSource) -> String {
    match source {
        crate::foreground::ForegroundSource::Auto => "auto",
        crate::foreground::ForegroundSource::Sway => "sway",
        crate::foreground::ForegroundSource::Hyprland => "hyprland",
        crate::foreground::ForegroundSource::X11 => "x11",
        crate::foreground::ForegroundSource::Unsupported => "unsupported",
    }
    .to_owned()
}

fn default_recorded_cpu_perf_max_tasks() -> usize {
    128
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecordedTime {
    pub unix_seconds: u64,
    pub unix_nanos: u32,
    #[serde(alias = "local")]
    pub system_time_debug: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionTask {
    pub task: u32,
    pub active: bool,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub removed_ms: Option<u64>,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub exe_dev: Option<u64>,
    #[serde(default)]
    pub exe_ino: Option<u64>,
    pub comm: String,
    pub latency: RecordedLatency,
    pub cpu: RecordedCpuSnapshot,
    pub top_spikes: Vec<RecordedSpike>,
    #[serde(default)]
    pub migration_count: u64,
    #[serde(default)]
    pub cross_numa_migrations: u64,
    #[serde(default)]
    pub top_wakers: Vec<WakerEntry>,
    #[serde(default)]
    pub sched_policy: Option<String>,
    #[serde(default)]
    pub stat_wait_sum_ns: Option<u64>,
    #[serde(default)]
    pub stat_wait_sum_ns_saturated: bool,
    #[serde(default)]
    pub stat_wait_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_perf: Option<CpuPerfRecord>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct WakerEntry {
    pub waker_tid: u32,
    pub waker_comm: String,
    pub count: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecordedLatency {
    pub samples: u64,
    pub stored_samples: u64,
    pub truncated_samples: u64,
    pub percentile_scope: String,
    #[serde(default)]
    pub histogram: Vec<crate::metrics::LatencyHistogramBucket>,
    pub min_ns: u64,
    pub avg_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecordedCpuSnapshot {
    pub busiest_cpu: Option<u32>,
    pub busiest_cpu_samples: u64,
    pub worst_cpu: Option<u32>,
    pub worst_cpu_max_ns: u64,
    pub spikiest_cpu: Option<u32>,
    pub spikiest_cpu_spikes: u64,
    pub per_cpu: Vec<crate::metrics::CpuLine>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct RecordedSpike {
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    pub cpu: u32,
    #[serde(default)]
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    #[serde(default)]
    pub switch_prev_pid: u32,
    #[serde(default)]
    pub switch_prev_state: i64,
    #[serde(default)]
    pub switch_prev_state_label: String,
    // Diagnostic-only count of monitored pending wakeups for this target/task.
    // This is not CPU runqueue depth and must not be used as true CPU contention.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    /// Approximate per-CPU runnable depth reconstructed from sched wakeup/switch
    /// tracepoints. This is not literal rq->nr_running.
    #[serde(default)]
    pub observed_runnable_depth: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
    #[serde(default)]
    pub scx_ops: Option<String>,
    #[serde(default)]
    pub scx_state: Option<String>,
    #[serde(default)]
    pub scx_enable_seq: Option<String>,

    #[serde(default)]
    pub waker_tid: u32,
    #[serde(default)]
    pub waker_comm: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cause_tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_cause: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct SessionSpike {
    pub task: u32,
    pub active: bool,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    pub comm: String,
    pub cpu: u32,
    #[serde(default)]
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    #[serde(default)]
    pub switch_prev_pid: u32,
    #[serde(default)]
    pub switch_prev_state: i64,
    #[serde(default)]
    pub switch_prev_state_label: String,
    // Diagnostic-only count of monitored pending wakeups for this target/task.
    // This is not CPU runqueue depth and must not be used as true CPU contention.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    /// Approximate per-CPU runnable depth reconstructed from sched wakeup/switch
    /// tracepoints. This is not literal rq->nr_running.
    #[serde(default)]
    pub observed_runnable_depth: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
    #[serde(default)]
    pub scx_ops: Option<String>,
    #[serde(default)]
    pub scx_state: Option<String>,
    #[serde(default)]
    pub scx_enable_seq: Option<String>,

    #[serde(default)]
    pub waker_tid: u32,
    #[serde(default)]
    pub waker_comm: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cause_tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_cause: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foreground::{ForegroundEvent, ForegroundEventInput};

    #[test]
    fn foreground_event_serializes_without_title_by_default() {
        let event = ForegroundEvent::new(ForegroundEventInput {
            elapsed_ms: 1_000,
            source: crate::foreground::ForegroundSource::Sway,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(4242),
            app_id: Some("steam_app_379430".to_owned()),
            class: Some("steam_app_379430".to_owned()),
            title: Some("Private game or browser title".to_owned()),
            include_title: false,
            window_id: Some("7".to_owned()),
            workspace: Some("gaming".to_owned()),
            confidence: 0.95,
            stale_ms: None,
            reason: "focused Sway node from swaymsg get_tree".to_owned(),
        });

        let value = serde_json::to_value(&event).unwrap();

        assert_eq!(
            value.get("elapsed_ms").and_then(serde_json::Value::as_u64),
            Some(1_000)
        );
        assert_eq!(
            value.get("source").and_then(serde_json::Value::as_str),
            Some("sway")
        );
        assert_eq!(
            value.get("status").and_then(serde_json::Value::as_str),
            Some("available")
        );
        assert_eq!(
            value.get("pid").and_then(serde_json::Value::as_u64),
            Some(4242)
        );
        assert_eq!(
            value.get("app_id").and_then(serde_json::Value::as_str),
            Some("steam_app_379430")
        );
        assert_eq!(
            value.get("class").and_then(serde_json::Value::as_str),
            Some("steam_app_379430")
        );
        assert!(value.get("title").unwrap().is_null());
        assert_eq!(
            value.get("workspace").and_then(serde_json::Value::as_str),
            Some("gaming")
        );
    }

    #[test]
    fn foreground_event_serializes_expected_fields() {
        let event = ForegroundEvent {
            elapsed_ms: 1234,
            source: crate::foreground::ForegroundSource::Sway,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(4242),
            app_id: Some("steam".to_owned()),
            class: Some("Steam".to_owned()),
            title: None,
            window_id: Some("123".to_owned()),
            workspace: Some("games".to_owned()),
            confidence: 0.95,
            stale_ms: None,
            reason: "focused Sway node from swaymsg get_tree".to_owned(),
        };

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"elapsed_ms\":1234"));
        assert!(json.contains("\"source\":\"sway\""));
        assert!(json.contains("\"status\":\"available\""));
        assert!(json.contains("\"pid\":4242"));
        assert!(json.contains("\"app_id\":\"steam\""));
        assert!(json.contains("\"class\":\"Steam\""));
        assert!(json.contains("\"title\":null"));
        assert!(json.contains("\"window_id\":\"123\""));
        assert!(json.contains("\"workspace\":\"games\""));
        assert!(json.contains("\"confidence\":0.95"));
    }

    #[test]
    fn foreground_event_serializes_stale_ms() {
        let event = ForegroundEvent {
            elapsed_ms: 2_000,
            source: crate::foreground::ForegroundSource::Sway,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(4242),
            app_id: Some("steam".to_owned()),
            class: Some("Steam".to_owned()),
            title: None,
            window_id: Some("42".to_owned()),
            workspace: Some("games".to_owned()),
            confidence: 0.50,
            stale_ms: Some(750),
            reason: "using stale foreground snapshot from 750ms ago".to_owned(),
        };

        let value = serde_json::to_value(&event).unwrap();

        assert_eq!(
            value.get("stale_ms").and_then(serde_json::Value::as_u64),
            Some(750)
        );
    }

    #[test]
    fn foreground_event_deserializes_old_events_without_stale_ms() {
        let value = serde_json::json!({
            "elapsed_ms": 1234,
            "source": "sway",
            "status": "available",
            "pid": 4242,
            "app_id": "steam",
            "class": "Steam",
            "title": null,
            "window_id": "42",
            "workspace": "games",
            "confidence": 0.95,
            "reason": "old foreground event"
        });

        let event: ForegroundEvent = serde_json::from_value(value).unwrap();

        assert_eq!(event.stale_ms, None);
    }

    #[test]
    fn recorded_config_defaults_foreground_fields_for_old_sessions() {
        let config = RecordedConfig::default();

        assert!(!config.foreground_window);
        assert_eq!(config.foreground_source, "");
        assert_eq!(config.foreground_poll_ms, 0);
        assert_eq!(config.foreground_max_stale_ms, 0);
        assert!(!config.foreground_include_title);
    }

    #[test]
    fn recorded_config_defaults_auto_focus_fields_for_old_sessions() {
        let config = RecordedConfig::default();

        assert!(!config.auto_focus);
        assert!(!config.foreground_window);
        assert_eq!(config.focus_source, "");
        assert_eq!(config.foreground_source, "");
        assert_eq!(config.foreground_poll_ms, 0);
        assert_eq!(config.foreground_max_stale_ms, 0);
        assert!(!config.foreground_include_title);
        assert_eq!(config.auto_focus_poll_ms, 0);
        assert_eq!(config.auto_focus_min_confidence, 0.0);
        assert_eq!(config.auto_focus_switch_cooldown_ms, 0);
        assert_eq!(config.auto_focus_switch_margin, 0.0);
        assert_eq!(config.auto_focus_required_polls, 0);
        assert_eq!(config.auto_focus_max_roots, 0);
    }

    #[test]
    fn session_metadata_defaults_focus_fields_for_old_sessions() {
        let core = SessionMetadataCore::default();

        assert_eq!(core.focus_mode, None);
        assert_eq!(core.final_focus_kind, None);
        assert_eq!(core.focus_switch_count, 0);
        assert_eq!(core.focus_event_count, 0);
        assert_eq!(core.foreground_event_count, 0);
        assert_eq!(core.foreground_source, None);
        assert_eq!(core.final_foreground_pid, None);
        assert_eq!(core.final_foreground_app_id, None);
        assert_eq!(core.final_foreground_class, None);
        assert_eq!(core.final_foreground_status, None);
        assert_eq!(core.final_foreground_window_id, None);
        assert_eq!(core.final_foreground_workspace, None);
        assert_eq!(core.final_foreground_confidence, None);
        assert_eq!(core.final_foreground_stale_ms, None);
        assert_eq!(core.final_foreground_reason, None);
    }

    #[test]
    fn session_artifact_serializes_block_io_correlation_basis() {
        let session = SessionFile {
            core: SessionMetadataCore {
                block_io_correlation_basis:
                    crate::ebpf_loader::BlockIoCorrelationBasis::RequestPointer
                        .as_str()
                        .to_owned(),
                block_io_correlation_confidence:
                    crate::ebpf_loader::BlockIoCorrelationBasis::RequestPointer
                        .confidence()
                        .to_owned(),
                ..SessionMetadataCore::default()
            },
            ..SessionFile::default()
        };

        let value = serde_json::to_value(&session.core).unwrap();

        assert_eq!(
            value
                .get("block_io_correlation_basis")
                .and_then(serde_json::Value::as_str),
            Some("request-pointer")
        );
        assert_eq!(
            value
                .get("block_io_correlation_confidence")
                .and_then(serde_json::Value::as_str),
            Some("high")
        );
    }

    #[test]
    fn session_metadata_defaults_block_io_correlation_basis_for_old_sessions() {
        let json = serde_json::json!({
            "schema_version": 0,
            "run_name": null,
            "started_at": RecordedTime::default(),
            "ended_at": RecordedTime::default(),
            "monotonic_start_ns": null,
            "monotonic_end_ns": null,
            "duration_ms": 0,
            "metadata": SystemMetadata::default(),
            "target_pids_max": 0,
            "active_target_pids_count": 0,
            "active_expanded_tasks": [],
            "stop_reason": "",
            "config": RecordedConfig::default(),
            "tasks": [],
            "top_spikes": []
        });

        let session: SessionFile = serde_json::from_value(json).unwrap();

        assert_eq!(session.core.block_io_correlation_basis, "dev+sector");
    }
}
