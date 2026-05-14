use std::{path::PathBuf, time::Duration};

use crate::{
    config::{CsvStreamTarget, FocusSource, ForegroundSource, model::MonitorConfig},
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
    pub retention_max_run_count: Option<Option<usize>>,
    pub retention_max_total_bytes: Option<Option<u64>>,
    pub retention_max_age_seconds: Option<Option<u64>>,
    pub retention_min_free_bytes: Option<Option<u64>>,

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

    pub watch_poll_ms: Option<u64>,
    pub watch_timeout: Option<Option<Duration>>,

    pub alert_threshold_ns: Option<Option<u64>>,
    pub alert_webhook_url: Option<Option<String>>,

    pub csv_stream: Option<Option<CsvStreamTarget>>,
    pub verbose: Option<bool>,

    pub hwmon_root: Option<Option<PathBuf>>,
    pub hwmon_drm_card: Option<Option<String>>,
    pub hwmon_render_node: Option<Option<PathBuf>>,

    pub mangohud_log: Option<Option<PathBuf>>,
    pub mangohud_log_live: Option<bool>,

    pub tui: Option<bool>,

    pub cpu_perf_kernel: Option<bool>,
    pub cpu_perf_max_tasks: Option<usize>,
    pub cpu_perf_cache_refs: Option<bool>,

    pub runtime_slices_max_tasks: Option<usize>,

    pub ringbuf_size_kb: Option<Option<u32>>,
    pub wakeup_map_factor: Option<Option<u32>>,

    pub remote: Option<Option<String>>,
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
            retention_max_run_count: Some(config.recording.retention.max_run_count),
            retention_max_total_bytes: Some(config.recording.retention.max_total_bytes),
            retention_max_age_seconds: Some(config.recording.retention.max_age_seconds),
            retention_min_free_bytes: Some(config.recording.retention.min_free_bytes),

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

            watch_poll_ms: Some(config.watch.poll_ms),
            watch_timeout: Some(config.watch.timeout),

            alert_threshold_ns: Some(config.alerts.threshold_ns),
            alert_webhook_url: Some(config.alerts.webhook_url),

            csv_stream: Some(config.streams.csv),
            verbose: Some(config.streams.verbose),

            hwmon_root: Some(config.hwmon.root),
            hwmon_drm_card: Some(config.hwmon.drm_card),
            hwmon_render_node: Some(config.hwmon.render_node),

            mangohud_log: Some(config.mangohud.log),
            mangohud_log_live: Some(config.mangohud.log_live),

            tui: Some(config.ui.tui),

            cpu_perf_kernel: Some(config.cpu_perf.include_kernel),
            cpu_perf_max_tasks: Some(config.cpu_perf.max_tasks),
            cpu_perf_cache_refs: Some(config.cpu_perf.collect_cache_refs),

            runtime_slices_max_tasks: Some(config.runtime_slices.max_tasks),

            ringbuf_size_kb: Some(config.ebpf_sizing.ringbuf_size_kb),
            wakeup_map_factor: Some(config.ebpf_sizing.wakeup_map_factor),

            remote: Some(config.remote.endpoint),
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
            summary_period_ms: user_file.summary_period_ms.or(user_file.summary_ms),
            spike_threshold_ns: user_file
                .spike_threshold_ns
                .or_else(|| user_file.spike_us.map(|value| value.saturating_mul(1_000))),
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
            retention_max_run_count: user_file.retention_max_run_count.map(Some),
            retention_max_total_bytes: user_file.retention_max_total_bytes.map(Some),
            retention_max_age_seconds: user_file.retention_max_age_seconds.map(Some),
            retention_min_free_bytes: user_file.retention_min_free_bytes.map(Some),
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
}
