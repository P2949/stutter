use std::fmt;

use crate::{
    cli::Config,
    config::{
        layer::MonitorConfigLayer,
        model::{
            FocusConfig, MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, SafetyConfig,
            TargetConfig, TimingConfig,
        },
    },
    presets::Preset,
};

#[derive(Debug)]
pub enum ConfigError {
    UserConfig(anyhow::Error),
    InvalidPreset(anyhow::Error),
    InvalidUserLayer(anyhow::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserConfig(err) => write!(f, "failed to load user config: {err:#}"),
            Self::InvalidPreset(err) => write!(f, "failed to resolve monitor preset: {err:#}"),
            Self::InvalidUserLayer(err) => {
                write!(f, "failed to convert user config to monitor layer: {err:#}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveMonitorConfig {
    pub config: MonitorConfig,
}

impl EffectiveMonitorConfig {
    pub fn from_layers(
        defaults: MonitorConfig,
        user_file: Option<MonitorConfigLayer>,
        preset: Option<MonitorConfigLayer>,
        cli: MonitorConfigLayer,
    ) -> Result<Self, ConfigError> {
        let mut config = defaults;

        if let Some(layer) = user_file {
            apply_layer(&mut config, layer);
        }

        if let Some(layer) = preset {
            apply_layer(&mut config, layer);
        }

        apply_layer(&mut config, cli);

        Ok(Self { config })
    }

    pub fn from_cli_config(config: &Config) -> Result<Self, ConfigError> {
        let user_file = crate::config_file::load_user_config()
            .map_err(ConfigError::UserConfig)?
            .as_ref()
            .map(MonitorConfigLayer::from_user_file)
            .transpose()
            .map_err(ConfigError::InvalidUserLayer)?;

        let preset = config
            .preset
            .as_deref()
            .map(|value| {
                let preset = value
                    .parse::<Preset>()
                    .map_err(ConfigError::InvalidPreset)?;
                Ok::<MonitorConfigLayer, ConfigError>(MonitorConfigLayer::from_preset_defaults(
                    preset.defaults(),
                ))
            })
            .transpose()?;

        let cli = config
            .monitor_config_layer
            .clone()
            .unwrap_or_else(|| MonitorConfigLayer::from_existing_cli_config(config));

        Self::from_layers(MonitorConfig::default(), user_file, preset, cli)
    }

    pub fn into_monitor_config(self) -> MonitorConfig {
        self.config
    }
}

pub fn resolve_arc_monitor_config(
    config: std::sync::Arc<Config>,
) -> Result<std::sync::Arc<Config>, ConfigError> {
    let effective_config = resolve_monitor_config(&config)?;
    let effective = effective_config.clone();
    let mut resolved = (*config).clone();

    resolved.target_pids = effective.target.target_pids.clone();
    resolved.tree_pids = effective.target.tree_pids.clone();
    resolved.cgroupv2 = effective.target.cgroupv2.clone();
    resolved.exclude_tree_pids = effective.target.exclude_tree_pids.clone();
    resolved.watch_process = effective.target.watch_process.clone();
    resolved.persistent = effective.target.persistent;
    resolved.keep_missing_pid = effective.target.keep_missing_pid;
    resolved.max_tasks = effective.target.max_tasks;

    resolved.summary_period_ms = effective.timing.summary_period_ms;
    resolved.epoch_period_ms = effective.timing.epoch_period_ms;
    resolved.max_duration = effective.timing.max_duration;
    resolved.spike_threshold_ns = effective.timing.spike_threshold_ns;

    resolved.irq_latency = effective.probes.irq_latency;
    resolved.irqs = effective.probes.irqs.clone();
    resolved.hwmon = effective.probes.hwmon;
    resolved.cpu_freq = effective.probes.cpu_freq;
    resolved.faults = effective.probes.faults;
    resolved.cpu_perf = effective.probes.cpu_perf;
    resolved.block_io = effective.probes.block_io;
    resolved.stat_wait = effective.probes.stat_wait;
    resolved.runtime_slices = effective.probes.runtime_slices;

    resolved.retain_intervals = effective.recording.retain_intervals;
    if let Some(recording) = resolved.recording.as_mut() {
        recording.run_name = effective.recording.run_name.clone();
        recording.out_dir = effective.recording.output_dir.clone();
    }

    resolved.json_stream = effective.outputs.json_stream;
    resolved.metrics_port = effective.outputs.metrics_port;
    resolved.otlp_endpoint = effective.outputs.otlp_endpoint.clone();
    resolved.otel_service_name = effective.outputs.otel_service_name.clone();

    resolved.auto_focus = effective.focus.auto_focus;
    resolved.focus_source = effective.focus.focus_source;
    resolved.foreground_window = effective.focus.foreground_window;
    resolved.foreground_source = effective.focus.foreground_source;
    resolved.foreground_poll_ms = effective.focus.foreground_poll_ms;
    resolved.foreground_max_stale_ms = effective.focus.foreground_max_stale_ms;
    resolved.foreground_include_title = effective.focus.foreground_include_title;
    resolved.auto_focus_poll_ms = effective.focus.auto_focus_poll_ms;
    resolved.auto_focus_min_confidence = effective.focus.auto_focus_min_confidence;
    resolved.auto_focus_switch_cooldown_ms = effective.focus.auto_focus_switch_cooldown_ms;
    resolved.auto_focus_switch_margin = effective.focus.auto_focus_switch_margin;
    resolved.auto_focus_required_polls = effective.focus.auto_focus_required_polls;
    resolved.auto_focus_max_roots = effective.focus.auto_focus_max_roots;

    resolved.follow_exec = effective.safety.follow_exec;
    resolved.native_cgroup_filter = effective.safety.native_cgroup_filter;
    resolved.task_filters = crate::process_tree::TaskFilters {
        include_comm: effective
            .target
            .include_comm
            .iter()
            .map(|pattern| crate::process_tree::CompiledPattern::new(pattern.clone()))
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(ConfigError::InvalidUserLayer)?,
        exclude_comm: effective
            .target
            .exclude_comm
            .iter()
            .map(|pattern| crate::process_tree::CompiledPattern::new(pattern.clone()))
            .collect::<anyhow::Result<Vec<_>>>()
            .map_err(ConfigError::InvalidUserLayer)?,
    };
    resolved.monitor_config_layer =
        Some(MonitorConfigLayer::from_monitor_config(effective.clone()));

    Ok(std::sync::Arc::new(resolved))
}

pub fn resolve_monitor_config(config: &Config) -> Result<MonitorConfig, ConfigError> {
    Ok(EffectiveMonitorConfig::from_cli_config(config)?.into_monitor_config())
}

pub fn apply_layer(config: &mut MonitorConfig, layer: MonitorConfigLayer) {
    apply_target_layer(&mut config.target, &layer);
    apply_timing_layer(&mut config.timing, &layer);
    apply_probe_layer(&mut config.probes, &layer);
    apply_recording_layer(&mut config.recording, &layer);
    apply_output_layer(&mut config.outputs, &layer);
    apply_focus_layer(&mut config.focus, &layer);
    apply_safety_layer(&mut config.safety, &layer);
}

fn apply_target_layer(config: &mut TargetConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.target_pids {
        config.target_pids = value.clone();
    }
    if let Some(value) = &layer.tree_pids {
        config.tree_pids = value.clone();
    }
    if let Some(value) = &layer.cgroupv2 {
        config.cgroupv2 = value.clone();
    }
    if let Some(value) = &layer.exclude_tree_pids {
        config.exclude_tree_pids = value.clone();
    }
    if let Some(value) = &layer.include_comm {
        config.include_comm = value.clone();
    }
    if let Some(value) = &layer.exclude_comm {
        config.exclude_comm = value.clone();
    }
    if let Some(value) = &layer.watch_process {
        config.watch_process = value.clone();
    }
    if let Some(value) = layer.persistent {
        config.persistent = value;
    }
    if let Some(value) = layer.keep_missing_pid {
        config.keep_missing_pid = value;
    }
    if let Some(value) = layer.max_tasks {
        config.max_tasks = value;
    }
}

fn apply_timing_layer(config: &mut TimingConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.summary_period_ms {
        config.summary_period_ms = value;
    }
    if let Some(value) = layer.epoch_period_ms {
        config.epoch_period_ms = value;
    }
    if let Some(value) = layer.max_duration {
        config.max_duration = value;
    }
    if let Some(value) = layer.spike_threshold_ns {
        config.spike_threshold_ns = value;
    }
}

fn apply_probe_layer(config: &mut ProbeConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.irq_latency {
        config.irq_latency = value;
    }
    if let Some(value) = &layer.irqs {
        config.irqs = value.clone();
    }
    if let Some(value) = layer.hwmon {
        config.hwmon = value;
    }
    if let Some(value) = layer.cpu_freq {
        config.cpu_freq = value;
    }
    if let Some(value) = layer.faults {
        config.faults = value;
    }
    if let Some(value) = layer.cpu_perf {
        config.cpu_perf = value;
    }
    if let Some(value) = layer.block_io {
        config.block_io = value;
    }
    if let Some(value) = layer.stat_wait {
        config.stat_wait = value;
    }
    if let Some(value) = layer.runtime_slices {
        config.runtime_slices = value;
    }
}

fn apply_recording_layer(config: &mut RecordingConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.run_name {
        config.run_name = value.clone();
    }
    if let Some(value) = &layer.output_dir {
        config.output_dir = value.clone();
    }
    if let Some(value) = layer.retain_intervals {
        config.retain_intervals = value;
    }
}

fn apply_output_layer(config: &mut OutputConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.json_stream {
        config.json_stream = value;
    }
    if let Some(value) = layer.metrics_port {
        config.metrics_port = value;
    }
    if let Some(value) = &layer.otlp_endpoint {
        config.otlp_endpoint = value.clone();
    }
    if let Some(value) = &layer.otel_service_name {
        config.otel_service_name = value.clone();
    }
}

fn apply_focus_layer(config: &mut FocusConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.auto_focus {
        config.auto_focus = value;
    }
    if let Some(value) = layer.focus_source {
        config.focus_source = value;
    }
    if let Some(value) = layer.foreground_window {
        config.foreground_window = value;
    }
    if let Some(value) = layer.foreground_source {
        config.foreground_source = value;
    }
    if let Some(value) = layer.foreground_poll_ms {
        config.foreground_poll_ms = value;
    }
    if let Some(value) = layer.foreground_max_stale_ms {
        config.foreground_max_stale_ms = value;
    }
    if let Some(value) = layer.foreground_include_title {
        config.foreground_include_title = value;
    }
    if let Some(value) = layer.auto_focus_poll_ms {
        config.auto_focus_poll_ms = value;
    }
    if let Some(value) = layer.auto_focus_min_confidence {
        config.auto_focus_min_confidence = value;
    }
    if let Some(value) = layer.auto_focus_switch_cooldown_ms {
        config.auto_focus_switch_cooldown_ms = value;
    }
    if let Some(value) = layer.auto_focus_switch_margin {
        config.auto_focus_switch_margin = value;
    }
    if let Some(value) = layer.auto_focus_required_polls {
        config.auto_focus_required_polls = value;
    }
    if let Some(value) = layer.auto_focus_max_roots {
        config.auto_focus_max_roots = value;
    }
}

fn apply_safety_layer(config: &mut SafetyConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.follow_exec {
        config.follow_exec = value;
    }
    if let Some(value) = layer.native_cgroup_filter {
        config.native_cgroup_filter = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{FocusSource, ForegroundSourceArg};

    #[test]
    fn cli_false_override_replaces_user_true() {
        let user = MonitorConfigLayer {
            hwmon: Some(true),
            cpu_freq: Some(true),
            ..Default::default()
        };
        let cli = MonitorConfigLayer {
            hwmon: Some(false),
            cpu_freq: Some(false),
            ..Default::default()
        };

        let effective =
            EffectiveMonitorConfig::from_layers(MonitorConfig::default(), Some(user), None, cli)
                .unwrap();

        assert!(!effective.config.probes.hwmon);
        assert!(!effective.config.probes.cpu_freq);
    }

    #[test]
    fn later_layers_override_earlier_layers_even_when_value_is_default() {
        let user = MonitorConfigLayer {
            summary_period_ms: Some(250),
            focus_source: Some(FocusSource::Hybrid),
            foreground_source: Some(ForegroundSourceArg::Sway),
            auto_focus_min_confidence: Some(0.80),
            ..Default::default()
        };
        let cli = MonitorConfigLayer {
            summary_period_ms: Some(1_000),
            focus_source: Some(FocusSource::Heuristic),
            foreground_source: Some(ForegroundSourceArg::Auto),
            auto_focus_min_confidence: Some(0.60),
            ..Default::default()
        };

        let effective =
            EffectiveMonitorConfig::from_layers(MonitorConfig::default(), Some(user), None, cli)
                .unwrap();

        assert_eq!(effective.config.timing.summary_period_ms, 1_000);
        assert_eq!(effective.config.focus.focus_source, FocusSource::Heuristic);
        assert_eq!(
            effective.config.focus.foreground_source,
            ForegroundSourceArg::Auto
        );
        assert_eq!(effective.config.focus.auto_focus_min_confidence, 0.60);
    }

    #[test]
    fn preset_layer_sits_between_user_file_and_cli() {
        let user = MonitorConfigLayer {
            block_io: Some(false),
            ..Default::default()
        };
        let preset = MonitorConfigLayer {
            block_io: Some(true),
            ..Default::default()
        };
        let cli = MonitorConfigLayer {
            block_io: Some(false),
            ..Default::default()
        };

        let effective = EffectiveMonitorConfig::from_layers(
            MonitorConfig::default(),
            Some(user),
            Some(preset),
            cli,
        )
        .unwrap();

        assert!(!effective.config.probes.block_io);
    }
}
