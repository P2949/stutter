use std::{path::PathBuf, sync::Arc, time::Duration};

use clap::Args;
use serde::Serialize;

use crate::{
    cli::RecordingMode,
    config::{
        CsvStreamTarget, FocusSource, ForegroundSource, TARGET_PIDS_MAX,
        effective::resolve_monitor_config_sources,
        layer::MonitorConfigLayer,
        merge::{CliOverrides, ConfigSources, DefaultConfig, PresetConfig},
        model::MonitorConfig,
    },
};

#[derive(Args, Clone, Debug, Default, Serialize)]
pub struct MonitorArgs {
    #[arg(short, long)]
    pub pid: Vec<u32>,

    #[arg(short, long = "tree-pid")]
    pub tree_pid: Vec<u32>,

    #[arg(long = "exclude-tree-pid")]
    pub exclude_tree_pid: Vec<u32>,

    #[arg(long = "include-comm")]
    pub include_comm: Vec<String>,

    #[arg(long = "exclude-comm")]
    pub exclude_comm: Vec<String>,

    #[arg(short, long = "summary-ms")]
    pub summary_period_ms: Option<u64>,

    #[arg(long = "epoch")]
    pub epoch_period_ms: Option<u64>,

    #[arg(short, long = "spike-us")]
    pub spike_threshold_us: Option<u64>,

    #[arg(long = "alert-threshold-ms")]
    pub alert_threshold_ms: Option<u64>,

    #[arg(long = "alert-webhook-url")]
    pub alert_webhook_url: Option<String>,

    #[arg(long)]
    pub hwmon: bool,

    #[arg(long)]
    pub no_hwmon: bool,

    #[arg(long = "hwmon-root")]
    pub hwmon_root: Option<PathBuf>,

    #[arg(long = "hwmon-drm-card")]
    pub hwmon_drm_card: Option<String>,

    #[arg(long = "hwmon-render-node")]
    pub hwmon_render_node: Option<PathBuf>,

    #[arg(long)]
    pub cpu_freq: bool,

    #[arg(long)]
    pub no_cpu_freq: bool,

    #[arg(long)]
    pub faults: bool,

    #[arg(long)]
    pub no_faults: bool,

    #[arg(long)]
    pub stat_wait: bool,

    #[arg(long)]
    pub no_stat_wait: bool,

    #[arg(long)]
    pub block_io: bool,

    #[arg(long)]
    pub no_block_io: bool,

    #[arg(long)]
    pub runtime_slices: bool,

    #[arg(long)]
    pub no_runtime_slices: bool,

    #[arg(long)]
    pub irq_latency: bool,

    #[arg(long = "irq")]
    pub irqs: Vec<u32>,

    #[arg(long)]
    pub cpu_perf: bool,

    #[arg(long)]
    pub cpu_perf_kernel: bool,

    #[arg(long)]
    pub cpu_perf_cache_refs: bool,

    #[arg(long, default_value = "1024")]
    pub cpu_perf_max_tasks: u32,

    #[arg(long, default_value = "1024")]
    pub runtime_slices_max_tasks: u32,

    #[arg(long)]
    pub ringbuf_size_kb: Option<u32>,

    #[arg(long)]
    pub wakeup_map_factor: Option<u32>,

    #[arg(long)]
    pub otlp_endpoint: Option<String>,

    #[arg(long, default_value = "stutter")]
    pub otel_service_name: String,

    #[arg(long)]
    pub csv: Option<PathBuf>,

    #[arg(long)]
    pub stream_csv: Option<String>,

    #[arg(long)]
    pub json_stream: bool,

    #[arg(long)]
    pub tui: bool,

    #[arg(long, default_value = "1024")]
    pub max_tasks: Option<u32>,

    #[arg(long)]
    pub watch_process: Option<String>,

    #[arg(long, default_value = "1000")]
    pub watch_poll_ms: u64,

    #[arg(long)]
    pub watch_timeout_seconds: Option<u64>,

    #[arg(long)]
    pub persistent: bool,

    #[arg(long)]
    pub keep_missing_pid: bool,

    #[arg(long)]
    pub follow_exec: bool,

    #[arg(long)]
    pub no_follow_exec: bool,

    #[arg(long)]
    pub cgroupv2: Option<PathBuf>,

    #[arg(long)]
    pub native_cgroup_filter: bool,

    #[arg(long)]
    pub run_name: Option<String>,

    #[arg(short, long)]
    pub out_dir: Option<PathBuf>,

    #[arg(long)]
    pub no_record: bool,

    #[arg(long)]
    pub retention_max_runs: Option<u32>,

    #[arg(long)]
    pub retention_max_bytes: Option<u64>,

    #[arg(long)]
    pub retention_max_age_seconds: Option<u64>,

    #[arg(long)]
    pub retention_min_free_bytes: Option<u64>,

    #[arg(long)]
    pub preset: Option<String>,

    #[arg(long)]
    pub mangohud_log: Option<PathBuf>,

    #[arg(long)]
    pub auto_focus: bool,

    #[arg(long, default_value = "1000")]
    pub auto_focus_poll_ms: u64,

