use std::time::Duration;

use clap::ArgMatches;

use super::*;
use crate::config::TARGET_PIDS_MAX;

#[derive(Args, Debug, Clone)]
pub(super) struct MonitorArgs {
    #[arg(long = "pid", short = 'p', value_name = "PID")]
    pub(super) target_pids: Vec<u32>,

    #[arg(long = "tree-pid", value_name = "PID")]
    pub(super) tree_pids: Vec<u32>,

    #[arg(long = "exclude-tree-pid", value_name = "PID")]
    pub(super) exclude_tree_pids: Vec<u32>,

    #[arg(long = "summary-ms", value_name = "MS")]
    pub(super) summary_period_ms: Option<u64>,

    #[arg(long = "epoch", value_name = "MS")]
    pub(super) epoch_period_ms: Option<u64>,

    #[arg(long = "spike-us", value_name = "US")]
    pub(super) spike_threshold_us: Option<u64>,

    #[arg(long = "live-diagnosis-cluster-window-ms", value_name = "MS")]
    pub(super) live_diagnosis_cluster_window_ms: Option<u64>,

    #[arg(long = "alert-threshold-ms", value_name = "MS")]
    pub(super) alert_threshold_ms: Option<u64>,

    #[arg(long = "alert-webhook-url", value_name = "URL")]
    pub(super) alert_webhook_url: Option<String>,

    #[arg(long, short = 'v')]
    pub(super) verbose: bool,

    #[arg(long = "run-name", value_name = "NAME")]
    pub(super) run_name: Option<String>,

    #[arg(long = "out-dir", alias = "out", value_name = "PATH")]
    pub(super) out_dir: Option<PathBuf>,

    #[arg(long = "include-comm", value_name = "PATTERN")]
    pub(super) include_comm: Vec<String>,

    #[arg(long = "exclude-comm", value_name = "PATTERN")]
    pub(super) exclude_comm: Vec<String>,

    #[arg(long = "keep-missing-pid")]
    pub(super) keep_missing_pid: bool,

    #[arg(long = "watch-process", value_name = "COMM")]
    pub(super) watch_process: Option<String>,

    #[arg(long)]
    pub(super) persistent: bool,

    #[arg(long = "watch-poll-ms", default_value_t = 2_000)]
    pub(super) watch_poll_ms: u64,

    #[arg(long = "watch-timeout-seconds", value_name = "SECONDS")]
    pub(super) watch_timeout_seconds: Option<u64>,

    #[arg(long, value_name = "N")]
    pub(super) max_tasks: Option<usize>,

    #[arg(long = "csv", value_name = "PATH")]
    pub(super) csv_path: Option<PathBuf>,

