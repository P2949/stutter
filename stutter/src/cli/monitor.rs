use std::time::Duration;

use clap::ArgMatches;

use super::*;
use crate::config::TARGET_PIDS_MAX;

#[derive(Args, Debug, Clone)]
pub struct MonitorArgs {
    #[arg(long = "pid", short = 'p', value_name = "PID")]
    pub target_pids: Vec<u32>,

    #[arg(long = "tree-pid", value_name = "PID")]
    pub tree_pids: Vec<u32>,

    #[arg(long = "exclude-tree-pid", value_name = "PID")]
    pub exclude_tree_pids: Vec<u32>,

    #[arg(long = "summary-ms", value_name = "MS")]
    pub summary_period_ms: Option<u64>,

    #[arg(long = "epoch", value_name = "MS")]
    pub epoch_period_ms: Option<u64>,

    #[arg(long = "spike-us", value_name = "US")]
    pub spike_threshold_us: Option<u64>,

    #[arg(long = "alert-threshold-ms", value_name = "MS")]
    pub alert_threshold_ms: Option<u64>,

    #[arg(long = "alert-webhook-url", value_name = "URL")]
    pub alert_webhook_url: Option<String>,

    #[arg(long, short = 'v')]
    pub verbose: bool,

    #[arg(long = "run-name", value_name = "NAME")]
    pub run_name: Option<String>,

    #[arg(long = "out-dir", alias = "out", value_name = "PATH")]
    pub out_dir: Option<PathBuf>,

    #[arg(long = "include-comm", value_name = "PATTERN")]
    pub include_comm: Vec<String>,

    #[arg(long = "exclude-comm", value_name = "PATTERN")]
    pub exclude_comm: Vec<String>,

    #[arg(long = "keep-missing-pid")]
    pub keep_missing_pid: bool,

    #[arg(long = "watch-process", value_name = "COMM")]
    pub watch_process: Option<String>,

    #[arg(long)]
    pub persistent: bool,

    #[arg(long = "watch-poll-ms", default_value_t = 2_000)]
    pub watch_poll_ms: u64,

    #[arg(long = "watch-timeout-seconds", value_name = "SECONDS")]
    pub watch_timeout_seconds: Option<u64>,

    #[arg(long, value_name = "N")]
    pub max_tasks: Option<usize>,

    #[arg(long = "csv", value_name = "PATH")]
    pub csv_path: Option<PathBuf>,

