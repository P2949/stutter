use crate::{
    config::{
        layer::MonitorConfigLayer,
        merge::{self, ConfigSources},
        model::{
            FocusConfig, MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, SafetyConfig,
            TargetConfig, TimingConfig,
        },
        source::{ConfigDiagnostic, FieldProvenance},
    },
    error::ConfigError,
};

#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveMonitorConfig {
    pub config: MonitorConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMonitorConfig {
    pub config: MonitorConfig,
    pub provenance: Vec<FieldProvenance>,
    pub diagnostics: Vec<ConfigDiagnostic>,
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

    pub fn into_monitor_config(self) -> MonitorConfig {
        self.config
    }
}

pub fn resolve_monitor_config_sources(
    sources: ConfigSources,
) -> Result<ResolvedMonitorConfig, ConfigError> {
    let mut config = merge::merge_config_sources_checked(sources)?;
    compile_task_filters(&mut config)?;

    Ok(ResolvedMonitorConfig {
        config,
        provenance: Vec::new(),
        diagnostics: Vec::new(),
    })
}

pub(crate) fn compile_task_filters(config: &mut MonitorConfig) -> Result<(), ConfigError> {
    let compiled_include = config
        .target
        .include_comm
        .iter()
        .map(|pattern| crate::process_tree::CompiledPattern::new(pattern.clone()))
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(ConfigError::InvalidUserLayer)?;
    let compiled_exclude = config
        .target
        .exclude_comm
        .iter()
        .map(|pattern| crate::process_tree::CompiledPattern::new(pattern.clone()))
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(ConfigError::InvalidUserLayer)?;

    config.target.task_filters = crate::process_tree::TaskFilters {
        include_comm: compiled_include,
        exclude_comm: compiled_exclude,
    };

    Ok(())
}

pub fn apply_layer(config: &mut MonitorConfig, layer: MonitorConfigLayer) {
    apply_target_layer(&mut config.target, &layer);
    apply_timing_layer(&mut config.timing, &layer);
    apply_probe_layer(&mut config.probes, &layer);
    apply_recording_layer(&mut config.recording, &layer);
    apply_output_layer(&mut config.outputs, &layer);
    apply_focus_layer(&mut config.focus, &layer);
    apply_safety_layer(&mut config.safety, &layer);
    apply_watch_layer(&mut config.watch, &layer);
    apply_alert_layer(&mut config.alerts, &layer);
    apply_stream_layer(&mut config.streams, &layer);
    apply_hwmon_layer(&mut config.hwmon, &layer);
    apply_mangohud_layer(&mut config.mangohud, &layer);
    apply_cpu_perf_layer(&mut config.cpu_perf, &layer);
    apply_runtime_slices_layer(&mut config.runtime_slices, &layer);
    apply_ebpf_sizing_layer(&mut config.ebpf_sizing, &layer);
    apply_ui_layer(&mut config.ui, &layer);
    apply_remote_layer(&mut config.remote, &layer);
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

fn apply_watch_layer(config: &mut crate::config::model::WatchConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.watch_poll_ms {
        config.poll_ms = value;
    }
    if let Some(value) = layer.watch_timeout {
        config.timeout = value;
    }
}

fn apply_alert_layer(config: &mut crate::config::model::AlertConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.alert_threshold_ns {
        config.threshold_ns = value;
    }
    if let Some(value) = &layer.alert_webhook_url {
        config.webhook_url = value.clone();
    }
}

fn apply_stream_layer(config: &mut crate::config::model::StreamConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.csv_stream {
        config.csv = value.clone();
    }
    if let Some(value) = layer.json_stream {
        config.json_stream = value;
    }
    if let Some(value) = layer.verbose {
        config.verbose = value;
    }
}

fn apply_hwmon_layer(config: &mut crate::config::model::HwmonConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.hwmon {
        config.enabled = value;
    }
    if let Some(value) = &layer.hwmon_root {
        config.root = value.clone();
    }
    if let Some(value) = &layer.hwmon_drm_card {
        config.drm_card = value.clone();
    }
    if let Some(value) = &layer.hwmon_render_node {
        config.render_node = value.clone();
    }
}

fn apply_mangohud_layer(
    config: &mut crate::config::model::MangoHudConfig,
    layer: &MonitorConfigLayer,
) {
    if let Some(value) = &layer.mangohud_log {
        config.log = value.clone();
    }
    if let Some(value) = layer.mangohud_log_live {
        config.log_live = value;
    }
}

fn apply_cpu_perf_layer(
    config: &mut crate::config::model::CpuPerfConfig,
    layer: &MonitorConfigLayer,
) {
    if let Some(value) = layer.cpu_perf {
        config.enabled = value;
    }
    if let Some(value) = layer.cpu_perf_kernel {
        config.include_kernel = value;
    }
    if let Some(value) = layer.cpu_perf_max_tasks {
        config.max_tasks = value;
    }
    if let Some(value) = layer.cpu_perf_cache_refs {
        config.collect_cache_refs = value;
    }
}

fn apply_runtime_slices_layer(
    config: &mut crate::config::model::RuntimeSlicesConfig,
    layer: &MonitorConfigLayer,
) {
    if let Some(value) = layer.runtime_slices {
        config.enabled = value;
    }
    if let Some(value) = layer.runtime_slices_max_tasks {
        config.max_tasks = value;
    }
}

fn apply_ebpf_sizing_layer(
    config: &mut crate::config::model::EbpfSizingConfig,
    layer: &MonitorConfigLayer,
) {
    if let Some(value) = layer.ringbuf_size_kb {
        config.ringbuf_size_kb = value;
    }
    if let Some(value) = layer.wakeup_map_factor {
        config.wakeup_map_factor = value;
    }
}

fn apply_ui_layer(config: &mut crate::config::model::UiConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = layer.tui {
        config.tui = value;
    }
}

fn apply_remote_layer(config: &mut crate::config::model::RemoteConfig, layer: &MonitorConfigLayer) {
    if let Some(value) = &layer.remote {
        config.endpoint = value.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FocusSource, ForegroundSource};

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
            foreground_source: Some(ForegroundSource::Sway),
            auto_focus_min_confidence: Some(0.80),
            ..Default::default()
        };
        let cli = MonitorConfigLayer {
            summary_period_ms: Some(1_000),
            focus_source: Some(FocusSource::Heuristic),
            foreground_source: Some(ForegroundSource::Auto),
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
            ForegroundSource::Auto
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
