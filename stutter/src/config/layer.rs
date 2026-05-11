use std::{path::PathBuf, time::Duration};

use crate::{
    cli::Config,
    config::{FocusSource, ForegroundSource, TARGET_PIDS_MAX, model::MonitorConfig},
    config_file::UserConfigFile,
    presets::PresetDefaults,
};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MonitorConfigLayer {
    pub target_pids: Option<Vec<u32>>,
    pub tree_pids: Option<Vec<u32>>,
    pub cgroupv2: Option<Option<PathBuf>>,
    pub exclude_tree_pids: Option<Vec<u32>>,
    pub include_comm: Option<Vec<String>>,
    pub exclude_comm: Option<Vec<String>>,
    pub watch_process: Option<Option<String>>,
    pub persistent: Option<bool>,
    pub keep_missing_pid: Option<bool>,
    pub max_tasks: Option<usize>,

    pub summary_period_ms: Option<u64>,
    pub epoch_period_ms: Option<Option<u64>>,
    pub max_duration: Option<Option<Duration>>,
    pub spike_threshold_ns: Option<u64>,

    pub irq_latency: Option<bool>,
    pub irqs: Option<Vec<u32>>,
    pub hwmon: Option<bool>,
    pub cpu_freq: Option<bool>,
    pub faults: Option<bool>,
    pub cpu_perf: Option<bool>,
    pub block_io: Option<bool>,
    pub stat_wait: Option<bool>,
    pub runtime_slices: Option<bool>,

    pub run_name: Option<Option<String>>,
    pub output_dir: Option<Option<PathBuf>>,
    pub retain_intervals: Option<Option<usize>>,

    pub json_stream: Option<bool>,
    pub metrics_port: Option<Option<u16>>,
    pub otlp_endpoint: Option<Option<String>>,
    pub otel_service_name: Option<String>,

    pub auto_focus: Option<bool>,
    pub focus_source: Option<FocusSource>,
    pub foreground_window: Option<bool>,
    pub foreground_source: Option<ForegroundSource>,
    pub foreground_poll_ms: Option<u64>,
    pub foreground_max_stale_ms: Option<u64>,
    pub foreground_include_title: Option<bool>,
    pub auto_focus_poll_ms: Option<u64>,
    pub auto_focus_min_confidence: Option<f32>,
    pub auto_focus_switch_cooldown_ms: Option<u64>,
    pub auto_focus_switch_margin: Option<f32>,
    pub auto_focus_required_polls: Option<u32>,
    pub auto_focus_max_roots: Option<usize>,

    pub follow_exec: Option<bool>,
    pub native_cgroup_filter: Option<bool>,
}

