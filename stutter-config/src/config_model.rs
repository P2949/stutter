//! Canonical monitor configuration model.
//!
//! All pure configuration structs live here. Runtime defaults that depend on
//! OS probing, eBPF loading, or foreground provider detection remain in the
//! main `stutter` crate.
//!
//! Types exported via `stutter-config` so that any workspace crate (config
//! parsing, API layer, autotune, etc.) can use them without depending on the
//! full `stutter` binary.

use std::{path::PathBuf, time::Duration};

use crate::types::{CsvStreamTarget, FocusSource, ForegroundSource, WaylandPresentationSource};

// ── Foreground defaults ────────────────────────────────────────────────────

/// Default interval between foreground-window polls.
pub const DEFAULT_FOREGROUND_POLL_MS: u64 = 1_000;

// ── Monitor config aggregate ───────────────────────────────────────────────

/// Top-level monitor configuration assembled from defaults and per-source
/// layers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonitorConfig {
    pub target: TargetConfig,
    pub timing: TimingConfig,
    pub diagnosis: DiagnosisConfig,
    pub probes: ProbeConfig,
    pub recording: RecordingConfig,
    pub outputs: OutputConfig,
    pub focus: FocusConfig,
    pub safety: SafetyConfig,
    pub watch: WatchConfig,
    pub alerts: AlertConfig,
    pub streams: StreamConfig,
    pub hwmon: HwmonConfig,
    pub mangohud: MangoHudConfig,
    pub cpu_perf: CpuPerfConfig,
    pub runtime_slices: RuntimeSlicesConfig,
    pub kms_timing: KmsTimingConfig,
    pub drm_fence: DrmFenceConfig,
    pub wayland_presentation: WaylandPresentationConfig,
    pub dmabuf: DmaBufConfig,
    pub display_path: DisplayPathConfig,
    pub ebpf_sizing: EbpfSizingConfig,
    pub ui: UiConfig,
    pub remote: RemoteConfig,
}

impl MonitorConfig {
    /// Returns true if `streams.csv` targets stdout.
    pub fn csv_streams_to_stdout(&self) -> bool {
        matches!(self.streams.csv, Some(CsvStreamTarget::Stdout))
    }

    /// Returns true if the user specified at least one explicit target
    /// (pid, tree-pid, watch-process, or cgroup path).
    pub fn has_explicit_target(&self) -> bool {
        !self.target.target_pids.is_empty()
            || !self.target.tree_pids.is_empty()
            || self.target.watch_process.is_some()
            || self.target.cgroupv2.is_some()
    }

    /// Returns true if auto-focus is enabled and no explicit target is
    /// specified.
    pub fn auto_focus_enabled(&self) -> bool {
        self.focus.auto_focus && !self.has_explicit_target()
    }
}

// ── Sub-configuration structs ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct TargetConfig {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub cgroupv2: Option<PathBuf>,
    pub exclude_tree_pids: Vec<u32>,
    pub include_comm: Vec<String>,
    pub exclude_comm: Vec<String>,
    pub watch_process: Option<String>,
    pub persistent: bool,
    pub keep_missing_pid: bool,
    pub max_tasks: usize,
}

