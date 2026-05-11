pub mod effective;
pub mod layer;
pub mod merge;
pub mod model;
pub mod source;
pub mod types;

use model::{
    AlertConfig, CpuPerfConfig, EbpfSizingConfig, FocusConfig, HwmonConfig, MangoHudConfig,
    MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, RemoteConfig, RuntimeSlicesConfig,
    SafetyConfig, StreamConfig, TargetConfig, TimingConfig, UiConfig, WatchConfig,
};
pub use types::{CsvStreamTarget, FocusSource, ForegroundSource, TARGET_PIDS_MAX};

use crate::cli::Config;

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
                focus_source: config.focus_source,
                foreground_window: config.foreground_window,
                foreground_source: config.foreground_source,
                foreground_poll_ms: config.foreground_poll_ms,
                foreground_max_stale_ms: config.foreground_max_stale_ms,
                foreground_include_title: config.foreground_include_title,
                auto_focus_poll_ms: config.auto_focus_poll_ms,
                auto_focus_min_confidence: config.auto_focus_min_confidence,
                auto_focus_switch_cooldown_ms: config.auto_focus_switch_cooldown_ms,
                auto_focus_switch_margin: config.auto_focus_switch_margin,
                auto_focus_required_polls: config.auto_focus_required_polls,
                auto_focus_max_roots: config.auto_focus_max_roots,
            },
            safety: SafetyConfig {
                follow_exec: config.follow_exec,
                native_cgroup_filter: config.native_cgroup_filter,
            },
            watch: WatchConfig {
                poll_ms: config.watch_poll_ms,
                timeout: config.watch_timeout,
            },
            alerts: AlertConfig {
                threshold_ns: config.alert_threshold_ns,
                webhook_url: config.alert_webhook_url.clone(),
            },
            streams: StreamConfig {
                csv: config.csv_stream.clone(),
                json_stream: config.json_stream,
                verbose: config.verbose,
            },
            hwmon: HwmonConfig {
                enabled: config.hwmon,
                root: config.hwmon_root.clone(),
                drm_card: config.hwmon_drm_card.clone(),
                render_node: config.hwmon_render_node.clone(),
            },
            mangohud: MangoHudConfig {
                log: config.mangohud_log.clone(),
                log_live: config.mangohud_log_live,
            },
            cpu_perf: CpuPerfConfig {
                enabled: config.cpu_perf,
                include_kernel: config.cpu_perf_kernel,
                max_tasks: config.cpu_perf_max_tasks,
                collect_cache_refs: config.cpu_perf_cache_refs,
            },
            runtime_slices: RuntimeSlicesConfig {
                enabled: config.runtime_slices,
                max_tasks: config.runtime_slices_max_tasks,
            },
            ebpf_sizing: EbpfSizingConfig {
                ringbuf_size_kb: config.ringbuf_size_kb,
                wakeup_map_factor: config.wakeup_map_factor,
            },
            ui: UiConfig { tui: config.tui },
            remote: RemoteConfig {
                endpoint: config.remote.clone(),
            },
        }
    }
}