impl MonitorConfigLayer {
    pub fn from_monitor_config(config: MonitorConfig) -> Self {
        Self {
            target_pids: Some(config.target.target_pids),
            tree_pids: Some(config.target.tree_pids),
            cgroupv2: Some(config.target.cgroupv2),
            exclude_tree_pids: Some(config.target.exclude_tree_pids),
            include_comm: Some(config.target.include_comm),
            exclude_comm: Some(config.target.exclude_comm),
            watch_process: Some(config.target.watch_process),
            persistent: Some(config.target.persistent),
            keep_missing_pid: Some(config.target.keep_missing_pid),
            max_tasks: Some(config.target.max_tasks),

            summary_period_ms: Some(config.timing.summary_period_ms),
            epoch_period_ms: Some(config.timing.epoch_period_ms),
            max_duration: Some(config.timing.max_duration),
            spike_threshold_ns: Some(config.timing.spike_threshold_ns),

            irq_latency: Some(config.probes.irq_latency),
            irqs: Some(config.probes.irqs),
            hwmon: Some(config.probes.hwmon),
            cpu_freq: Some(config.probes.cpu_freq),
            faults: Some(config.probes.faults),
            cpu_perf: Some(config.probes.cpu_perf),
            block_io: Some(config.probes.block_io),
            stat_wait: Some(config.probes.stat_wait),
            runtime_slices: Some(config.probes.runtime_slices),

            run_name: Some(config.recording.run_name),
            output_dir: Some(config.recording.output_dir),
            retain_intervals: Some(config.recording.retain_intervals),

            json_stream: Some(config.outputs.json_stream),
            metrics_port: Some(config.outputs.metrics_port),
            otlp_endpoint: Some(config.outputs.otlp_endpoint),
            otel_service_name: Some(config.outputs.otel_service_name),

            auto_focus: Some(config.focus.auto_focus),
            focus_source: Some(config.focus.focus_source),
            foreground_window: Some(config.focus.foreground_window),
            foreground_source: Some(config.focus.foreground_source),
            foreground_poll_ms: Some(config.focus.foreground_poll_ms),
            foreground_max_stale_ms: Some(config.focus.foreground_max_stale_ms),
            foreground_include_title: Some(config.focus.foreground_include_title),
            auto_focus_poll_ms: Some(config.focus.auto_focus_poll_ms),
            auto_focus_min_confidence: Some(config.focus.auto_focus_min_confidence),
            auto_focus_switch_cooldown_ms: Some(config.focus.auto_focus_switch_cooldown_ms),
            auto_focus_switch_margin: Some(config.focus.auto_focus_switch_margin),
            auto_focus_required_polls: Some(config.focus.auto_focus_required_polls),
            auto_focus_max_roots: Some(config.focus.auto_focus_max_roots),

            follow_exec: Some(config.safety.follow_exec),
            native_cgroup_filter: Some(config.safety.native_cgroup_filter),
        }
    }

    pub fn from_user_file(user_file: &UserConfigFile) -> anyhow::Result<Self> {
        let focus_source = match user_file.focus_source.as_deref() {
            Some(value) => Some(crate::config_file::parse_focus_source_value(value)?),
            None => None,
        };

        let foreground_source = match user_file.foreground_source.as_deref() {
            Some(value) => Some(crate::config_file::parse_foreground_source_value(value)?),
            None => None,
        };

        Ok(Self {
            summary_period_ms: user_file.summary_ms,
            spike_threshold_ns: user_file.spike_us.map(|value| value.saturating_mul(1_000)),
            hwmon: user_file.hwmon,
            cpu_freq: match (user_file.cpu_freq, user_file.no_cpu_freq) {
                (_, Some(true)) => Some(false),
                (Some(value), _) => Some(value),
                _ => None,
            },
            include_comm: user_file.include_comm.clone(),
            exclude_comm: user_file.exclude_comm.clone(),
            max_tasks: user_file.max_tasks,
            retain_intervals: user_file.retain_intervals.map(Some),
            foreground_window: user_file.foreground_window,
            focus_source,
            foreground_source,
            foreground_poll_ms: user_file.foreground_poll_ms,
            foreground_max_stale_ms: user_file.foreground_max_stale_ms,
            foreground_include_title: user_file.foreground_include_title,
            ..Self::default()
        })
    }

    pub fn from_preset_defaults(defaults: PresetDefaults) -> Self {
        Self {
            hwmon: defaults.hwmon,
            cpu_freq: defaults.cpu_freq,
            faults: defaults.faults,
            stat_wait: defaults.stat_wait,
            block_io: defaults.block_io,
            runtime_slices: defaults.runtime_slices,
            irq_latency: defaults.irq_latency,
            ..Self::default()
        }
    }