impl Default for TargetConfig {
    fn default() -> Self {
        Self {
            target_pids: Vec::new(),
            tree_pids: Vec::new(),
            cgroupv2: None,
            exclude_tree_pids: Vec::new(),
            include_comm: Vec::new(),
            exclude_comm: Vec::new(),
            watch_process: None,
            persistent: false,
            keep_missing_pid: false,
            max_tasks: crate::types::TARGET_PIDS_MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimingConfig {
    pub summary_period_ms: u64,
    pub epoch_period_ms: Option<u64>,
    pub max_duration: Option<Duration>,
    pub spike_threshold_ns: u64,
}

impl Default for TimingConfig {
    fn default() -> Self {
        Self {
            summary_period_ms: 1_000,
            epoch_period_ms: None,
            max_duration: None,
            spike_threshold_ns: 1_000_000,
        }
    }
}

pub const DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS: u64 = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosisConfig {
    pub live_cluster_window_ms: u64,
}

impl Default for DiagnosisConfig {
    fn default() -> Self {
        Self {
            live_cluster_window_ms: DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProbeConfig {
    pub irq_latency: bool,
    pub irqs: Vec<u32>,
    pub hwmon: bool,
    pub cpu_freq: bool,
    pub faults: bool,
    pub cpu_perf: bool,
    pub block_io: bool,
    pub stat_wait: bool,
    pub runtime_slices: bool,
    pub kms_timing: bool,
    pub drm_fence_latency: bool,
    pub wayland_presentation: bool,
    pub dmabuf_tracking: bool,
    pub gpu_engine_sampling: bool,
    pub display_topology: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct KmsTimingConfig {
    pub drm_card: Option<String>,
    pub connector: Option<String>,
    pub crtc: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DrmFenceConfig {
    pub render_card: Option<String>,
    pub display_card: Option<String>,
    pub driver_filter: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaylandPresentationConfig {
    pub log_path: Option<PathBuf>,
    pub source: WaylandPresentationSource,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DmaBufConfig {
    pub log_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayPathConfig {
    pub label: Option<String>,
    pub render_gpu: Option<String>,
    pub scanout_gpu: Option<String>,
    pub connector: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecordingConfig {
    pub run_name: Option<String>,
    pub scenario_name: Option<String>,
    pub scenario_hash: Option<String>,
    pub workload_label: Option<String>,
    pub route_label: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub retain_intervals: Option<usize>,
    pub retention: RecordingRetentionConfig,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecordingRetentionConfig {
    pub max_run_count: Option<usize>,
    pub max_total_bytes: Option<u64>,
    pub max_age_seconds: Option<u64>,
    pub min_free_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputConfig {
    pub json_stream: bool,
    pub metrics_port: Option<u16>,
    pub otlp_endpoint: Option<String>,
    pub otel_service_name: String,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            json_stream: false,
            metrics_port: None,
            otlp_endpoint: None,
            otel_service_name: "stutter".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FocusConfig {
    pub auto_focus: bool,
    pub focus_source: FocusSource,
    pub foreground_window: bool,
    pub foreground_source: ForegroundSource,
    pub foreground_poll_ms: u64,
    pub foreground_max_stale_ms: u64,
    pub foreground_include_title: bool,
    pub auto_focus_poll_ms: u64,
    pub auto_focus_min_confidence: f32,
    pub auto_focus_switch_cooldown_ms: u64,
    pub auto_focus_switch_margin: f32,
    pub auto_focus_required_polls: u32,
    pub auto_focus_max_roots: usize,
}

impl Default for FocusConfig {
    fn default() -> Self {
        Self {
            auto_focus: false,
            focus_source: FocusSource::Heuristic,
            foreground_window: false,
            foreground_source: ForegroundSource::Auto,
            foreground_poll_ms: DEFAULT_FOREGROUND_POLL_MS,
            foreground_max_stale_ms: 2_500,
            foreground_include_title: false,
            auto_focus_poll_ms: 1_000,
            auto_focus_min_confidence: 0.60,
            auto_focus_switch_cooldown_ms: 5_000,
            auto_focus_switch_margin: 0.20,
            auto_focus_required_polls: 2,
            auto_focus_max_roots: 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WatchConfig {
    pub poll_ms: u64,
    pub timeout: Option<Duration>,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            poll_ms: 2_000,
            timeout: None,
        }
    }
}

pub const DEFAULT_MANGOHUD_TAIL_IDLE_SLEEP_MS: u64 = 75;
pub const DEFAULT_MANGOHUD_ALIGNMENT_POLL_MS: u64 = 500;
pub const DEFAULT_DESKTOP_ALERT_TIMEOUT_MS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq)]
pub struct AlertConfig {
    pub threshold_ns: Option<u64>,
    pub webhook_url: Option<String>,
    pub desktop_timeout_ms: u64,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            threshold_ns: None,
            webhook_url: None,
            desktop_timeout_ms: DEFAULT_DESKTOP_ALERT_TIMEOUT_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StreamConfig {
    pub csv: Option<CsvStreamTarget>,
    pub json_stream: bool,
    pub verbose: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HwmonConfig {
    pub enabled: bool,
    pub root: Option<PathBuf>,
    pub drm_card: Option<String>,
    pub render_node: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MangoHudConfig {
    pub log: Option<PathBuf>,
    pub log_live: bool,
    pub tail_idle_sleep_ms: u64,
    pub alignment_poll_ms: u64,
}

impl Default for MangoHudConfig {
    fn default() -> Self {
        Self {
            log: None,
            log_live: false,
            tail_idle_sleep_ms: DEFAULT_MANGOHUD_TAIL_IDLE_SLEEP_MS,
            alignment_poll_ms: DEFAULT_MANGOHUD_ALIGNMENT_POLL_MS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CpuPerfConfig {
    pub enabled: bool,
    pub include_kernel: bool,
    pub max_tasks: usize,
    pub collect_cache_refs: bool,
}

impl Default for CpuPerfConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            include_kernel: false,
            max_tasks: 128,
            collect_cache_refs: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeSlicesConfig {
    pub enabled: bool,
    pub max_tasks: usize,
}

impl Default for RuntimeSlicesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_tasks: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct EbpfSizingConfig {
    pub ringbuf_size_kb: Option<u32>,
    pub wakeup_map_factor: Option<u32>,
    pub target_pids_entries: Option<u32>,
    pub target_cgroup_ids_entries: Option<u32>,
    pub target_irqs_entries: Option<u32>,
    pub runnable_task_cpu_factor: Option<u32>,
    pub prev_faults_factor: Option<u32>,
    pub irq_start_entries: Option<u32>,
    pub block_start_entries: Option<u32>,
    pub kms_flip_start_entries: Option<u32>,
    pub drm_fence_wait_start_entries: Option<u32>,
    pub drm_fence_signal_entries: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct UiConfig {
    pub tui: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RemoteConfig {
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SafetyConfig {
    pub follow_exec: bool,
    pub native_cgroup_filter: bool,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            follow_exec: true,
            native_cgroup_filter: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_DESKTOP_ALERT_TIMEOUT_MS, DEFAULT_FOREGROUND_POLL_MS,
        DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS, DEFAULT_MANGOHUD_ALIGNMENT_POLL_MS,
        DEFAULT_MANGOHUD_TAIL_IDLE_SLEEP_MS, MonitorConfig,
    };

    #[test]
    fn monitor_config_defaults_compile_and_are_sensible() {
        let config = MonitorConfig::default();
        assert_eq!(config.timing.summary_period_ms, 1_000);
        assert_eq!(config.timing.spike_threshold_ns, 1_000_000);
        assert_eq!(
            config.diagnosis.live_cluster_window_ms,
            DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS
        );
        assert_eq!(config.focus.foreground_poll_ms, DEFAULT_FOREGROUND_POLL_MS);
        assert_eq!(config.watch.poll_ms, 2_000);
        assert_eq!(
            config.alerts.desktop_timeout_ms,
            DEFAULT_DESKTOP_ALERT_TIMEOUT_MS
        );
        assert_eq!(
            config.mangohud.tail_idle_sleep_ms,
            DEFAULT_MANGOHUD_TAIL_IDLE_SLEEP_MS
        );
        assert_eq!(
            config.mangohud.alignment_poll_ms,
            DEFAULT_MANGOHUD_ALIGNMENT_POLL_MS
        );
        assert_eq!(config.target.max_tasks, crate::types::TARGET_PIDS_MAX);
        assert!(config.safety.follow_exec);
        assert_eq!(config.outputs.otel_service_name, "stutter");
        assert_eq!(config.cpu_perf.max_tasks, 128);
        assert_eq!(config.runtime_slices.max_tasks, 256);
    }

    #[test]
    fn monitor_config_csv_streams_to_stdout_helper() {
        use crate::types::CsvStreamTarget;
        let mut config = MonitorConfig::default();
        assert!(!config.csv_streams_to_stdout());

        config.streams.csv = Some(CsvStreamTarget::Stdout);
        assert!(config.csv_streams_to_stdout());

        config.streams.csv = Some(CsvStreamTarget::File("/tmp/out.csv".into()));
        assert!(!config.csv_streams_to_stdout());
    }

    #[test]
    fn monitor_config_has_explicit_target_helper() {
        let mut config = MonitorConfig::default();
        assert!(!config.has_explicit_target());

        config.target.target_pids = vec![1234];
        assert!(config.has_explicit_target());
    }

    #[test]
    fn monitor_config_auto_focus_enabled_helper() {
        let mut config = MonitorConfig::default();
        config.focus.auto_focus = true;
        assert!(config.auto_focus_enabled());

        config.target.watch_process = Some("game".to_owned());
        assert!(!config.auto_focus_enabled());
    }
}