    #[arg(
        long = "stream-csv",
        value_name = "PATH_OR_-",
        conflicts_with = "csv_path"
    )]
    pub(super) stream_csv: Option<String>,

    #[arg(long = "irq-latency")]
    pub(super) irq_latency: bool,

    #[arg(long = "irq", value_name = "IRQ")]
    pub(super) irqs: Vec<u32>,

    #[arg(long = "hwmon", id = "hwmon", conflicts_with = "no_hwmon")]
    pub(super) hwmon: bool,

    #[arg(long = "no-hwmon", help = "Disable GPU hwmon telemetry")]
    pub(super) no_hwmon: bool,

    #[arg(long = "hwmon-root", value_name = "PATH", requires = "hwmon")]
    pub(super) hwmon_root: Option<PathBuf>,

    #[arg(long = "hwmon-drm-card", value_name = "CARD", requires = "hwmon")]
    pub(super) hwmon_drm_card: Option<String>,

    #[arg(long = "hwmon-render-node", value_name = "NODE", requires = "hwmon")]
    pub(super) hwmon_render_node: Option<PathBuf>,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub(super) mangohud_log: Option<PathBuf>,

    #[arg(long = "mangohud-log-live", requires = "mangohud_log")]
    pub(super) mangohud_log_live: bool,

    #[arg(long = "tui")]
    pub(super) tui: bool,

    #[arg(long = "retain-intervals", value_name = "N")]
    pub(super) retain_intervals: Option<usize>,

    #[arg(long = "retention-max-runs", value_name = "N")]
    pub(super) retention_max_run_count: Option<usize>,

    #[arg(long = "retention-max-bytes", value_name = "BYTES")]
    pub(super) retention_max_total_bytes: Option<u64>,

    #[arg(long = "retention-max-age-seconds", value_name = "SECONDS")]
    pub(super) retention_max_age_seconds: Option<u64>,

    #[arg(long = "retention-min-free-bytes", value_name = "BYTES")]
    pub(super) retention_min_free_bytes: Option<u64>,

    #[arg(long = "no-record")]
    pub(super) no_record: bool,

    #[arg(
        long = "cpu-freq",
        help = "Collect CPU frequency information (enabled by default for recording runs)",
        conflicts_with = "no_cpu_freq"
    )]
    pub(super) cpu_freq: bool,

    #[arg(long = "no-cpu-freq", help = "Disable CPU frequency collection")]
    pub(super) no_cpu_freq: bool,

    #[arg(long = "cgroupv2", value_name = "PATH")]
    pub(super) cgroupv2: Option<PathBuf>,

    #[arg(long = "native-cgroup-filter", requires = "cgroupv2")]
    pub(super) native_cgroup_filter: bool,

    #[arg(
        long = "follow-exec",
        default_value_t = true,
        action = ArgAction::SetTrue,
        conflicts_with = "no_follow_exec"
    )]
    pub(super) follow_exec: bool,

    #[arg(long = "no-follow-exec", action = ArgAction::SetTrue)]
    pub(super) no_follow_exec: bool,

    #[arg(long = "faults", conflicts_with = "no_faults")]
    pub(super) faults: bool,

    #[arg(long = "no-faults", help = "Disable page fault collection")]
    pub(super) no_faults: bool,

    #[arg(
        long = "cpu-perf",
        help = "Collect per-task CPU hardware counters for IPC/cache-miss diagnostics"
    )]
    pub(super) cpu_perf: bool,

    #[arg(
        long = "cpu-perf-kernel",
        help = "Include kernel/hypervisor time in CPU perf counters; default is user-space only"
    )]
    pub(super) cpu_perf_kernel: bool,

    #[arg(
        long = "cpu-perf-max-tasks",
        default_value_t = 128,
        value_name = "N",
        help = "Maximum active target tasks to attach CPU perf counters to"
    )]
    pub(super) cpu_perf_max_tasks: usize,

    #[arg(
        long = "cpu-perf-cache-refs",
        help = "Also collect cache references so cache miss rate can be computed; otherwise only cache MPKI is computed"
    )]
    pub(super) cpu_perf_cache_refs: bool,

    #[arg(long = "block-io", conflicts_with = "no_block_io")]
    pub(super) block_io: bool,

    #[arg(long = "no-block-io", help = "Disable block I/O collection")]
    pub(super) no_block_io: bool,

    #[arg(long = "stat-wait", conflicts_with = "no_stat_wait")]
    pub(super) stat_wait: bool,

    #[arg(long = "no-stat-wait", help = "Disable stat-wait collection")]
    pub(super) no_stat_wait: bool,

    #[arg(
        long = "runtime-slices",
        conflicts_with = "no_runtime_slices",
        help = "Collect per-thread CPU runtime/wait slices from procfs schedstat"
    )]
    pub(super) runtime_slices: bool,

    #[arg(
        long = "no-runtime-slices",
        help = "Disable per-thread runtime-slice collection"
    )]
    pub(super) no_runtime_slices: bool,

    #[arg(
        long = "runtime-slices-max-tasks",
        default_value_t = 256,
        value_name = "N"
    )]
    pub(super) runtime_slices_max_tasks: usize,

    #[arg(long = "kms-timing")]
    pub(super) kms_timing: bool,

    #[arg(long = "kms-card", value_name = "cardN")]
    pub(super) kms_card: Option<String>,

    #[arg(long = "kms-connector", value_name = "NAME")]
    pub(super) kms_connector: Option<String>,

    #[arg(long = "kms-crtc", value_name = "ID")]
    pub(super) kms_crtc: Option<u32>,

    #[arg(long = "drm-fence-latency")]
    pub(super) drm_fence_latency: bool,

    #[arg(long = "drm-fence-render-card", value_name = "cardN")]
    pub(super) drm_fence_render_card: Option<String>,

    #[arg(long = "drm-fence-display-card", value_name = "cardN")]
    pub(super) drm_fence_display_card: Option<String>,

    #[arg(long = "drm-fence-driver", value_name = "amdgpu|i915|auto")]
    pub(super) drm_fence_driver: Option<String>,

    #[arg(long = "wayland-presentation")]
    pub(super) wayland_presentation: bool,

    #[arg(long = "wayland-presentation-log", value_name = "PATH")]
    pub(super) wayland_presentation_log: Option<PathBuf>,

    #[arg(
        long = "wayland-presentation-source",
        value_enum,
        default_value_t = WaylandPresentationSource::ExternalLog
    )]
    pub(super) wayland_presentation_source: WaylandPresentationSource,

    #[arg(
        long = "dmabuf-tracking",
        help = "Ingest cooperative DMABUF format/modifier path events"
    )]
    pub(super) dmabuf_tracking: bool,

    #[arg(long = "dmabuf-log", value_name = "PATH")]
    pub(super) dmabuf_log: Option<PathBuf>,

    #[arg(
        long = "gpu-engine-sampling",
        help = "Collect per-GPU engine activity samples for display-path diagnosis"
    )]
    pub(super) gpu_engine_sampling: bool,

    #[arg(long = "display-path-label", value_name = "LABEL")]
    pub(super) display_path_label: Option<String>,

    #[arg(long = "display-render-gpu", value_name = "DRIVER")]
    pub(super) display_render_gpu: Option<String>,

    #[arg(long = "display-scanout-gpu", value_name = "DRIVER")]
    pub(super) display_scanout_gpu: Option<String>,

    #[arg(long = "display-connector", value_name = "NAME")]
    pub(super) display_connector: Option<String>,

    #[arg(
        long = "json-stream",
        help = "Emit scheduler spike events to stdout as newline-delimited JSON"
    )]
    pub(super) json_stream: bool,

    #[arg(long = "metrics-port", value_name = "PORT")]
    pub(super) metrics_port: Option<u16>,

    #[arg(
        long = "preset",
        value_name = "NAME",
        help = "Apply named monitor defaults: gaming, recording, diagnosis, lightweight"
    )]
    pub(super) preset: Option<String>,

    #[arg(
        long = "ebpf-ringbuf-size-kb",
        alias = "ringbuf-size-kb",
        value_name = "KB"
    )]
    pub(super) ringbuf_size_kb: Option<u32>,

    #[arg(
        long = "ebpf-wakeup-map-factor",
        alias = "wakeup-map-factor",
        value_name = "N"
    )]
    pub(super) wakeup_map_factor: Option<u32>,

    #[arg(long = "ebpf-block-start-entries", value_name = "N")]
    pub(super) block_start_entries: Option<u32>,

    #[arg(long = "ebpf-drm-fence-wait-start-entries", value_name = "N")]
    pub(super) drm_fence_wait_start_entries: Option<u32>,

    #[arg(long = "ebpf-drm-fence-signal-entries", value_name = "N")]
    pub(super) drm_fence_signal_entries: Option<u32>,

    #[arg(long = "otlp-endpoint", value_name = "URL")]
    pub(super) otlp_endpoint: Option<String>,

    #[arg(long = "otel-service-name", default_value = "stutter")]
    pub(super) otel_service_name: String,

    #[arg(long = "auto-focus")]
    pub(super) auto_focus: bool,

    #[arg(
        long = "focus-source",
        value_enum,
        default_value_t = FocusSource::Heuristic,
        help = "Auto-focus source: heuristic, foreground, or hybrid"
    )]
    pub(super) focus_source: FocusSource,

    #[arg(
        long = "foreground-window",
        help = "Record foreground-window events even when explicit targets are used"
    )]
    pub(super) foreground_window: bool,

    #[arg(
        long = "foreground-source",
        value_enum,
        default_value_t = ForegroundSource::Auto,
        help = "Foreground-window provider: auto, sway, hyprland, x11"
    )]
    pub(super) foreground_source: ForegroundSource,

    #[arg(long = "foreground-poll-ms", default_value_t = 1000)]
    pub(super) foreground_poll_ms: u64,

    #[arg(long = "foreground-max-stale-ms", default_value_t = 2500)]
    pub(super) foreground_max_stale_ms: u64,

    #[arg(long = "foreground-include-title")]
    pub(super) foreground_include_title: bool,

    #[arg(long = "auto-focus-poll-ms", default_value_t = 1000)]
    pub(super) auto_focus_poll_ms: u64,

    #[arg(long = "auto-focus-min-confidence", default_value_t = 0.60)]
    pub(super) auto_focus_min_confidence: f32,

    #[arg(long = "auto-focus-switch-cooldown-ms", default_value_t = 5000)]
    pub(super) auto_focus_switch_cooldown_ms: u64,

    #[arg(long = "auto-focus-switch-margin", default_value_t = 0.20)]
    pub(super) auto_focus_switch_margin: f32,

    #[arg(long = "auto-focus-required-polls", default_value_t = 2)]
    pub(super) auto_focus_required_polls: u32,

    #[arg(long = "auto-focus-max-roots", default_value_t = 4)]
    pub(super) auto_focus_max_roots: usize,

    #[arg(long = "remote", value_name = "URL")]
    pub(super) remote: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct RecordArgs {
    #[command(flatten)]
    pub(super) monitor: MonitorArgs,

    #[arg(long, value_name = "SECONDS")]
    pub(super) duration: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct BenchArgs {
    #[command(flatten)]
    pub(super) monitor: MonitorArgs,

    #[arg(long, value_name = "SECONDS")]
    pub(super) duration: u64,

    #[arg(long = "scenario", value_name = "NAME")]
    pub(super) scenario: String,

    #[arg(
        long = "role",
        value_name = "baseline|current",
        default_value = "baseline"
    )]
    pub(super) role: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct MonitorArgPresence {
    pub(super) watch_poll_ms: bool,
    pub(super) follow_exec: bool,
    pub(super) cpu_perf_max_tasks: bool,
    pub(super) runtime_slices_max_tasks: bool,
    pub(super) live_diagnosis_cluster_window_ms: bool,
    pub(super) otel_service_name: bool,
    pub(super) focus_source: bool,
    pub(super) foreground_source: bool,
    pub(super) wayland_presentation_source: bool,
    pub(super) foreground_poll_ms: bool,
    pub(super) foreground_max_stale_ms: bool,
    pub(super) auto_focus_poll_ms: bool,
    pub(super) auto_focus_min_confidence: bool,
    pub(super) auto_focus_switch_cooldown_ms: bool,
    pub(super) auto_focus_switch_margin: bool,
    pub(super) auto_focus_required_polls: bool,
    pub(super) auto_focus_max_roots: bool,
}