    #[arg(
        long = "stream-csv",
        value_name = "PATH_OR_-",
        conflicts_with = "csv_path"
    )]
    pub stream_csv: Option<String>,

    #[arg(long = "irq-latency")]
    pub irq_latency: bool,

    #[arg(long = "irq", value_name = "IRQ")]
    pub irqs: Vec<u32>,

    #[arg(long = "hwmon", id = "hwmon", conflicts_with = "no_hwmon")]
    pub hwmon: bool,

    #[arg(long = "no-hwmon", help = "Disable GPU hwmon telemetry")]
    pub no_hwmon: bool,

    #[arg(long = "hwmon-root", value_name = "PATH", requires = "hwmon")]
    pub hwmon_root: Option<PathBuf>,

    #[arg(long = "hwmon-drm-card", value_name = "CARD", requires = "hwmon")]
    pub hwmon_drm_card: Option<String>,

    #[arg(long = "hwmon-render-node", value_name = "NODE", requires = "hwmon")]
    pub hwmon_render_node: Option<PathBuf>,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub mangohud_log: Option<PathBuf>,

    #[arg(long = "mangohud-log-live", requires = "mangohud_log")]
    pub mangohud_log_live: bool,

    #[arg(long = "tui")]
    pub tui: bool,

    #[arg(long = "retain-intervals", value_name = "N")]
    pub retain_intervals: Option<usize>,

    #[arg(long = "retention-max-runs", value_name = "N")]
    pub retention_max_run_count: Option<usize>,

    #[arg(long = "retention-max-bytes", value_name = "BYTES")]
    pub retention_max_total_bytes: Option<u64>,

    #[arg(long = "retention-max-age-seconds", value_name = "SECONDS")]
    pub retention_max_age_seconds: Option<u64>,

    #[arg(long = "retention-min-free-bytes", value_name = "BYTES")]
    pub retention_min_free_bytes: Option<u64>,

    #[arg(long = "no-record")]
    pub no_record: bool,

    #[arg(
        long = "cpu-freq",
        help = "Collect CPU frequency information (enabled by default for recording runs)",
        conflicts_with = "no_cpu_freq"
    )]
    pub cpu_freq: bool,

    #[arg(long = "no-cpu-freq", help = "Disable CPU frequency collection")]
    pub no_cpu_freq: bool,

    #[arg(long = "cgroupv2", value_name = "PATH")]
    pub cgroupv2: Option<PathBuf>,

    #[arg(long = "native-cgroup-filter", requires = "cgroupv2")]
    pub native_cgroup_filter: bool,

    #[arg(
        long = "follow-exec",
        default_value_t = true,
        action = ArgAction::SetTrue,
        conflicts_with = "no_follow_exec"
    )]
    pub follow_exec: bool,

    #[arg(long = "no-follow-exec", action = ArgAction::SetTrue)]
    pub no_follow_exec: bool,

    #[arg(long = "faults", conflicts_with = "no_faults")]
    pub faults: bool,

    #[arg(long = "no-faults", help = "Disable page fault collection")]
    pub no_faults: bool,

    #[arg(
        long = "cpu-perf",
        help = "Collect per-task CPU hardware counters for IPC/cache-miss diagnostics"
    )]
    pub cpu_perf: bool,

    #[arg(
        long = "cpu-perf-kernel",
        help = "Include kernel/hypervisor time in CPU perf counters; default is user-space only"
    )]
    pub cpu_perf_kernel: bool,

    #[arg(
        long = "cpu-perf-max-tasks",
        default_value_t = 128,
        value_name = "N",
        help = "Maximum active target tasks to attach CPU perf counters to"
    )]
    pub cpu_perf_max_tasks: usize,

    #[arg(
        long = "cpu-perf-cache-refs",
        help = "Also collect cache references so cache miss rate can be computed; otherwise only cache MPKI is computed"
    )]
    pub cpu_perf_cache_refs: bool,

    #[arg(long = "block-io", conflicts_with = "no_block_io")]
    pub block_io: bool,

    #[arg(long = "no-block-io", help = "Disable block I/O collection")]
    pub no_block_io: bool,

    #[arg(long = "stat-wait", conflicts_with = "no_stat_wait")]
    pub stat_wait: bool,

    #[arg(long = "no-stat-wait", help = "Disable stat-wait collection")]
    pub no_stat_wait: bool,

    #[arg(
        long = "runtime-slices",
        conflicts_with = "no_runtime_slices",
        help = "Collect per-thread CPU runtime/wait slices from procfs schedstat"
    )]
    pub runtime_slices: bool,

    #[arg(
        long = "no-runtime-slices",
        help = "Disable per-thread runtime-slice collection"
    )]
    pub no_runtime_slices: bool,

    #[arg(
        long = "runtime-slices-max-tasks",
        default_value_t = 256,
        value_name = "N"
    )]
    pub runtime_slices_max_tasks: usize,

    #[arg(
        long = "json-stream",
        help = "Emit scheduler spike events to stdout as newline-delimited JSON"
    )]
    pub json_stream: bool,

    #[arg(long = "metrics-port", value_name = "PORT")]
    pub metrics_port: Option<u16>,

    #[arg(
        long = "preset",
        value_name = "NAME",
        help = "Apply named monitor defaults: gaming, recording, diagnosis, lightweight"
    )]
    pub preset: Option<String>,

    #[arg(long = "ringbuf-size-kb", value_name = "KB")]
    pub ringbuf_size_kb: Option<u32>,

    #[arg(long = "wakeup-map-factor", value_name = "N")]
    pub wakeup_map_factor: Option<u32>,

    #[arg(long = "otlp-endpoint", value_name = "URL")]
    pub otlp_endpoint: Option<String>,

    #[arg(long = "otel-service-name", default_value = "stutter")]
    pub otel_service_name: String,

    #[arg(long = "auto-focus")]
    pub auto_focus: bool,

    #[arg(
        long = "focus-source",
        value_enum,
        default_value_t = FocusSource::Heuristic,
        help = "Auto-focus source: heuristic, foreground, or hybrid"
    )]
    pub focus_source: FocusSource,

    #[arg(
        long = "foreground-window",
        help = "Record foreground-window events even when explicit targets are used"
    )]
    pub foreground_window: bool,

    #[arg(
        long = "foreground-source",
        value_enum,
        default_value_t = ForegroundSource::Auto,
        help = "Foreground-window provider: auto, sway, hyprland, x11"
    )]
    pub foreground_source: ForegroundSource,

    #[arg(long = "foreground-poll-ms", default_value_t = 1000)]
    pub foreground_poll_ms: u64,

    #[arg(long = "foreground-max-stale-ms", default_value_t = 2500)]
    pub foreground_max_stale_ms: u64,

    #[arg(long = "foreground-include-title")]
    pub foreground_include_title: bool,

    #[arg(long = "auto-focus-poll-ms", default_value_t = 1000)]
    pub auto_focus_poll_ms: u64,

    #[arg(long = "auto-focus-min-confidence", default_value_t = 0.60)]
    pub auto_focus_min_confidence: f32,

    #[arg(long = "auto-focus-switch-cooldown-ms", default_value_t = 5000)]
    pub auto_focus_switch_cooldown_ms: u64,

    #[arg(long = "auto-focus-switch-margin", default_value_t = 0.20)]
    pub auto_focus_switch_margin: f32,

    #[arg(long = "auto-focus-required-polls", default_value_t = 2)]
    pub auto_focus_required_polls: u32,

    #[arg(long = "auto-focus-max-roots", default_value_t = 4)]
    pub auto_focus_max_roots: usize,

    #[arg(long = "remote", value_name = "URL")]
    pub remote: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct RecordArgs {
    #[command(flatten)]
    pub monitor: MonitorArgs,

    #[arg(long, value_name = "SECONDS")]
    pub duration: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub struct BenchArgs {
    #[command(flatten)]
    pub monitor: MonitorArgs,

    #[arg(long, value_name = "SECONDS")]
    pub duration: u64,

    #[arg(long = "scenario", value_name = "NAME")]
    pub scenario: String,

    #[arg(
        long = "role",
        value_name = "baseline|current",
        default_value = "baseline"
    )]
    pub role: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MonitorArgPresence {
    pub watch_poll_ms: bool,
    pub follow_exec: bool,
    pub cpu_perf_max_tasks: bool,
    pub runtime_slices_max_tasks: bool,
    pub otel_service_name: bool,
    pub focus_source: bool,
    pub foreground_source: bool,
    pub foreground_poll_ms: bool,
    pub foreground_max_stale_ms: bool,
    pub auto_focus_poll_ms: bool,
    pub auto_focus_min_confidence: bool,
    pub auto_focus_switch_cooldown_ms: bool,
    pub auto_focus_switch_margin: bool,
    pub auto_focus_required_polls: bool,
    pub auto_focus_max_roots: bool,
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
            otel_service_name: command_line(matches, "otel_service_name"),
            focus_source: command_line(matches, "focus_source"),
            foreground_source: command_line(matches, "foreground_source"),
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

    pub fn autotune_monitor_defaults() -> Self {
        Self {
            focus_source: true,
            auto_focus_min_confidence: true,
            auto_focus_required_polls: true,
            auto_focus_max_roots: true,
            ..Self::default()
        }
    }
}

