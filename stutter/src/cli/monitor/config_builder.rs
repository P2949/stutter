//! Monitor config construction split from the parent CLI module to keep the argument surface below the architecture size gate.

use super::{
    foreground_validation::{
        normalize_foreground_monitor_args, validate_foreground_monitor_args,
        validate_foreground_title_monitor_args,
    },
    *,
};

impl FocusSource {
    fn parse_config_value(value: &str) -> anyhow::Result<Self> {
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
    fn parse_config_value(value: &str) -> anyhow::Result<Self> {
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

pub(crate) fn monitor_arg_presence_from_matches(
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

pub(crate) fn monitor_config_from_monitor_args(
    args: MonitorArgs,
    recording_mode: RecordingMode,
) -> anyhow::Result<MonitorConfig> {
    let file_config = crate::config_file::load_user_config()?;
    monitor_config_from_monitor_args_with_file(args, file_config, recording_mode)
}

pub(crate) fn monitor_config_from_monitor_args_with_presence(
    args: MonitorArgs,
    recording_mode: RecordingMode,
    cli_presence: MonitorArgPresence,
) -> anyhow::Result<MonitorConfig> {
    if cli_presence == MonitorArgPresence::default() {
        return monitor_config_from_monitor_args(args, recording_mode);
    }

    let file_config = crate::config_file::load_user_config()?;
    monitor_config_from_monitor_args_with_file_and_presence(
        args,
        file_config,
        recording_mode,
        cli_presence,
    )
}

pub(crate) fn monitor_config_from_monitor_args_with_file(
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

pub(crate) fn monitor_config_from_monitor_args_with_file_and_presence(
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
    if !cli_presence.live_diagnosis_cluster_window_ms
        && let Some(window_ms) = file_config.live_diagnosis_cluster_window_ms
    {
        args.live_diagnosis_cluster_window_ms = Some(window_ms);
    }

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

    if let Some(kb) = args.ringbuf_size_kb
        && !(64..=16 * 1024).contains(&kb)
    {
        anyhow::bail!("--ringbuf-size-kb must be between 64 and 16384");
    }

    if let Some(factor) = args.wakeup_map_factor
        && (factor == 0 || factor > 64)
    {
        anyhow::bail!("--wakeup-map-factor must be between 1 and 64");
    }

    if args.otlp_endpoint.is_some() && !cfg!(feature = "otel") {
        anyhow::bail!("OpenTelemetry support was not compiled in. Rebuild with --features otel.");
    }

    if args.otel_service_name.trim().is_empty() {
        anyhow::bail!("--otel-service-name must not be empty");
    }

    if let Some(endpoint) = &args.otlp_endpoint
        && endpoint.trim().is_empty()
    {
        anyhow::bail!("--otlp-endpoint must not be empty");
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
    if matches!(args.live_diagnosis_cluster_window_ms, Some(0)) {
        anyhow::bail!("--live-diagnosis-cluster-window-ms must be greater than zero");
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
    if matches!(args.kms_card.as_deref(), Some("")) {
        anyhow::bail!("--kms-card must not be empty");
    }
    if matches!(args.kms_connector.as_deref(), Some("")) {
        anyhow::bail!("--kms-connector must not be empty");
    }
    if matches!(args.drm_fence_render_card.as_deref(), Some("")) {
        anyhow::bail!("--drm-fence-render-card must not be empty");
    }
    if matches!(args.drm_fence_display_card.as_deref(), Some("")) {
        anyhow::bail!("--drm-fence-display-card must not be empty");
    }
    if let Some(driver) = args.drm_fence_driver.as_deref() {
        if driver.trim().is_empty() {
            anyhow::bail!("--drm-fence-driver must not be empty");
        }
        if !matches!(driver, "amdgpu" | "i915" | "auto") {
            anyhow::bail!("--drm-fence-driver must be one of: amdgpu, i915, auto");
        }
    }
    if matches!(args.wayland_presentation_log.as_deref(), Some(path) if path.as_os_str().is_empty())
    {
        anyhow::bail!("--wayland-presentation-log must not be empty");
    }
    if matches!(args.dmabuf_log.as_deref(), Some(path) if path.as_os_str().is_empty()) {
        anyhow::bail!("--dmabuf-log must not be empty");
    }
    for (flag, value) in [
        ("--display-path-label", args.display_path_label.as_deref()),
        ("--display-render-gpu", args.display_render_gpu.as_deref()),
        ("--display-scanout-gpu", args.display_scanout_gpu.as_deref()),
        ("--display-connector", args.display_connector.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            anyhow::bail!("{flag} must not be empty");
        }
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