    pub fn from_existing_cli_config(config: &Config) -> Self {
        Self {
            target_pids: (!config.target_pids.is_empty()).then(|| config.target_pids.clone()),
            tree_pids: (!config.tree_pids.is_empty()).then(|| config.tree_pids.clone()),
            cgroupv2: config.cgroupv2.clone().map(Some),
            exclude_tree_pids: (!config.exclude_tree_pids.is_empty())
                .then(|| config.exclude_tree_pids.clone()),
            include_comm: (!config.task_filters.include_comm.is_empty()).then(|| {
                config
                    .task_filters
                    .include_comm
                    .iter()
                    .map(|pattern| pattern.raw.clone())
                    .collect()
            }),
            exclude_comm: (!config.task_filters.exclude_comm.is_empty()).then(|| {
                config
                    .task_filters
                    .exclude_comm
                    .iter()
                    .map(|pattern| pattern.raw.clone())
                    .collect()
            }),
            watch_process: config.watch_process.clone().map(Some),
            persistent: config.persistent.then_some(true),
            keep_missing_pid: config.keep_missing_pid.then_some(true),
            max_tasks: (config.max_tasks != TARGET_PIDS_MAX).then_some(config.max_tasks),

            summary_period_ms: (config.summary_period_ms != 1_000)
                .then_some(config.summary_period_ms),
            epoch_period_ms: config.epoch_period_ms.map(Some),
            max_duration: config.max_duration.map(Some),
            spike_threshold_ns: (config.spike_threshold_ns != 1_000_000)
                .then_some(config.spike_threshold_ns),

            irq_latency: config.irq_latency.then_some(true),
            irqs: (!config.irqs.is_empty()).then(|| config.irqs.clone()),
            hwmon: config.hwmon.then_some(true),
            cpu_freq: config.cpu_freq.then_some(true),
            faults: config.faults.then_some(true),
            cpu_perf: config.cpu_perf.then_some(true),
            block_io: config.block_io.then_some(true),
            stat_wait: config.stat_wait.then_some(true),
            runtime_slices: config.runtime_slices.then_some(true),

            run_name: config
                .recording
                .as_ref()
                .and_then(|recording| recording.run_name.clone().map(Some)),
            output_dir: config
                .recording
                .as_ref()
                .and_then(|recording| recording.out_dir.clone().map(Some)),
            retain_intervals: config.retain_intervals.map(Some),

            json_stream: config.json_stream.then_some(true),
            metrics_port: config.metrics_port.map(Some),
            otlp_endpoint: config.otlp_endpoint.clone().map(Some),
            otel_service_name: (config.otel_service_name != "stutter")
                .then(|| config.otel_service_name.clone()),

            auto_focus: config.auto_focus.then_some(true),
            focus_source: (config.focus_source != FocusSource::Heuristic)
                .then_some(config.focus_source),
            foreground_window: config.foreground_window.then_some(true),
            foreground_source: (config.foreground_source != ForegroundSource::Auto)
                .then_some(config.foreground_source),
            foreground_poll_ms: (config.foreground_poll_ms != 1_000)
                .then_some(config.foreground_poll_ms),
            foreground_max_stale_ms: (config.foreground_max_stale_ms != 2_500)
                .then_some(config.foreground_max_stale_ms),
            foreground_include_title: config.foreground_include_title.then_some(true),
            auto_focus_poll_ms: (config.auto_focus_poll_ms != 1_000)
                .then_some(config.auto_focus_poll_ms),
            auto_focus_min_confidence: (config.auto_focus_min_confidence != 0.60)
                .then_some(config.auto_focus_min_confidence),
            auto_focus_switch_cooldown_ms: (config.auto_focus_switch_cooldown_ms != 5_000)
                .then_some(config.auto_focus_switch_cooldown_ms),
            auto_focus_switch_margin: (config.auto_focus_switch_margin != 0.20)
                .then_some(config.auto_focus_switch_margin),
            auto_focus_required_polls: (config.auto_focus_required_polls != 2)
                .then_some(config.auto_focus_required_polls),
            auto_focus_max_roots: (config.auto_focus_max_roots != 4)
                .then_some(config.auto_focus_max_roots),

            follow_exec: (!config.follow_exec).then_some(false),
            native_cgroup_filter: config.native_cgroup_filter.then_some(true),
        }
    }
}