impl MonitorArgs {
    pub fn into_monitor_config_layer(self, presence: MonitorArgPresence) -> MonitorConfigLayer {
        MonitorConfigLayer {
            target_pids: (!self.target_pids.is_empty()).then(|| self.target_pids.clone()),
            tree_pids: (!self.tree_pids.is_empty()).then(|| self.tree_pids.clone()),
            exclude_tree_pids: (!self.exclude_tree_pids.is_empty())
                .then(|| self.exclude_tree_pids.clone()),
            summary_period_ms: self.summary_period_ms,
            epoch_period_ms: self.epoch_period_ms.map(Some),
            spike_threshold_ns: self
                .spike_threshold_us
                .map(|value| value.saturating_mul(1_000)),
            alert_threshold_ns: self
                .alert_threshold_ms
                .map(|value| Some(value.saturating_mul(1_000_000))),
            alert_webhook_url: self.alert_webhook_url.clone().map(Some),
            verbose: self.verbose.then_some(true),
            watch_poll_ms: presence.watch_poll_ms.then_some(self.watch_poll_ms),
            watch_timeout: self
                .watch_timeout_seconds
                .map(|seconds| Some(Duration::from_secs(seconds))),
            include_comm: (!self.include_comm.is_empty()).then(|| self.include_comm.clone()),
            exclude_comm: (!self.exclude_comm.is_empty()).then(|| self.exclude_comm.clone()),
            keep_missing_pid: self.keep_missing_pid.then_some(true),
            watch_process: self.watch_process.clone().map(Some),
            persistent: self.persistent.then_some(true),
            max_tasks: self.max_tasks,
            csv_stream: match (&self.csv_path, &self.stream_csv) {
                (Some(path), None) => Some(Some(CsvStreamTarget::File(path.clone()))),
                (None, Some(value)) if value == "-" => Some(Some(CsvStreamTarget::Stdout)),
                (None, Some(value)) if value.trim().is_empty() => None,
                (None, Some(value)) => Some(Some(CsvStreamTarget::File(PathBuf::from(value)))),
                (None, None) => None,
                (Some(_), Some(_)) => None,
            },
            irq_latency: self.irq_latency.then_some(true),
            irqs: (!self.irqs.is_empty()).then(|| self.irqs.clone()),
            hwmon: if self.no_hwmon {
                Some(false)
            } else if self.hwmon {
                Some(true)
            } else {
                None
            },
            hwmon_root: self.hwmon_root.clone().map(Some),
            hwmon_drm_card: self.hwmon_drm_card.clone().map(Some),
            hwmon_render_node: self.hwmon_render_node.clone().map(Some),
            cpu_freq: if self.no_cpu_freq {
                Some(false)
            } else if self.cpu_freq {
                Some(true)
            } else {
                None
            },
            cgroupv2: self.cgroupv2.clone().map(Some),
            native_cgroup_filter: self.native_cgroup_filter.then_some(true),
            follow_exec: if self.no_follow_exec {
                Some(false)
            } else if presence.follow_exec {
                Some(self.follow_exec)
            } else {
                None
            },
            faults: if self.no_faults {
                Some(false)
            } else if self.faults {
                Some(true)
            } else {
                None
            },
            cpu_perf: self.cpu_perf.then_some(true),
            cpu_perf_kernel: self.cpu_perf_kernel.then_some(true),
            cpu_perf_max_tasks: presence
                .cpu_perf_max_tasks
                .then_some(self.cpu_perf_max_tasks),
            cpu_perf_cache_refs: self.cpu_perf_cache_refs.then_some(true),
            block_io: if self.no_block_io {
                Some(false)
            } else if self.block_io {
                Some(true)
            } else {
                None
            },
            stat_wait: if self.no_stat_wait {
                Some(false)
            } else if self.stat_wait {
                Some(true)
            } else {
                None
            },
            runtime_slices: if self.no_runtime_slices {
                Some(false)
            } else if self.runtime_slices {
                Some(true)
            } else {
                None
            },
            runtime_slices_max_tasks: presence
                .runtime_slices_max_tasks
                .then_some(self.runtime_slices_max_tasks),
            mangohud_log: self.mangohud_log.clone().map(Some),
            mangohud_log_live: self.mangohud_log_live.then_some(true),
            tui: self.tui.then_some(true),
            json_stream: self.json_stream.then_some(true),
            metrics_port: self.metrics_port.map(Some),
            ringbuf_size_kb: self.ringbuf_size_kb.map(Some),
            wakeup_map_factor: self.wakeup_map_factor.map(Some),
            otlp_endpoint: self.otlp_endpoint.clone().map(Some),
            otel_service_name: presence
                .otel_service_name
                .then(|| self.otel_service_name.clone()),
            auto_focus: self.auto_focus.then_some(true),
            focus_source: presence.focus_source.then_some(self.focus_source),
            foreground_window: self.foreground_window.then_some(true),
            foreground_source: presence.foreground_source.then_some(self.foreground_source),
            foreground_poll_ms: presence
                .foreground_poll_ms
                .then_some(self.foreground_poll_ms),
            foreground_max_stale_ms: presence
                .foreground_max_stale_ms
                .then_some(self.foreground_max_stale_ms),
            foreground_include_title: self.foreground_include_title.then_some(true),
            auto_focus_poll_ms: presence
                .auto_focus_poll_ms
                .then_some(self.auto_focus_poll_ms),
            auto_focus_min_confidence: presence
                .auto_focus_min_confidence
                .then_some(self.auto_focus_min_confidence),
            auto_focus_switch_cooldown_ms: presence
                .auto_focus_switch_cooldown_ms
                .then_some(self.auto_focus_switch_cooldown_ms),
            auto_focus_switch_margin: presence
                .auto_focus_switch_margin
                .then_some(self.auto_focus_switch_margin),
            auto_focus_required_polls: presence
                .auto_focus_required_polls
                .then_some(self.auto_focus_required_polls),
            auto_focus_max_roots: presence
                .auto_focus_max_roots
                .then_some(self.auto_focus_max_roots),
            retain_intervals: self.retain_intervals.map(Some),
            retention_max_run_count: self.retention_max_run_count.map(Some),
            retention_max_total_bytes: self.retention_max_total_bytes.map(Some),
            retention_max_age_seconds: self.retention_max_age_seconds.map(Some),
            retention_min_free_bytes: self.retention_min_free_bytes.map(Some),
            run_name: self.run_name.clone().map(Some),
            output_dir: self.out_dir.clone().map(Some),
            remote: self.remote.clone().map(Some),
            ..MonitorConfigLayer::default()
        }
    }
}

