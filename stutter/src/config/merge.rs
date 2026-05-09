use super::model::{
    FocusConfig, MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, SafetyConfig,
    TargetConfig, TimingConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Default,
    UserFile,
    Preset,
    Cli,
}

#[derive(Debug, Clone, Default)]
pub struct DefaultConfig {
    pub config: MonitorConfig,
}

#[derive(Debug, Clone, Default)]
pub struct PresetConfig {
    pub config: MonitorConfig,
}

#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub config: MonitorConfig,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigSources {
    pub defaults: DefaultConfig,
    pub user_file: Option<crate::config_file::UserConfigFile>,
    pub preset: Option<PresetConfig>,
    pub cli: CliOverrides,
}

pub fn merge_config_sources(sources: ConfigSources) -> MonitorConfig {
    let mut config = sources.defaults.config;

    if let Some(user_file) = sources.user_file.as_ref() {
        config = apply_user_file(config, user_file);
    }

    if let Some(preset) = sources.preset {
        config = merge_monitor_config(config, preset.config);
    }

    merge_monitor_config(config, sources.cli.config)
}

pub fn merge_user_file(config: MonitorConfig) -> MonitorConfig {
    match crate::config_file::load_user_config() {
        Ok(Some(user_file)) => apply_user_file(config, &user_file),
        Ok(None) => config,
        Err(err) => {
            log::warn!("failed_to_load_user_config_for_monitor_config err={err:#}");
            config
        }
    }
}

pub fn merge_monitor_config(base: MonitorConfig, override_config: MonitorConfig) -> MonitorConfig {
    let default = MonitorConfig::default();

    MonitorConfig {
        target: merge_target_config(base.target, override_config.target, default.target),
        timing: merge_timing_config(base.timing, override_config.timing, default.timing),
        probes: merge_probe_config(base.probes, override_config.probes, default.probes),
        recording: merge_recording_config(
            base.recording,
            override_config.recording,
            default.recording,
        ),
        outputs: merge_output_config(base.outputs, override_config.outputs, default.outputs),
        focus: merge_focus_config(base.focus, override_config.focus, default.focus),
        safety: merge_safety_config(base.safety, override_config.safety, default.safety),
    }
}

fn override_field<T: PartialEq>(base: T, override_value: T, default_value: T) -> T {
    if override_value != default_value {
        override_value
    } else {
        base
    }
}

fn merge_target_config(
    base: TargetConfig,
    override_config: TargetConfig,
    default: TargetConfig,
) -> TargetConfig {
    TargetConfig {
        target_pids: override_field(
            base.target_pids,
            override_config.target_pids,
            default.target_pids,
        ),
        tree_pids: override_field(base.tree_pids, override_config.tree_pids, default.tree_pids),
        cgroupv2: override_field(base.cgroupv2, override_config.cgroupv2, default.cgroupv2),
        exclude_tree_pids: override_field(
            base.exclude_tree_pids,
            override_config.exclude_tree_pids,
            default.exclude_tree_pids,
        ),
        include_comm: override_field(
            base.include_comm,
            override_config.include_comm,
            default.include_comm,
        ),
        exclude_comm: override_field(
            base.exclude_comm,
            override_config.exclude_comm,
            default.exclude_comm,
        ),
        watch_process: override_field(
            base.watch_process,
            override_config.watch_process,
            default.watch_process,
        ),
        persistent: override_field(base.persistent, override_config.persistent, default.persistent),
        keep_missing_pid: override_field(
            base.keep_missing_pid,
            override_config.keep_missing_pid,
            default.keep_missing_pid,
        ),
        max_tasks: override_field(base.max_tasks, override_config.max_tasks, default.max_tasks),
    }
}

fn merge_timing_config(
    base: TimingConfig,
    override_config: TimingConfig,
    default: TimingConfig,
) -> TimingConfig {
    TimingConfig {
        summary_period_ms: override_field(
            base.summary_period_ms,
            override_config.summary_period_ms,
            default.summary_period_ms,
        ),
        epoch_period_ms: override_field(
            base.epoch_period_ms,
            override_config.epoch_period_ms,
            default.epoch_period_ms,
        ),
        max_duration: override_field(
            base.max_duration,
            override_config.max_duration,
            default.max_duration,
        ),
        spike_threshold_ns: override_field(
            base.spike_threshold_ns,
            override_config.spike_threshold_ns,
            default.spike_threshold_ns,
        ),
    }
}