impl MonitorArgPresence {
    fn from_matches(matches: &ArgMatches) -> Self {
        fn command_line(matches: &ArgMatches, id: &str) -> bool {
            matches.value_source(id) == Some(ValueSource::CommandLine)
        }

        Self {
            watch_poll_ms: command_line(matches, "watch_poll_ms"),
            follow_exec: command_line(matches, "follow_exec"),
            cpu_perf_max_tasks: command_line(matches, "cpu_perf_max_tasks"),
            runtime_slices_max_tasks: command_line(matches, "runtime_slices_max_tasks"),
            live_diagnosis_cluster_window_ms: command_line(
                matches,
                "live_diagnosis_cluster_window_ms",
            ),
            otel_service_name: command_line(matches, "otel_service_name"),
            focus_source: command_line(matches, "focus_source"),
            foreground_source: command_line(matches, "foreground_source"),
            wayland_presentation_source: command_line(matches, "wayland_presentation_source"),
            foreground_poll_ms: command_line(matches, "foreground_poll_ms"),
            foreground_max_stale_ms: command_line(matches, "foreground_max_stale_ms"),
            auto_focus_poll_ms: command_line(matches, "auto_focus_poll_ms"),
            auto_focus_min_confidence: command_line(matches, "auto_focus_min_confidence"),
            auto_focus_switch_cooldown_ms: command_line(matches, "auto_focus_switch_cooldown_ms"),
            auto_focus_switch_margin: command_line(matches, "auto_focus_switch_margin"),
            auto_focus_required_polls: command_line(matches, "auto_focus_required_polls"),
            auto_focus_max_roots: command_line(matches, "auto_focus_max_roots"),
        }
    }

