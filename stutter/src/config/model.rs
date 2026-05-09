#![allow(dead_code)]

use std::{path::PathBuf, time::Duration};

#[derive(Debug, Clone, Default)]
pub struct MonitorConfig {
    pub target: TargetConfig,
    pub timing: TimingConfig,
    pub probes: ProbeConfig,
    pub recording: RecordingConfig,
    pub outputs: OutputConfig,
    pub focus: FocusConfig,
    pub safety: SafetyConfig,
}

#[derive(Debug, Clone, Default)]
pub struct TargetConfig {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub cgroupv2: Option<PathBuf>,
    pub exclude_tree_pids: Vec<u32>,
    pub watch_process: Option<String>,
    pub persistent: bool,
    pub keep_missing_pid: bool,
    pub max_tasks: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TimingConfig {
    pub summary_period_ms: u64,
    pub epoch_period_ms: Option<u64>,
    pub max_duration: Option<Duration>,
}

#[derive(Debug, Clone, Default)]
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

#[derive(Debug, Clone, Default)]
pub struct RecordingConfig {
    pub run_name: Option<String>,
    pub output_dir: Option<PathBuf>,
    pub retain_intervals: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct OutputConfig {
    pub json_stream: bool,
    pub metrics_port: Option<u16>,
    pub otlp_endpoint: Option<String>,
    pub otel_service_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct FocusConfig {
    pub auto_focus: bool,
    pub foreground_window: bool,
    pub foreground_poll_ms: u64,
    pub foreground_max_stale_ms: u64,
    pub foreground_include_title: bool,
}

#[derive(Debug, Clone, Default)]
pub struct SafetyConfig {
    pub follow_exec: bool,
    pub native_cgroup_filter: bool,
}