fn merge_probe_config(
    base: ProbeConfig,
    override_config: ProbeConfig,
    default: ProbeConfig,
) -> ProbeConfig {
    ProbeConfig {
        irq_latency: override_field(
            base.irq_latency,
            override_config.irq_latency,
            default.irq_latency,
        ),
        irqs: override_field(base.irqs, override_config.irqs, default.irqs),
        hwmon: override_field(base.hwmon, override_config.hwmon, default.hwmon),
        cpu_freq: override_field(base.cpu_freq, override_config.cpu_freq, default.cpu_freq),
        faults: override_field(base.faults, override_config.faults, default.faults),
        cpu_perf: override_field(base.cpu_perf, override_config.cpu_perf, default.cpu_perf),
        block_io: override_field(base.block_io, override_config.block_io, default.block_io),
        stat_wait: override_field(base.stat_wait, override_config.stat_wait, default.stat_wait),
        runtime_slices: override_field(
            base.runtime_slices,
            override_config.runtime_slices,
            default.runtime_slices,
        ),
    }
}

fn merge_recording_config(
    base: RecordingConfig,
    override_config: RecordingConfig,
    default: RecordingConfig,
) -> RecordingConfig {
    RecordingConfig {
        run_name: override_field(base.run_name, override_config.run_name, default.run_name),
        output_dir: override_field(
            base.output_dir,
            override_config.output_dir,
            default.output_dir,
        ),
        retain_intervals: override_field(
            base.retain_intervals,
            override_config.retain_intervals,
            default.retain_intervals,
        ),
    }
}

fn merge_output_config(
    base: OutputConfig,
    override_config: OutputConfig,
    default: OutputConfig,
) -> OutputConfig {
    OutputConfig {
        json_stream: override_field(
            base.json_stream,
            override_config.json_stream,
            default.json_stream,
        ),
        metrics_port: override_field(
            base.metrics_port,
            override_config.metrics_port,
            default.metrics_port,
        ),
        otlp_endpoint: override_field(
            base.otlp_endpoint,
            override_config.otlp_endpoint,
            default.otlp_endpoint,
        ),
        otel_service_name: override_field(
            base.otel_service_name,
            override_config.otel_service_name,
            default.otel_service_name,
        ),
    }
}

fn merge_focus_config(
    base: FocusConfig,
    override_config: FocusConfig,
    default: FocusConfig,
) -> FocusConfig {
    FocusConfig {
        auto_focus: override_field(
            base.auto_focus,
            override_config.auto_focus,
            default.auto_focus,
        ),
        focus_source: override_field(
            base.focus_source,
            override_config.focus_source,
            default.focus_source,
        ),
        foreground_window: override_field(
            base.foreground_window,
            override_config.foreground_window,
            default.foreground_window,
        ),
        foreground_source: override_field(
            base.foreground_source,
            override_config.foreground_source,
            default.foreground_source,
        ),
        foreground_poll_ms: override_field(
            base.foreground_poll_ms,
            override_config.foreground_poll_ms,
            default.foreground_poll_ms,
        ),
        foreground_max_stale_ms: override_field(
            base.foreground_max_stale_ms,
            override_config.foreground_max_stale_ms,
            default.foreground_max_stale_ms,
        ),
        foreground_include_title: override_field(
            base.foreground_include_title,
            override_config.foreground_include_title,
            default.foreground_include_title,
        ),
        auto_focus_poll_ms: override_field(
            base.auto_focus_poll_ms,
            override_config.auto_focus_poll_ms,
            default.auto_focus_poll_ms,
        ),
        auto_focus_min_confidence: override_field(
            base.auto_focus_min_confidence,
            override_config.auto_focus_min_confidence,
            default.auto_focus_min_confidence,
        ),
        auto_focus_switch_cooldown_ms: override_field(
            base.auto_focus_switch_cooldown_ms,
            override_config.auto_focus_switch_cooldown_ms,
            default.auto_focus_switch_cooldown_ms,
        ),
        auto_focus_switch_margin: override_field(
            base.auto_focus_switch_margin,
            override_config.auto_focus_switch_margin,
            default.auto_focus_switch_margin,
        ),
        auto_focus_required_polls: override_field(
            base.auto_focus_required_polls,
            override_config.auto_focus_required_polls,
            default.auto_focus_required_polls,
        ),
        auto_focus_max_roots: override_field(
            base.auto_focus_max_roots,
            override_config.auto_focus_max_roots,
            default.auto_focus_max_roots,
        ),
    }
}

fn merge_safety_config(
    base: SafetyConfig,
    override_config: SafetyConfig,
    default: SafetyConfig,
) -> SafetyConfig {
    SafetyConfig {
        follow_exec: override_field(
            base.follow_exec,
            override_config.follow_exec,
            default.follow_exec,
        ),
        native_cgroup_filter: override_field(
            base.native_cgroup_filter,
            override_config.native_cgroup_filter,
            default.native_cgroup_filter,
        ),
    }
}