    #[arg(long, default_value = "0.60")]
    pub auto_focus_min_confidence: f64,

    #[arg(long, default_value = "5000")]
    pub auto_focus_switch_cooldown_ms: u64,

    #[arg(long, default_value = "0.20")]
    pub auto_focus_switch_margin: f64,

    #[arg(long, default_value = "2")]
    pub auto_focus_required_polls: u32,

    #[arg(long, default_value = "4")]
    pub auto_focus_max_roots: u32,

    #[arg(long)]
    pub foreground_window: bool,

    #[arg(long, default_value = "auto")]
    pub foreground_source: ForegroundSource,

    #[arg(long, default_value = "1000")]
    pub foreground_poll_ms: u64,

    #[arg(long, default_value = "2500")]
    pub foreground_max_stale_ms: u64,

    #[arg(long)]
    pub foreground_include_title: bool,

    #[arg(long, default_value = "heuristic")]
    pub focus_source: FocusSource,
}

#[derive(Clone, Debug, Default)]
pub struct MonitorArgPresence {
    pub summary_period_ms: bool,
    pub spike_threshold_us: bool,
    pub max_tasks: bool,
    pub follow_exec: bool,
    pub no_follow_exec: bool,
    pub focus_source: bool,
    pub foreground_source: bool,
    pub foreground_poll_ms: bool,
    pub foreground_max_stale_ms: bool,
}

