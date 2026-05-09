pub mod merge;
pub mod model;
pub mod recorded;

use model::{
    FocusConfig, MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, SafetyConfig,
    TargetConfig, TimingConfig,
};

use crate::cli::{Config, FocusSource, ForegroundSourceArg};

impl From<&Config> for MonitorConfig {
    fn from(config: &Config) -> Self {
        Self {
            target: TargetConfig {
                target_pids: config.target_pids.clone(),
                tree_pids: config.tree_pids.clone(),
                cgroupv2: config.cgroupv2.clone(),
                exclude_tree_pids: config.exclude_tree_pids.clone(),
                include_comm: config
                    .task_filters
                    .include_comm
                    .iter()
                    .map(|p| p.raw.clone())
                    .collect(),
                exclude_comm: config
                    .task_filters
                    .exclude_comm
                    .iter()
                    .map(|p| p.raw.clone())
                    .collect(),
                watch_process: config.watch_process.clone(),
                persistent: config.persistent,
                keep_missing_pid: config.keep_missing_pid,
                max_tasks: config.max_tasks,
            },
            timing: TimingConfig {
                summary_period_ms: config.summary_period_ms,
                epoch_period_ms: config.epoch_period_ms,
                max_duration: config.max_duration,
                spike_threshold_ns: config.spike_threshold_ns,
            },
            probes: ProbeConfig {
                irq_latency: config.irq_latency,
                irqs: config.irqs.clone(),
                hwmon: config.hwmon,
                cpu_freq: config.cpu_freq,
                faults: config.faults,
                cpu_perf: config.cpu_perf,
                block_io: config.block_io,
                stat_wait: config.stat_wait,
                runtime_slices: config.runtime_slices,
            },
            recording: RecordingConfig {
                run_name: config.recording.as_ref().and_then(|r| r.run_name.clone()),
                output_dir: config.recording.as_ref().and_then(|r| r.out_dir.clone()),
                retain_intervals: config.retain_intervals,
            },
            outputs: OutputConfig {
                json_stream: config.json_stream,
                metrics_port: config.metrics_port,
                otlp_endpoint: config.otlp_endpoint.clone(),
                otel_service_name: config.otel_service_name.clone(),
            },
            focus: FocusConfig {
                auto_focus: config.auto_focus,
                focus_source: focus_source_label(config.focus_source),
                foreground_window: config.foreground_window,
                foreground_source: foreground_source_label(config.foreground_source),
                foreground_poll_ms: config.foreground_poll_ms,
                foreground_max_stale_ms: config.foreground_max_stale_ms,
                foreground_include_title: config.foreground_include_title,
                auto_focus_poll_ms: config.auto_focus_poll_ms,
                auto_focus_min_confidence: format!("{:.2}", config.auto_focus_min_confidence),
                auto_focus_switch_cooldown_ms: config.auto_focus_switch_cooldown_ms,
                auto_focus_switch_margin: format!("{:.2}", config.auto_focus_switch_margin),
                auto_focus_required_polls: config.auto_focus_required_polls,
                auto_focus_max_roots: config.auto_focus_max_roots,
            },
            safety: SafetyConfig {
                follow_exec: config.follow_exec,
                native_cgroup_filter: config.native_cgroup_filter,
            },
        }
    }
}

fn focus_source_label(source: FocusSource) -> String {
    match source {
        FocusSource::Heuristic => "heuristic",
        FocusSource::Foreground => "foreground",
        FocusSource::Hybrid => "hybrid",
    }
    .to_owned()
}

fn foreground_source_label(source: ForegroundSourceArg) -> String {
    match source {
        ForegroundSourceArg::Auto => "auto",
        ForegroundSourceArg::Sway => "sway",
        ForegroundSourceArg::Hyprland => "hyprland",
        ForegroundSourceArg::X11 => "x11",
    }
    .to_owned()
}