impl Default for MonitorArgs {
    fn default() -> Self {
        Self {
            target_pids: Vec::new(),
            tree_pids: Vec::new(),
            exclude_tree_pids: Vec::new(),
            summary_period_ms: None,
            epoch_period_ms: None,
            spike_threshold_us: None,
            alert_threshold_ms: None,
            alert_webhook_url: None,
            verbose: false,
            run_name: None,
            out_dir: None,
            include_comm: Vec::new(),
            exclude_comm: Vec::new(),
            keep_missing_pid: false,
            watch_process: None,
            persistent: false,
            watch_poll_ms: 2000,
            watch_timeout_seconds: None,
            max_tasks: None,
            csv_path: None,
            stream_csv: None,
            irq_latency: false,
            irqs: Vec::new(),
            hwmon: false,
            no_hwmon: false,
            hwmon_root: None,
            hwmon_drm_card: None,
            hwmon_render_node: None,
            mangohud_log: None,
            mangohud_log_live: false,
            tui: false,
            retain_intervals: None,
            retention_max_run_count: None,
            retention_max_total_bytes: None,
            retention_max_age_seconds: None,
            retention_min_free_bytes: None,
            no_record: false,
            cpu_freq: false,
            no_cpu_freq: false,
            cgroupv2: None,
            native_cgroup_filter: false,
            follow_exec: true,
            no_follow_exec: false,
            faults: false,
            no_faults: false,
            cpu_perf: false,
            cpu_perf_kernel: false,
            cpu_perf_max_tasks: 128,
            cpu_perf_cache_refs: false,
            block_io: false,
            no_block_io: false,
            stat_wait: false,
            no_stat_wait: false,
            runtime_slices: false,
            no_runtime_slices: false,
            runtime_slices_max_tasks: 256,
            json_stream: false,
            metrics_port: None,
            preset: None,
            ringbuf_size_kb: None,
            wakeup_map_factor: None,
            otlp_endpoint: None,
            otel_service_name: "stutter".to_owned(),
            auto_focus: false,
            focus_source: FocusSource::Heuristic,
            foreground_window: false,
            foreground_source: ForegroundSource::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            foreground_include_title: false,
            auto_focus_poll_ms: 1000,
            auto_focus_min_confidence: 0.60,
            auto_focus_switch_cooldown_ms: 5000,
            auto_focus_switch_margin: 0.20,
            auto_focus_required_polls: 2,
            auto_focus_max_roots: 4,
            remote: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RecordingMode {
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

impl FocusSource {
    pub fn parse_config_value(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "heuristic" => Ok(Self::Heuristic),
            "foreground" => Ok(Self::Foreground),
            "hybrid" => Ok(Self::Hybrid),
            other => anyhow::bail!(
                "focus_source must be heuristic, foreground, or hybrid, got {other:?}"
            ),
        }
    }
}

impl ForegroundSource {
    pub fn parse_config_value(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "sway" => Ok(Self::Sway),
            "hyprland" => Ok(Self::Hyprland),
            "x11" => Ok(Self::X11),
            other => anyhow::bail!(
                "foreground_source must be auto, sway, hyprland, or x11, got {other:?}"
            ),
        }
    }
}

pub fn monitor_arg_presence_from_matches(
    matches: &ArgMatches,
    subcommand: Option<&str>,
) -> MonitorArgPresence {
    match subcommand {
        Some(expected) => match matches.subcommand() {
            Some((actual, sub_matches)) if actual == expected => {
                MonitorArgPresence::from_matches(sub_matches)
            }
            _ => MonitorArgPresence::default(),
        },
        None => MonitorArgPresence::from_matches(matches),
    }
}

fn merge_bool(
    builtin: bool,
    file_value: Option<bool>,
    preset_value: Option<bool>,
    cli_positive: bool,
    cli_negative: bool,
) -> bool {
    if cli_negative {
        false
    } else if cli_positive {
        true
    } else if let Some(value) = preset_value {
        value
    } else if let Some(value) = file_value {
        value
    } else {
        builtin
    }
}

#[allow(dead_code)]
pub fn monitor_config_from_monitor_args(
    args: MonitorArgs,
    recording_mode: RecordingMode,
) -> anyhow::Result<MonitorConfig> {
    let file_config = crate::config_file::load_user_config()?;
    monitor_config_from_monitor_args_with_file_and_presence(
        args,
        file_config,
        recording_mode,
        MonitorArgPresence::default(),
    )
}

pub fn monitor_config_from_monitor_args_with_presence(
    args: MonitorArgs,
    recording_mode: RecordingMode,
    cli_presence: MonitorArgPresence,
) -> anyhow::Result<MonitorConfig> {
    let file_config = crate::config_file::load_user_config()?;
    monitor_config_from_monitor_args_with_file_and_presence(
        args,
        file_config,
        recording_mode,
        cli_presence,
    )
}

#[allow(dead_code)]
pub fn monitor_config_from_monitor_args_with_file(
    args: MonitorArgs,
    file_config: Option<crate::config_file::UserConfigFile>,
    recording_mode: RecordingMode,
) -> anyhow::Result<MonitorConfig> {
    monitor_config_from_monitor_args_with_file_and_presence(
        args,
        file_config,
        recording_mode,
        MonitorArgPresence::default(),
    )
}

pub fn monitor_config_from_monitor_args_with_file_and_presence(
    mut args: MonitorArgs,
    file_config: Option<crate::config_file::UserConfigFile>,
    recording_mode: RecordingMode,
    cli_presence: MonitorArgPresence,
) -> anyhow::Result<MonitorConfig> {
    // This is a very large function - the full implementation needs to be moved from mod.rs
    // For now, let me include the full implementation from the original file
    let user_file = file_config;
    let file_config = user_file.clone().unwrap_or_default();

    let preset = match args.preset.as_deref() {
        Some(name) => Some(name.parse::<crate::presets::Preset>()?),
        None => None,
    };

    let preset_defaults = preset.map(|preset| preset.defaults()).unwrap_or_default();

    let summary_period_ms = args
        .summary_period_ms
        .or(file_config.summary_ms)
        .unwrap_or(1_000);
    let spike_threshold_us = args
        .spike_threshold_us
        .or(file_config.spike_us)
        .unwrap_or(1_000);
    let max_tasks = args.max_tasks.or(file_config.max_tasks).unwrap_or(1024);

    if !args.include_comm.is_empty() {
        // use CLI
    } else if let Some(config_include) = file_config.include_comm.clone() {
        args.include_comm = config_include;
    }

    if !args.exclude_comm.is_empty() {
        // use CLI
    } else if let Some(config_exclude) = file_config.exclude_comm.clone() {
        args.exclude_comm = config_exclude;
    }

    let _hwmon = merge_bool(
        false,
        file_config.hwmon,
        preset_defaults.hwmon,
        args.hwmon,
        args.no_hwmon,
    );

    let cpu_freq_config = merge_bool(
        false,
        file_config.cpu_freq.or(file_config.no_cpu_freq.map(|n| !n)),
        preset_defaults.cpu_freq,
        args.cpu_freq,
        args.no_cpu_freq,
    );

    let faults = merge_bool(
        false,
        None,
        preset_defaults.faults,
        args.faults,
        args.no_faults,
    );

    let stat_wait = merge_bool(
        false,
        None,
        preset_defaults.stat_wait,
        args.stat_wait,
        args.no_stat_wait,
    );

    let block_io = merge_bool(
        false,
        None,
        preset_defaults.block_io,
        args.block_io,
        args.no_block_io,
    );
    let runtime_slices = merge_bool(
        false,
        None,
        preset_defaults.runtime_slices,
        args.runtime_slices,
        args.no_runtime_slices,
    );

    let irq_latency = merge_bool(
        false,
        None,
        preset_defaults.irq_latency,
        args.irq_latency,
        false,
    );

    if !args.foreground_window
        && let Some(foreground_window) = file_config.foreground_window
    {
        args.foreground_window = foreground_window;
    }

    if !cli_presence.focus_source
        && let Some(focus_source) = file_config.focus_source.as_deref()
    {
        args.focus_source = FocusSource::parse_config_value(focus_source)?;
    }

    if !cli_presence.foreground_source
        && let Some(foreground_source) = file_config.foreground_source.as_deref()
    {
        args.foreground_source = ForegroundSource::parse_config_value(foreground_source)?;
    }

    if !cli_presence.foreground_poll_ms
        && let Some(foreground_poll_ms) = file_config.foreground_poll_ms
    {
        args.foreground_poll_ms = foreground_poll_ms;
    }

    if !cli_presence.foreground_max_stale_ms
        && let Some(foreground_max_stale_ms) = file_config.foreground_max_stale_ms
    {
        args.foreground_max_stale_ms = foreground_max_stale_ms;
    }

    if !args.foreground_include_title
        && let Some(foreground_include_title) = file_config.foreground_include_title
    {
        args.foreground_include_title = foreground_include_title;
    }

    validate_foreground_title_monitor_args(&args)?;
    normalize_foreground_monitor_args(&mut args);
    validate_foreground_monitor_args(&args)?;

    validate_pids("--pid", &args.target_pids)?;
    validate_pids("--tree-pid", &args.tree_pids)?;
    validate_pids("--exclude-tree-pid", &args.exclude_tree_pids)?;

    #[allow(clippy::collapsible_if)]
    if let Some(kb) = args.ringbuf_size_kb {
        if !(64..=16 * 1024).contains(&kb) {
            anyhow::bail!("--ringbuf-size-kb must be between 64 and 16384");
        }
    }

    #[allow(clippy::collapsible_if)]
    if let Some(factor) = args.wakeup_map_factor {
        if factor == 0 || factor > 64 {
            anyhow::bail!("--wakeup-map-factor must be between 1 and 64");
        }
    }

    if args.otlp_endpoint.is_some() && !cfg!(feature = "otel") {
        anyhow::bail!("OpenTelemetry support was not compiled in. Rebuild with --features otel.");
    }

    if args.otel_service_name.trim().is_empty() {
        anyhow::bail!("--otel-service-name must not be empty");
    }

    #[allow(clippy::collapsible_if)]
    if let Some(endpoint) = &args.otlp_endpoint {
        if endpoint.trim().is_empty() {
            anyhow::bail!("--otlp-endpoint must not be empty");
        }
    }

    if summary_period_ms == 0 {
        anyhow::bail!("--summary-ms must be greater than zero");
    }
    if matches!(args.epoch_period_ms, Some(0)) {
        anyhow::bail!("--epoch must be greater than zero");
    }

    if spike_threshold_us == 0 {
        anyhow::bail!("--spike-us must be greater than zero");
    }
    if matches!(args.alert_threshold_ms, Some(0)) {
        anyhow::bail!("--alert-threshold-ms must be greater than zero");
    }
    if args.watch_poll_ms == 0 {
        anyhow::bail!("--watch-poll-ms must be greater than zero");
    }
    if matches!(args.watch_timeout_seconds, Some(0)) {
        anyhow::bail!("--watch-timeout-seconds must be greater than zero");
    }
    if max_tasks == 0 {
        anyhow::bail!("--max-tasks must be greater than zero");
    }
    if args.cpu_perf_max_tasks == 0 {
        anyhow::bail!("--cpu-perf-max-tasks must be greater than zero");
    }
    if args.runtime_slices_max_tasks == 0 {
        anyhow::bail!("--runtime-slices-max-tasks must be greater than zero");
    }
    if matches!(args.retention_max_run_count, Some(0)) {
        anyhow::bail!("--retention-max-runs must be greater than zero");
    }
    if matches!(args.retention_max_total_bytes, Some(0)) {
        anyhow::bail!("--retention-max-bytes must be greater than zero");
    }
    if matches!(args.retention_max_age_seconds, Some(0)) {
        anyhow::bail!("--retention-max-age-seconds must be greater than zero");
    }
    if matches!(args.retention_min_free_bytes, Some(0)) {
        anyhow::bail!("--retention-min-free-bytes must be greater than zero");
    }

    args.target_pids.sort_unstable();
    args.target_pids.dedup();
    args.tree_pids.sort_unstable();
    args.tree_pids.dedup();
    args.exclude_tree_pids.sort_unstable();
    args.exclude_tree_pids.dedup();
    args.include_comm.sort();
    args.include_comm.dedup();
    args.exclude_comm.sort();
    args.exclude_comm.dedup();
    args.irqs.sort_unstable();
    args.irqs.dedup();

    validate_comm_patterns("--include-comm", &args.include_comm)?;
    validate_comm_patterns("--exclude-comm", &args.exclude_comm)?;

    if matches!(args.watch_process.as_deref(), Some("")) {
        anyhow::bail!("--watch-process must not be empty");
    }
    if args.persistent && args.watch_process.is_none() {
        anyhow::bail!("--persistent requires --watch-process");
    }
    if args.irq_latency && args.irqs.is_empty() {
        anyhow::bail!(
            "--irq-latency requires at least one explicit --irq <N>; inspect /proc/interrupts to find the IRQ number for your GPU or device"
        );
    }
    if matches!(args.hwmon_drm_card.as_deref(), Some("")) {
        anyhow::bail!("--hwmon-drm-card must not be empty");
    }
    if matches!(args.alert_webhook_url.as_deref(), Some("")) {
        anyhow::bail!("--alert-webhook-url must not be empty");
    }

    if args.target_pids.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many unique target PIDs: got {}, but TARGET_PIDS supports at most {}",
            args.target_pids.len(),
            TARGET_PIDS_MAX
        );
    }