pub fn monitor_arg_presence_from_matches(
    matches: &clap::ArgMatches,
    subcommand: Option<&str>,
) -> MonitorArgPresence {
    let matches = subcommand
        .and_then(|s| matches.subcommand_matches(s))
        .unwrap_or(matches);

    MonitorArgPresence {
        summary_period_ms: matches.get_one::<u64>("summary_period_ms").is_some(),
        spike_threshold_us: matches.get_one::<u64>("spike_threshold_us").is_some(),
        max_tasks: matches.get_one::<u32>("max_tasks").is_some(),
        follow_exec: matches.get_flag("follow_exec"),
        no_follow_exec: matches.get_flag("no_follow_exec"),
        focus_source: matches.get_one::<FocusSource>("focus_source").is_some(),
        foreground_source: matches
            .get_one::<ForegroundSource>("foreground_source")
            .is_some(),
        foreground_poll_ms: matches.get_one::<u64>("foreground_poll_ms").is_some(),
        foreground_max_stale_ms: matches.get_one::<u64>("foreground_max_stale_ms").is_some(),
    }
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

    validate_pids("--pid", &args.pid)?;
    validate_pids("--tree-pid", &args.tree_pid)?;
    validate_pids("--exclude-tree-pid", &args.exclude_tree_pid)?;

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
    if matches!(args.retention_max_runs, Some(0)) {
        anyhow::bail!("--retention-max-runs must be greater than zero");
    }
    if matches!(args.retention_max_bytes, Some(0)) {
        anyhow::bail!("--retention-max-bytes must be greater than zero");
    }
    if matches!(args.retention_max_age_seconds, Some(0)) {
        anyhow::bail!("--retention-max-age-seconds must be greater than zero");
    }
    if matches!(args.retention_min_free_bytes, Some(0)) {
        anyhow::bail!("--retention-min-free-bytes must be greater than zero");
    }

    args.pid.sort_unstable();
    args.pid.dedup();
    args.tree_pid.sort_unstable();
    args.tree_pid.dedup();
    args.exclude_tree_pid.sort_unstable();
    args.exclude_tree_pid.dedup();
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

    if args.pid.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many unique target PIDs: got {}, but TARGET_PIDS supports at most {}",
            args.pid.len(),
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

    match (&args.csv, &args.stream_csv) {
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

pub fn validate_pids(flag: &str, pids: &[u32]) -> anyhow::Result<()> {
    if pids.contains(&0) {
        anyhow::bail!("{flag} must be greater than zero");
    }
    Ok(())
}

pub fn validate_comm_patterns(flag: &str, patterns: &[String]) -> anyhow::Result<()> {
    for pattern in patterns {
        if pattern.is_empty() {
            anyhow::bail!("{flag} patterns must not be empty");
        }
    }
    Ok(())
}

pub fn normalize_foreground_monitor_args(args: &mut MonitorArgs) {
    if args.focus_source != FocusSource::Heuristic {
        args.foreground_window = true;
    }
}

pub fn validate_foreground_title_monitor_args(args: &MonitorArgs) -> anyhow::Result<()> {
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

pub fn validate_foreground_monitor_args(args: &MonitorArgs) -> anyhow::Result<()> {
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

pub fn merge_bool(
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

impl MonitorArgs {
    pub fn into_monitor_config_layer(self, cli_presence: MonitorArgPresence) -> MonitorConfigLayer {
        let mut layer = MonitorConfigLayer::default();

        if !self.pid.is_empty() {
            layer.target_pids = Some(self.pid);
        }
        if !self.tree_pid.is_empty() {
            layer.tree_pids = Some(self.tree_pid);
        }
        if !self.exclude_tree_pid.is_empty() {
            layer.exclude_tree_pids = Some(self.exclude_tree_pid);
        }
        if !self.include_comm.is_empty() {
            layer.include_comm = Some(self.include_comm);
        }
        if !self.exclude_comm.is_empty() {
            layer.exclude_comm = Some(self.exclude_comm);
        }

        if cli_presence.summary_period_ms {
            layer.summary_period_ms = self.summary_period_ms;
        }
        if cli_presence.spike_threshold_us {
            layer.spike_threshold_ns = self.spike_threshold_us.map(|us| us * 1000);
        }
        if cli_presence.max_tasks {
            layer.max_tasks = self.max_tasks;
        }

        if self.hwmon {
            layer.hwmon = Some(true);
        }
        if self.no_hwmon {
            layer.hwmon = Some(false);
        }
        layer.hwmon_root = self.hwmon_root;
        layer.hwmon_drm_card = self.hwmon_drm_card;
        layer.hwmon_render_node = self.hwmon_render_node;

        if self.cpu_freq {
            layer.cpu_freq = Some(true);
        }
        if self.no_cpu_freq {
            layer.cpu_freq = Some(false);
        }

        if self.faults {
            layer.faults = Some(true);
        }
        if self.no_faults {
            layer.faults = Some(false);
        }

        if self.stat_wait {
            layer.stat_wait = Some(true);
        }
        if self.no_stat_wait {
            layer.stat_wait = Some(false);
        }

        if self.block_io {
            layer.block_io = Some(true);
        }
        if self.no_block_io {
            layer.block_io = Some(false);
        }

        if self.runtime_slices {
            layer.runtime_slices = Some(true);
        }
        if self.no_runtime_slices {
            layer.runtime_slices = Some(false);
        }

        if self.irq_latency {
            layer.irq_latency = Some(true);
        }
        if !self.irqs.is_empty() {
            layer.irqs = Some(self.irqs);
        }

        if self.cpu_perf {
            layer.cpu_perf = Some(true);
        }
        layer.cpu_perf_kernel = Some(self.cpu_perf_kernel);
        layer.cpu_perf_cache_refs = Some(self.cpu_perf_cache_refs);
        layer.cpu_perf_max_tasks = Some(self.cpu_perf_max_tasks);

        layer.runtime_slices_max_tasks = Some(self.runtime_slices_max_tasks);

        layer.ringbuf_size_kb = self.ringbuf_size_kb;
        layer.wakeup_map_factor = self.wakeup_map_factor;

        layer.otlp_endpoint = self.otlp_endpoint;
        layer.otel_service_name = Some(self.otel_service_name);

        if let Some(path) = self.csv {
            layer.csv_path = Some(Some(path));
        }
        if let Some(stream) = self.stream_csv {
            layer.stream_csv = Some(Some(stream));
        }

        if self.json_stream {
            layer.json_stream = Some(true);
        }

        if self.tui {
            layer.tui = Some(true);
        }

        layer.watch_process = self.watch_process;
        layer.watch_poll_ms = Some(self.watch_poll_ms);
        layer.watch_timeout_seconds = self.watch_timeout_seconds;
        layer.persistent = Some(self.persistent);
        layer.keep_missing_pid = Some(self.keep_missing_pid);

        if cli_presence.follow_exec {
            layer.follow_exec = Some(true);
        }
        if cli_presence.no_follow_exec {
            layer.follow_exec = Some(false);
        }

        layer.cgroupv2 = self.cgroupv2;
        layer.native_cgroup_filter = Some(self.native_cgroup_filter);

        layer.run_name = self.run_name.map(Some);
        layer.output_dir = self.out_dir.map(Some);

        layer.retention_max_run_count = self.retention_max_runs;
        layer.retention_max_total_bytes = self.retention_max_bytes;
        layer.retention_max_age_seconds = self.retention_max_age_seconds;
        layer.retention_min_free_bytes = self.retention_min_free_bytes;

        layer.mangohud_log = self.mangohud_log;

        layer.auto_focus = Some(self.auto_focus);
        layer.auto_focus_poll_ms = Some(self.auto_focus_poll_ms);
        layer.auto_focus_min_confidence = Some(self.auto_focus_min_confidence);
        layer.auto_focus_switch_cooldown_ms = Some(self.auto_focus_switch_cooldown_ms);
        layer.auto_focus_switch_margin = Some(self.auto_focus_switch_margin);
        layer.auto_focus_required_polls = Some(self.auto_focus_required_polls);
        layer.auto_focus_max_roots = Some(self.auto_focus_max_roots);

        layer.foreground_window = Some(self.foreground_window);
        if cli_presence.foreground_source {
            layer.foreground_source = Some(self.foreground_source);
        }
        if cli_presence.foreground_poll_ms {
            layer.foreground_poll_ms = Some(self.foreground_poll_ms);
        }
        if cli_presence.foreground_max_stale_ms {
            layer.foreground_max_stale_ms = Some(self.foreground_max_stale_ms);
        }
        layer.foreground_include_title = Some(self.foreground_include_title);

        if cli_presence.focus_source {
            layer.focus_source = Some(self.focus_source);
        }

        layer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::FocusSource;

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
}
