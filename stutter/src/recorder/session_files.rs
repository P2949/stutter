use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    config::{FocusSource, ForegroundSource, TARGET_PIDS_MAX},
    ebpf_loader::{DropCountersSnapshot, NativeCgroupFilterStatus},
    metadata::SystemMetadata,
    metrics::CpuPerfRecord,
    process_tree::TaskClass,
};

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct ArtifactSchemaVersion(u32);

impl ArtifactSchemaVersion {
    pub const fn new(version: u32) -> Self {
        Self(version)
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    pub fn is_older_than(self, current: Self) -> bool {
        self < current
    }

    pub fn is_newer_than(self, current: Self) -> bool {
        self > current
    }
}

impl std::fmt::Display for ArtifactSchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialEq<u32> for ArtifactSchemaVersion {
    fn eq(&self, other: &u32) -> bool {
        self.0 == *other
    }
}

impl PartialEq<ArtifactSchemaVersion> for u32 {
    fn eq(&self, other: &ArtifactSchemaVersion) -> bool {
        *self == other.0
    }
}

impl std::ops::Add<u32> for ArtifactSchemaVersion {
    type Output = Self;

    fn add(self, rhs: u32) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl std::ops::Sub<u32> for ArtifactSchemaVersion {
    type Output = u32;

    fn sub(self, rhs: u32) -> Self::Output {
        self.0 - rhs
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordedProbeActivationWarning {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub message: String,
}

impl From<&crate::probe_activation::ProbeActivationWarning> for RecordedProbeActivationWarning {
    fn from(warning: &crate::probe_activation::ProbeActivationWarning) -> Self {
        Self {
            key: warning.key.map(|key| {
                crate::probe_registry::probe_spec(key)
                    .catalog_key
                    .to_owned()
            }),
            message: warning.message.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionMetadataCore {
    pub schema_version: ArtifactSchemaVersion,
    pub run_name: Option<String>,
    #[serde(default)]
    pub scenario_name: Option<String>,
    #[serde(default)]
    pub scenario_hash: Option<String>,
    #[serde(default)]
    pub workload_label: Option<String>,
    #[serde(default)]
    pub route_label: Option<String>,
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
    pub dmabuf_event_count: u64,
    #[serde(default)]
    pub gpu_engine_sample_count: u64,
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
    #[serde(default, skip_serializing_if = "NativeCgroupFilterStatus::is_disabled")]
    pub native_cgroup_filter: NativeCgroupFilterStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probe_activation_warnings: Vec<RecordedProbeActivationWarning>,
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
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub render_gpu: Option<String>,
    #[serde(default)]
    pub scanout_gpu: Option<String>,
    #[serde(default)]
    pub connector: Option<String>,
    #[serde(default)]
    pub render_card: Option<String>,
    #[serde(default)]
    pub render_render_node: Option<String>,
    #[serde(default)]
    pub render_driver: Option<String>,
    #[serde(default)]
    pub scanout_card: Option<String>,
    #[serde(default)]
    pub scanout_driver: Option<String>,
    #[serde(default)]
    pub is_cross_gpu: Option<bool>,
    #[serde(default)]
    pub session_type: Option<String>,
    #[serde(default)]
    pub compositor: Option<String>,
    #[serde(default)]
    pub topology_confidence: Option<String>,
    #[serde(default)]
    pub topology_warnings: Vec<String>,
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
    pub scenario_name: Option<String>,
    #[serde(default)]
    pub scenario_hash: Option<String>,
    #[serde(default)]
    pub workload_label: Option<String>,
    #[serde(default)]
    pub route_label: Option<String>,
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
    pub live_diagnosis_cluster_window_ms: u64,
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
    pub dmabuf_tracking: bool,
    #[serde(default)]
    pub dmabuf_log: Option<PathBuf>,
    #[serde(default)]
    pub gpu_engine_sampling: bool,
    #[serde(default)]
    pub display_topology: bool,
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
        ForegroundSource::Gnome => "gnome",
        ForegroundSource::Kde => "kde",
        ForegroundSource::X11 => "x11",
    }
    .to_owned()
}

pub(crate) fn foreground_source_label(source: crate::foreground::ForegroundSource) -> String {
    match source {
        crate::foreground::ForegroundSource::Auto => "auto",
        crate::foreground::ForegroundSource::Sway => "sway",
        crate::foreground::ForegroundSource::Hyprland => "hyprland",
        crate::foreground::ForegroundSource::Gnome => "gnome",
        crate::foreground::ForegroundSource::Kde => "kde",
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
    pub process_comm: String,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub exe_dev: Option<u64>,
    #[serde(default)]
    pub exe_ino: Option<u64>,
    pub comm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_cpus: Option<String>,
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
    pub process_comm: String,
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
    pub process_comm: String,
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
#[path = "session_files_tests.rs"]
mod tests;