    let spike_threshold_ns = spike_threshold_us
        .checked_mul(1_000)
        .ok_or_else(|| anyhow::anyhow!("--spike-us value is too large"))?;
    let summary_period_ms = args.epoch_period_ms.unwrap_or(summary_period_ms);
    let alert_threshold_ns = args
        .alert_threshold_ms
        .map(|threshold_ms| {
            threshold_ms
                .checked_mul(1_000_000)
                .ok_or_else(|| anyhow::anyhow!("--alert-threshold-ms value is too large"))
        })
        .transpose()?;

    match (&args.csv_path, &args.stream_csv) {
        (None, Some(value)) if value.trim().is_empty() => {
            anyhow::bail!("--stream-csv path must not be empty");
        }
        (Some(_), Some(_)) => {
            anyhow::bail!("--stream-csv conflicts with --csv");
        }
        _ => {}
    }

    let alert_webhook_url = if alert_threshold_ns.is_some() {
        args.alert_webhook_url.clone().or_else(|| {
            std::env::var("STUTTER_ALERT_WEBHOOK_URL")
                .ok()
                .filter(|url| !url.is_empty())
        })
    } else {
        args.alert_webhook_url.clone()
    };

    let mut layer = args.clone().into_monitor_config_layer(cli_presence);
    layer.alert_webhook_url = alert_webhook_url.map(Some);
    if let Some(max_duration) = recording_mode.max_duration() {
        layer.max_duration = Some(Some(max_duration));
    }
    if let Some(epoch) = args.epoch_period_ms {
        layer.summary_period_ms = Some(epoch);
    }

    let is_recording = if args.no_record {
        false
    } else {
        recording_mode.force_recording() || args.run_name.is_some() || args.out_dir.is_some()
    };

    if is_recording {
        let run_name = args.run_name.or_else(|| {
            recording_mode
                .force_recording()
                .then(|| "record".to_owned())
        });
        layer.run_name = run_name.map(Some);
        layer.output_dir = args.out_dir.map(Some);
    }

    let cpu_freq = (cpu_freq_config || is_recording) && !args.no_cpu_freq;
    if cpu_freq {
        layer.cpu_freq = Some(true);
    }

    let resolved = resolve_monitor_config_sources(ConfigSources {
        defaults: DefaultConfig {
            config: MonitorConfig::default(),
        },
        user_file,
        preset: preset.map(|preset| PresetConfig {
            layer: MonitorConfigLayer::from_preset_defaults(preset.defaults()),
        }),
        overrides: CliOverrides { layer }.into(),
    })?;
    let mut config = resolved.config;

    config.timing.summary_period_ms = summary_period_ms;
    config.timing.spike_threshold_ns = spike_threshold_ns;
    config.alerts.threshold_ns = alert_threshold_ns;

