use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonitorConfig {
    pub target: TargetConfig,
    pub timing: TimingConfig,
    pub probes: ProbeConfig,
    pub recording: RecordingConfig,
    pub outputs: OutputConfig,
    pub focus: FocusConfig,
    pub safety: SafetyConfig,
}

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
            max_tasks: crate::cli::TARGET_PIDS_MAX,
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
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecordingConfig {
    pub run_name: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub retain_intervals: Option<usize>,
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
    pub focus_source: String,
    pub foreground_window: bool,
    pub foreground_source: String,
    pub foreground_poll_ms: u64,
    pub foreground_max_stale_ms: u64,
    pub foreground_include_title: bool,
    pub auto_focus_poll_ms: u64,
    pub auto_focus_min_confidence: String,
    pub auto_focus_switch_cooldown_ms: u64,
    pub auto_focus_switch_margin: String,
    pub auto_focus_required_polls: u32,
    pub auto_focus_max_roots: usize,
}

impl Default for FocusConfig {
    fn default() -> Self {
        Self {
            auto_focus: false,
            focus_source: "heuristic".to_owned(),
            foreground_window: false,
            foreground_source: "auto".to_owned(),
            foreground_poll_ms: 1_000,
            foreground_max_stale_ms: 2_500,
            foreground_include_title: false,
            auto_focus_poll_ms: 1_000,
            auto_focus_min_confidence: "0.60".to_owned(),
            auto_focus_switch_cooldown_ms: 5_000,
            auto_focus_switch_margin: "0.20".to_owned(),
            auto_focus_required_polls: 2,
            auto_focus_max_roots: 4,
        }
    }
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