fn apply_user_file(
    mut config: MonitorConfig,
    user_file: &crate::config_file::UserConfigFile,
) -> MonitorConfig {
    if let Some(summary_ms) = user_file.summary_ms {
        config.timing.summary_period_ms = summary_ms;
    }

    if let Some(spike_us) = user_file.spike_us {
        config.timing.spike_threshold_ns = spike_us.saturating_mul(1_000);
    }

    if let Some(hwmon) = user_file.hwmon {
        config.probes.hwmon = hwmon;
    }

    if let Some(cpu_freq) = user_file.cpu_freq {
        config.probes.cpu_freq = cpu_freq;
    }

    if let Some(no_cpu_freq) = user_file.no_cpu_freq
        && no_cpu_freq
    {
        config.probes.cpu_freq = false;
    }

    if let Some(include_comm) = user_file.include_comm.clone() {
        config.target.include_comm = include_comm;
    }

    if let Some(exclude_comm) = user_file.exclude_comm.clone() {
        config.target.exclude_comm = exclude_comm;
    }

    if let Some(max_tasks) = user_file.max_tasks {
        config.target.max_tasks = max_tasks;
    }

    if let Some(retain_intervals) = user_file.retain_intervals {
        config.recording.retain_intervals = Some(retain_intervals);
    }

    if let Some(foreground_window) = user_file.foreground_window {
        config.focus.foreground_window = foreground_window;
    }

    if let Some(focus_source) = user_file.focus_source.clone() {
        config.focus.focus_source = focus_source;
    }

    if let Some(foreground_source) = user_file.foreground_source.clone() {
        config.focus.foreground_source = foreground_source;
    }

    if let Some(foreground_poll_ms) = user_file.foreground_poll_ms {
        config.focus.foreground_poll_ms = foreground_poll_ms;
    }

    if let Some(foreground_max_stale_ms) = user_file.foreground_max_stale_ms {
        config.focus.foreground_max_stale_ms = foreground_max_stale_ms;
    }

    if let Some(foreground_include_title) = user_file.foreground_include_title {
        config.focus.foreground_include_title = foreground_include_title;
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_monitor_config_keeps_base_when_override_is_default() {
        let mut base = MonitorConfig::default();
        base.timing.summary_period_ms = 500;
        base.probes.hwmon = true;

        let merged = merge_monitor_config(base.clone(), MonitorConfig::default());

        assert_eq!(merged.timing.summary_period_ms, 500);
        assert!(merged.probes.hwmon);
    }

    #[test]
    fn merge_monitor_config_uses_override_when_field_differs_from_default() {
        let base = MonitorConfig::default();
        let mut override_config = MonitorConfig::default();
        override_config.timing.summary_period_ms = 250;
        override_config.target.max_tasks = 64;
        override_config.focus.foreground_include_title = true;

        let merged = merge_monitor_config(base, override_config);

        assert_eq!(merged.timing.summary_period_ms, 250);
        assert_eq!(merged.target.max_tasks, 64);
        assert!(merged.focus.foreground_include_title);
    }

    #[test]
    fn merge_config_sources_applies_user_file_before_cli_overrides() {
        let mut cli = MonitorConfig::default();
        cli.timing.summary_period_ms = 1_000;

        let user_file = crate::config_file::UserConfigFile {
            summary_ms: Some(333),
            spike_us: Some(2_500),
            hwmon: Some(true),
            cpu_freq: Some(true),
            max_tasks: Some(77),
            retain_intervals: Some(12),
            foreground_window: Some(true),
            focus_source: Some("hybrid".to_owned()),
            foreground_source: Some("sway".to_owned()),
            foreground_poll_ms: Some(444),
            foreground_max_stale_ms: Some(555),
            foreground_include_title: Some(true),
            ..Default::default()
        };

        let merged = merge_config_sources(ConfigSources {
            defaults: DefaultConfig::default(),
            user_file: Some(user_file),
            preset: None,
            cli: CliOverrides { config: cli },
        });

        assert_eq!(merged.timing.summary_period_ms, 333);
        assert_eq!(merged.timing.spike_threshold_ns, 2_500_000);
        assert!(merged.probes.hwmon);
        assert!(merged.probes.cpu_freq);
        assert_eq!(merged.target.max_tasks, 77);
        assert_eq!(merged.recording.retain_intervals, Some(12));
        assert!(merged.focus.foreground_window);
        assert_eq!(merged.focus.focus_source, "hybrid");
        assert_eq!(merged.focus.foreground_source, "sway");
        assert_eq!(merged.focus.foreground_poll_ms, 444);
        assert_eq!(merged.focus.foreground_max_stale_ms, 555);
        assert!(merged.focus.foreground_include_title);
    }

    #[test]
    fn user_file_no_cpu_freq_overrides_cpu_freq_true() {
        let user_file = crate::config_file::UserConfigFile {
            cpu_freq: Some(true),
            no_cpu_freq: Some(true),
            ..Default::default()
        };

        let merged = merge_config_sources(ConfigSources {
            defaults: DefaultConfig::default(),
            user_file: Some(user_file),
            preset: None,
            cli: CliOverrides {
                config: MonitorConfig::default(),
            },
        });

        assert!(!merged.probes.cpu_freq);
    }
}