    config.probes.faults = faults;
    config.probes.stat_wait = stat_wait;
    config.probes.block_io = block_io;
    config.probes.runtime_slices = runtime_slices;
    config.probes.irq_latency = irq_latency;

    if config.csv_streams_to_stdout() && config.outputs.json_stream {
        anyhow::bail!(
            "--stream-csv - cannot be used with --json-stream because both write to stdout"
        );
    }

    Ok(config)
}

fn normalize_foreground_monitor_args(args: &mut MonitorArgs) {
    if args.focus_source != FocusSource::Heuristic {
        args.foreground_window = true;
    }
}

fn validate_foreground_title_monitor_args(args: &MonitorArgs) -> anyhow::Result<()> {
    let foreground_focus_requested = args.auto_focus
        && matches!(
            args.focus_source,
            FocusSource::Foreground | FocusSource::Hybrid
        );

    if args.foreground_include_title && !args.foreground_window && !foreground_focus_requested {
        anyhow::bail!(
            "--foreground-include-title requires --foreground-window or --auto-focus with --focus-source foreground or hybrid"
        );
    }

    Ok(())
}

fn validate_foreground_monitor_args(args: &MonitorArgs) -> anyhow::Result<()> {
    if args.foreground_poll_ms < 100 {
        anyhow::bail!("--foreground-poll-ms must be >= 100");
    }

    if args.foreground_max_stale_ms < args.foreground_poll_ms {
        eprintln!(
            "warning: foreground max stale is lower than poll interval; provider errors may clear focus quickly"
        );
    }

    Ok(())
}

#[cfg(test)]
#[allow(dead_code)]
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
mod monitor_target_filter_cli_tests {
    use super::*;

    #[test]
    fn parses_pid_targets_and_deduplicates() {
        let config = parse_monitor_config_for_phase15([
            "stutter", "monitor", "--pid", "42", "--pid", "7", "--pid", "42",
        ])
        .unwrap();

        assert_eq!(config.target.target_pids, vec![7, 42]);
        assert!(config.target.tree_pids.is_empty());
    }