    pub(super) fn autotune_monitor_defaults() -> Self {
        Self {
            focus_source: true,
            auto_focus_min_confidence: true,
            auto_focus_required_polls: true,
            auto_focus_max_roots: true,
            ..Self::default()
        }
    }
}

#[path = "monitor/args_impl.rs"]
mod args_impl;

#[derive(Debug, Clone, Copy)]
pub(super) enum RecordingMode {
    Monitor,
    ForceRecording { max_duration: Option<Duration> },
}

impl RecordingMode {
    fn force_recording(self) -> bool {
        matches!(self, Self::ForceRecording { .. })
    }

    fn max_duration(self) -> Option<Duration> {
        match self {
            Self::Monitor => None,
            Self::ForceRecording { max_duration } => max_duration,
        }
    }
}

#[path = "monitor/foreground_validation.rs"]
mod foreground_validation;

#[path = "monitor/config_builder.rs"]
mod config_builder;
pub(super) use config_builder::{
    monitor_arg_presence_from_matches, monitor_config_from_monitor_args_with_presence,
};
#[cfg(test)]
pub(super) use config_builder::{
    monitor_config_from_monitor_args_with_file,
    monitor_config_from_monitor_args_with_file_and_presence,
};

#[cfg(test)]
fn parse_monitor_config_for_phase15<const N: usize>(
    args: [&str; N],
) -> anyhow::Result<Arc<crate::config::model::MonitorConfig>> {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    match super::parse_app_command_from(args.iter().map(OsString::from))? {
        AppCommand::Monitor(input) => Ok(input.config.clone()),
        other => anyhow::bail!("expected AppCommand::Monitor, got {other:?}"),
    }
}

#[cfg(test)]
#[path = "monitor/tests/target_filter.rs"]
mod monitor_target_filter_cli_tests;

#[cfg(test)]
#[path = "monitor/tests/record_check_performance.rs"]
mod monitor_record_check_performance_cli_tests;

#[cfg(test)]
#[path = "monitor/tests/bench.rs"]
mod bench_cli_tests;

#[cfg(test)]
#[path = "monitor/tests/auto_focus.rs"]
mod auto_focus_cli_tests;