    #[test]
    fn parses_tree_pids_and_exclude_tree_pids() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--tree-pid",
            "42",
            "--tree-pid",
            "42",
            "--tree-pid",
            "7",
            "--exclude-tree-pid",
            "100",
            "--exclude-tree-pid",
            "100",
            "--exclude-tree-pid",
            "8",
        ])
        .unwrap();

        assert_eq!(config.target.tree_pids, vec![7, 42]);
        assert_eq!(config.target.exclude_tree_pids, vec![8, 100]);
    }

    #[test]
    fn parses_include_and_exclude_comm_filters() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "record",
            "--tree-pid",
            "42",
            "--include-comm",
            "RenderThread",
            "--exclude-comm",
            "steamwebhelper",
        ])
        .unwrap();

        assert_eq!(config.target.include_comm, vec!["RenderThread".to_owned()]);
        assert_eq!(
            config.target.exclude_comm,
            vec!["steamwebhelper".to_owned()]
        );
    }

    #[test]
    fn include_and_exclude_comm_filters_are_sorted_and_deduplicated() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--tree-pid",
            "42",
            "--include-comm",
            "RenderThread",
            "--include-comm",
            "Game",
            "--include-comm",
            "RenderThread",
            "--exclude-comm",
            "steamwebhelper",
            "--exclude-comm",
            "browser",
            "--exclude-comm",
            "steamwebhelper",
        ])
        .unwrap();

        assert_eq!(
            config.target.include_comm,
            vec!["Game".to_owned(), "RenderThread".to_owned()]
        );
        assert_eq!(
            config.target.exclude_comm,
            vec!["browser".to_owned(), "steamwebhelper".to_owned()]
        );
    }

    #[test]
    fn rejects_zero_pid_targets() {
        let err =
            parse_monitor_config_for_phase15(["stutter", "monitor", "--pid", "0"]).unwrap_err();

        assert!(err.to_string().contains("--pid must be greater than zero"));
    }

    #[test]
    fn rejects_zero_tree_pid_targets() {
        let err = parse_monitor_config_for_phase15(["stutter", "monitor", "--tree-pid", "0"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--tree-pid must be greater than zero")
        );
    }

    #[test]
    fn rejects_zero_exclude_tree_pid() {
        let err = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--tree-pid",
            "42",
            "--exclude-tree-pid",
            "0",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--exclude-tree-pid must be greater than zero")
        );
    }

    #[test]
    fn rejects_zero_max_tasks() {
        let err = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--max-tasks",
            "0",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--max-tasks must be greater than zero")
        );
    }

    #[test]
    fn parses_max_tasks() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--max-tasks",
            "2048",
        ])
        .unwrap();

        assert_eq!(config.target.max_tasks, 2048);
    }

    #[test]
    fn native_cgroup_filter_requires_cgroupv2() {
        let result = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--native-cgroup-filter",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn native_cgroup_filter_sets_config() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--cgroupv2",
            "/sys/fs/cgroup/test.slice",
            "--native-cgroup-filter",
        ])
        .unwrap();

        assert_eq!(
            config.target.cgroupv2.as_deref(),
            Some(std::path::Path::new("/sys/fs/cgroup/test.slice"))
        );
        assert!(config.safety.native_cgroup_filter);
    }

    #[test]
    fn native_cgroup_filter_defaults_false() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--cgroupv2",
            "/sys/fs/cgroup/test.slice",
        ])
        .unwrap();

        assert_eq!(
            config.target.cgroupv2.as_deref(),
            Some(std::path::Path::new("/sys/fs/cgroup/test.slice"))
        );
        assert!(!config.safety.native_cgroup_filter);
    }

    #[test]
    fn cli_accepts_auto_focus_foreground_source() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--auto-focus",
            "--focus-source",
            "foreground",
            "--foreground-source",
            "sway",
        ])
        .unwrap();

        assert!(config.focus.auto_focus);
        assert_eq!(config.focus.focus_source, FocusSource::Foreground);
        assert!(config.focus.foreground_window);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
    }

    #[test]
    fn foreground_include_title_requires_foreground_window_or_auto_focus_foreground() {
        let err =
            parse_monitor_config_for_phase15(["stutter", "monitor", "--foreground-include-title"])
                .unwrap_err()
                .to_string();

        assert!(err.contains(
            "--foreground-include-title requires --foreground-window or --auto-focus with --focus-source foreground or hybrid"
        ));

        let foreground_window = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--foreground-window",
            "--foreground-include-title",
        ])
        .unwrap();
        assert!(foreground_window.focus.foreground_window);
        assert!(foreground_window.focus.foreground_include_title);

        let foreground_focus = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--auto-focus",
            "--focus-source",
            "foreground",
            "--foreground-include-title",
        ])
        .unwrap();
        assert!(foreground_focus.focus.auto_focus);
        assert_eq!(foreground_focus.focus.focus_source, FocusSource::Foreground);
        assert!(foreground_focus.focus.foreground_window);
        assert!(foreground_focus.focus.foreground_include_title);
    }

    #[test]
    fn config_file_sets_summary_when_cli_omitted() {
        let args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        let file_config = crate::config_file::UserConfigFile {
            summary_ms: Some(500),
            ..Default::default()
        };

        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();

        assert_eq!(config.timing.summary_period_ms, 500);
    }

    #[test]
    fn cli_summary_overrides_config_file_summary() {
        let args = MonitorArgs {
            summary_period_ms: Some(200),
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        let file_config = crate::config_file::UserConfigFile {
            summary_ms: Some(500),
            ..Default::default()
        };

        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();

        assert_eq!(config.timing.summary_period_ms, 200);
    }

    #[test]
    fn include_comm_from_config_used_when_cli_omitted() {
        let args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        let file_config = crate::config_file::UserConfigFile {
            include_comm: Some(vec!["Game".to_owned(), "Render".to_owned()]),
            ..Default::default()
        };

        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();

        assert_eq!(
            config.target.include_comm,
            vec!["Game".to_owned(), "Render".to_owned()]
        );
    }

    #[test]
    fn cli_include_comm_overrides_config_list() {
        let args = MonitorArgs {
            include_comm: vec!["RenderThread".to_owned()],
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        let file_config = crate::config_file::UserConfigFile {
            include_comm: Some(vec!["Game".to_owned()]),
            ..Default::default()
        };

        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();

        assert_eq!(config.target.include_comm, vec!["RenderThread".to_owned()]);
    }

    #[test]
    fn exclude_comm_from_config_used_when_cli_omitted() {
        let args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        let file_config = crate::config_file::UserConfigFile {
            exclude_comm: Some(vec!["steamwebhelper".to_owned(), "browser".to_owned()]),
            ..Default::default()
        };

        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();

        assert_eq!(
            config.target.exclude_comm,
            vec!["browser".to_owned(), "steamwebhelper".to_owned()]
        );
    }

    #[test]
    fn cli_exclude_comm_overrides_config_list() {
        let args = MonitorArgs {
            exclude_comm: vec!["steamwebhelper".to_owned()],
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        let file_config = crate::config_file::UserConfigFile {
            exclude_comm: Some(vec!["browser".to_owned()]),
            ..Default::default()
        };

        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();

        assert_eq!(
            config.target.exclude_comm,
            vec!["steamwebhelper".to_owned()]
        );
    }

    #[test]
    fn file_focus_source_used_when_cli_omitted() {
        let args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        let file_config = crate::config_file::UserConfigFile {
            focus_source: Some("foreground".to_owned()),
            foreground_source: Some("sway".to_owned()),
            ..Default::default()
        };

        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();

        assert_eq!(config.focus.focus_source, FocusSource::Foreground);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
        assert!(config.focus.foreground_window);
    }

    #[test]
    fn cli_focus_source_overrides_config_file_focus_source() {
        let cli = Cli::try_parse_from([
            "stutter",
            "monitor",
            "--focus-source",
            "hybrid",
            "--foreground-source",
            "hyprland",
        ])
        .unwrap();
        let matches = Cli::command().get_matches_from([
            "stutter",
            "monitor",
            "--focus-source",
            "hybrid",
            "--foreground-source",
            "hyprland",
        ]);
        let presence = monitor_arg_presence_from_matches(&matches, Some("monitor"));

        let Some(Command::Monitor(args)) = cli.command else {
            panic!("expected monitor command");
        };

        let file_config = crate::config_file::UserConfigFile {
            focus_source: Some("foreground".to_owned()),
            foreground_source: Some("sway".to_owned()),
            ..Default::default()
        };

        let config = monitor_config_from_monitor_args_with_file_and_presence(
            args,
            Some(file_config),
            RecordingMode::Monitor,
            presence,
        )
        .unwrap();

        assert_eq!(config.focus.focus_source, FocusSource::Hybrid);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Hyprland);
        assert!(config.focus.foreground_window);
    }
}

#[cfg(test)]
mod monitor_record_check_performance_cli_tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::commands::input::AppCommand;

    fn parse_cli_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        crate::cli::parse_app_command_from(args)
    }

    #[test]
    fn rejects_zero_duration_record() {
        let err =
            parse_cli_command(["stutter", "record", "--pid", "42", "--duration", "0"]).unwrap_err();

        assert!(
            err.to_string()
                .contains("--duration must be greater than zero")
        );
    }

    #[test]
    fn record_command_forces_recording_mode_and_duration() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "record",
            "--pid",
            "42",
            "--duration",
            "5",
        ])
        .unwrap();

        assert_eq!(config.timing.max_duration, Some(Duration::from_secs(5)));
        assert_eq!(config.recording.run_name.as_deref(), Some("record"));
        assert!(config.probes.cpu_freq);
    }

    #[test]
    fn record_rejects_no_record_flag() {
        let err =
            parse_cli_command(["stutter", "record", "--pid", "42", "--no-record"]).unwrap_err();

        assert!(
            err.to_string()
                .contains("record --no-record is contradictory")
        );
    }

    #[test]
    fn parses_cpu_perf_monitor_flags() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--cpu-perf",
            "--cpu-perf-kernel",
            "--cpu-perf-max-tasks",
            "16",
        ])
        .unwrap();

        assert!(config.probes.cpu_perf);
        assert!(config.cpu_perf.include_kernel);
        assert_eq!(config.cpu_perf.max_tasks, 16);
        assert!(!config.cpu_perf.collect_cache_refs);
    }

    #[test]
    fn parses_cpu_perf_cache_refs_for_recording() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "record",
            "--pid",
            "42",
            "--cpu-perf",
            "--cpu-perf-cache-refs",
        ])
        .unwrap();

        assert!(config.probes.cpu_perf);
        assert!(config.cpu_perf.collect_cache_refs);
        assert!(config.recording.output_dir.is_some() || config.recording.run_name.is_some());
    }

    #[test]
    fn rejects_zero_cpu_perf_max_tasks() {
        let err = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--cpu-perf-max-tasks",
            "0",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--cpu-perf-max-tasks must be greater than zero")
        );
    }

    #[test]
    fn parses_runtime_slices_monitor_flags() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--runtime-slices",
            "--runtime-slices-max-tasks",
            "64",
        ])
        .unwrap();

        assert!(config.probes.runtime_slices);
        assert_eq!(config.runtime_slices.max_tasks, 64);
    }

    #[test]
    fn rejects_zero_runtime_slices_max_tasks() {
        let err = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--runtime-slices-max-tasks",
            "0",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--runtime-slices-max-tasks must be greater than zero")
        );
    }

    #[test]
    fn parses_ringbuf_size_and_wakeup_map_factor_flags() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--ringbuf-size-kb",
            "1000",
            "--wakeup-map-factor",
            "4",
        ])
        .unwrap();

        assert_eq!(config.ebpf_sizing.ringbuf_size_kb, Some(1000));
        assert_eq!(config.ebpf_sizing.wakeup_map_factor, Some(4));
    }

    #[test]
    fn rejects_invalid_ringbuf_size_bounds() {
        for value in ["63", "16385"] {
            let err = parse_monitor_config_for_phase15([
                "stutter",
                "monitor",
                "--pid",
                "42",
                "--ringbuf-size-kb",
                value,
            ])
            .unwrap_err();

            assert!(
                err.to_string()
                    .contains("--ringbuf-size-kb must be between 64 and 16384"),
                "expected ringbuf bound rejection for {value}, got {err:#}"
            );
        }
    }

    #[test]
    fn rejects_invalid_wakeup_map_factor_bounds() {
        for value in ["0", "65"] {
            let err = parse_monitor_config_for_phase15([
                "stutter",
                "monitor",
                "--pid",
                "42",
                "--wakeup-map-factor",
                value,
            ])
            .unwrap_err();

            assert!(
                err.to_string()
                    .contains("--wakeup-map-factor must be between 1 and 64"),
                "expected wakeup map factor rejection for {value}, got {err:#}"
            );
        }
    }

    #[test]
    fn parses_extended_check_command() {
        let command = parse_cli_command([
            "stutter",
            "check",
            "--baseline",
            "/tmp/base",
            "--current",
            "/tmp/current",
            "--max-regression-p99-ms",
            "0.5",
            "--max-max-regression-ms",
            "2.0",
            "--json",
            "--top",
            "5",
            "--filter-class",
            "Game",
        ])
        .unwrap();

        let AppCommand::Check(input) = command else {
            panic!("expected check command");
        };

        assert_eq!(input.baseline, PathBuf::from("/tmp/base"));
        assert_eq!(input.current, PathBuf::from("/tmp/current"));
        assert_eq!(input.max_regression_p99_ms, Some(0.5));
        assert_eq!(input.max_max_regression_ms, Some(2.0));
        assert!(input.json);
        assert_eq!(input.top, 5);
        assert_eq!(
            input.filter_class,
            Some(crate::process_tree::TaskClass::Game)
        );
    }

    #[test]
    fn rejects_check_without_thresholds() {
        let err = parse_cli_command([
            "stutter",
            "check",
            "--baseline",
            "/tmp/base",
            "--current",
            "/tmp/current",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("check requires at least one threshold")
        );
    }

    #[test]
    fn rejects_invalid_regression_threshold() {
        for value in ["NaN", "inf", "-1.0"] {
            let flag = format!("--max-regression-p99-ms={value}");
            let err = parse_cli_command([
                "stutter",
                "check",
                "--baseline",
                "run1/",
                "--current",
                "run2/",
                &flag,
            ])
            .unwrap_err();

            assert!(
                err.to_string()
                    .contains("--max-regression-p99-ms must be a finite non-negative value"),
                "expected p99 threshold rejection for {value}, got {err:#}"
            );
        }
    }

    #[test]
    fn rejects_zero_check_top() {
        let err = parse_cli_command([
            "stutter",
            "check",
            "--baseline",
            "/tmp/base",
            "--current",
            "/tmp/current",
            "--max-regression-p99-ms",
            "0.5",
            "--top",
            "0",
        ])
        .unwrap_err();

        assert!(err.to_string().contains("--top must be greater than zero"));
    }

    #[test]
    fn parses_recording_retention_flags() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "record",
            "--pid",
            "42",
            "--retention-max-runs",
            "7",
            "--retention-max-bytes",
            "2000000",
            "--retention-max-age-seconds",
            "86400",
            "--retention-min-free-bytes",
            "1000000",
        ])
        .unwrap();

        assert_eq!(config.recording.retention.max_run_count, Some(7));
        assert_eq!(config.recording.retention.max_total_bytes, Some(2_000_000));
        assert_eq!(config.recording.retention.max_age_seconds, Some(86_400));
        assert_eq!(config.recording.retention.min_free_bytes, Some(1_000_000));
    }

    #[test]
    fn rejects_zero_recording_retention_flags() {
        for flag in [
            "--retention-max-runs",
            "--retention-max-bytes",
            "--retention-max-age-seconds",
            "--retention-min-free-bytes",
        ] {
            let err =
                parse_monitor_config_for_phase15(["stutter", "record", "--pid", "42", flag, "0"])
                    .unwrap_err();

            assert!(
                err.to_string().contains("must be greater than zero"),
                "expected zero value rejection for {flag}, got {err:#}"
            );
        }
    }

    #[test]
    fn parses_mangohud_log_flags_for_recording() {
        let config = parse_monitor_config_for_phase15([
            "stutter",
            "record",
            "--pid",
            "42",
            "--mangohud-log",
            "/tmp/mango.csv",
            "--mangohud-log-live",
        ])
        .unwrap();

        assert_eq!(config.mangohud.log, Some(PathBuf::from("/tmp/mango.csv")));
        assert!(config.mangohud.log_live);
    }

    #[test]
    fn mangohud_log_live_requires_mangohud_log() {
        let err = parse_cli_command(["stutter", "record", "--pid", "42", "--mangohud-log-live"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("required arguments were not provided")
                || err.to_string().contains("--mangohud-log")
        );
    }
}

#[cfg(test)]
mod auto_focus_cli_tests {
    use super::*;

    #[test]
    fn monitor_args_default_contains_auto_focus_defaults() {
        let args = MonitorArgs::default();

        assert!(!args.auto_focus);
        assert_eq!(args.auto_focus_poll_ms, 1000);
        assert_eq!(args.auto_focus_min_confidence, 0.60);
        assert_eq!(args.auto_focus_switch_cooldown_ms, 5000);
        assert_eq!(args.auto_focus_switch_margin, 0.20);
        assert_eq!(args.auto_focus_required_polls, 2);
        assert_eq!(args.auto_focus_max_roots, 4);
    }

    #[test]
    fn monitor_cli_parses_auto_focus_fields() {
        let cli = Cli::parse_from([
            "stutter",
            "monitor",
            "--auto-focus",
            "--auto-focus-poll-ms",
            "250",
            "--auto-focus-min-confidence",
            "0.75",
            "--auto-focus-switch-cooldown-ms",
            "7500",
            "--auto-focus-switch-margin",
            "0.30",
            "--auto-focus-required-polls",
            "3",
            "--auto-focus-max-roots",
            "2",
        ]);

        let Command::Monitor(args) = cli.command.unwrap() else {
            panic!("expected monitor command");
        };

        assert!(args.auto_focus);
        assert_eq!(args.auto_focus_poll_ms, 250);
        assert_eq!(args.auto_focus_min_confidence, 0.75);
        assert_eq!(args.auto_focus_switch_cooldown_ms, 7500);
        assert_eq!(args.auto_focus_switch_margin, 0.30);
        assert_eq!(args.auto_focus_required_polls, 3);
        assert_eq!(args.auto_focus_max_roots, 2);
    }
}
