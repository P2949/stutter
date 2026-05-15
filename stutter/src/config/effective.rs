use crate::{
    config::{
        layer::MonitorConfigLayer,
        merge::{self, ConfigSources},
        model::{
            FocusConfig, MonitorConfig, OutputConfig, ProbeConfig, RecordingConfig, SafetyConfig,
            TargetConfig, TimingConfig,
        },
        schema::ConfigDiagnostic,
        source::{ConfigSource, FieldProvenance},
    },
    error::ConfigError,
};

#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveMonitorConfig {
    pub config: MonitorConfig,
    pub provenance: Vec<FieldProvenance>,
    pub diagnostics: Vec<ConfigDiagnostic>,
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
        Self::from_layers_with_sources(
            defaults,
            user_file,
            preset,
            cli,
            ConfigSource::Cli,
            Vec::new(),
        )
    }

    pub fn from_layers_with_sources(
        defaults: MonitorConfig,
        user_file: Option<MonitorConfigLayer>,
        preset: Option<MonitorConfigLayer>,
        overrides: MonitorConfigLayer,
        overrides_source: ConfigSource,
        diagnostics: Vec<ConfigDiagnostic>,
    ) -> Result<Self, ConfigError> {
        let mut config = defaults;
        let mut provenance = Vec::new();

        record_layer_provenance(
            &MonitorConfigLayer::from_monitor_config(config.clone()),
            ConfigSource::Default,
            &mut provenance,
        );

        if let Some(layer) = user_file {
            apply_layer_with_provenance(
                &mut config,
                layer,
                ConfigSource::UserFile,
                &mut provenance,
            );
        }

        if let Some(layer) = preset {
            apply_layer_with_provenance(&mut config, layer, ConfigSource::Preset, &mut provenance);
        }

        apply_layer_with_provenance(&mut config, overrides, overrides_source, &mut provenance);

        Ok(Self {
            config,
            provenance,
            diagnostics,
        })
    }

    pub fn into_monitor_config(self) -> MonitorConfig {
        self.config
    }
}

pub fn resolve_monitor_config_sources(
    sources: ConfigSources,
) -> Result<ResolvedMonitorConfig, ConfigError> {
    let effective = merge::merge_config_sources_effective_checked(sources)?;

    Ok(ResolvedMonitorConfig {
        config: effective.config,
        provenance: effective.provenance,
        diagnostics: effective.diagnostics,
    })
}

pub fn apply_layer(config: &mut MonitorConfig, layer: MonitorConfigLayer) {
    let mut ignored_provenance = Vec::new();
    apply_layer_with_provenance(config, layer, ConfigSource::Cli, &mut ignored_provenance);
}

fn apply_layer_with_provenance(
    config: &mut MonitorConfig,
    layer: MonitorConfigLayer,
    source: ConfigSource,
    provenance: &mut Vec<FieldProvenance>,
) {
    record_layer_provenance(&layer, source, provenance);

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

fn record_layer_provenance(
    layer: &MonitorConfigLayer,
    source: ConfigSource,
    provenance: &mut Vec<FieldProvenance>,
) {
    fn record_if_present<T>(
        value: &Option<T>,
        provenance: &mut Vec<FieldProvenance>,
        field: &'static str,
        source: ConfigSource,
    ) {
        if value.is_some() {
            provenance.push(FieldProvenance::new(field, source));
        }
    }

    record_if_present(&layer.target_pids, provenance, "target.target_pids", source);
    record_if_present(&layer.tree_pids, provenance, "target.tree_pids", source);
    record_if_present(&layer.cgroupv2, provenance, "target.cgroupv2", source);
    record_if_present(
        &layer.exclude_tree_pids,
        provenance,
        "target.exclude_tree_pids",
        source,
    );
    record_if_present(
        &layer.include_comm,
        provenance,
        "target.include_comm",
        source,
    );
    record_if_present(
        &layer.exclude_comm,
        provenance,
        "target.exclude_comm",
        source,
    );
    record_if_present(
        &layer.watch_process,
        provenance,
        "target.watch_process",
        source,
    );
    record_if_present(&layer.persistent, provenance, "target.persistent", source);
    record_if_present(
        &layer.keep_missing_pid,
        provenance,
        "target.keep_missing_pid",
        source,
    );
    record_if_present(&layer.max_tasks, provenance, "target.max_tasks", source);

    record_if_present(
        &layer.summary_period_ms,
        provenance,
        "timing.summary_period_ms",
        source,
    );
    record_if_present(
        &layer.epoch_period_ms,
        provenance,
        "timing.epoch_period_ms",
        source,
    );
    record_if_present(
        &layer.max_duration,
        provenance,
        "timing.max_duration",
        source,
    );
    record_if_present(
        &layer.spike_threshold_ns,
        provenance,
        "timing.spike_threshold_ns",
        source,
    );

    record_if_present(&layer.irq_latency, provenance, "probes.irq_latency", source);
    record_if_present(&layer.irqs, provenance, "probes.irqs", source);
    record_if_present(&layer.hwmon, provenance, "probes.hwmon", source);
    record_if_present(&layer.hwmon, provenance, "hwmon.enabled", source);
    record_if_present(&layer.cpu_freq, provenance, "probes.cpu_freq", source);
    record_if_present(&layer.faults, provenance, "probes.faults", source);
    record_if_present(&layer.cpu_perf, provenance, "probes.cpu_perf", source);
    record_if_present(&layer.cpu_perf, provenance, "cpu_perf.enabled", source);
    record_if_present(&layer.block_io, provenance, "probes.block_io", source);
    record_if_present(&layer.stat_wait, provenance, "probes.stat_wait", source);
    record_if_present(
        &layer.runtime_slices,
        provenance,
        "probes.runtime_slices",
        source,
    );
    record_if_present(
        &layer.runtime_slices,
        provenance,
        "runtime_slices.enabled",
        source,
    );

    record_if_present(&layer.run_name, provenance, "recording.run_name", source);
    record_if_present(
        &layer.output_dir,
        provenance,
        "recording.output_dir",
        source,
    );
    record_if_present(
        &layer.retain_intervals,
        provenance,
        "recording.retain_intervals",
        source,
    );
    record_if_present(
        &layer.retention_max_run_count,
        provenance,
        "recording.retention.max_run_count",
        source,
    );
    record_if_present(
        &layer.retention_max_total_bytes,
        provenance,
        "recording.retention.max_total_bytes",
        source,
    );
    record_if_present(
        &layer.retention_max_age_seconds,
        provenance,
        "recording.retention.max_age_seconds",
        source,
    );
    record_if_present(
        &layer.retention_min_free_bytes,
        provenance,
        "recording.retention.min_free_bytes",
        source,
    );

    record_if_present(
        &layer.json_stream,
        provenance,
        "outputs.json_stream",
        source,
    );
    record_if_present(
        &layer.json_stream,
        provenance,
        "streams.json_stream",
        source,
    );
    record_if_present(
        &layer.metrics_port,
        provenance,
        "outputs.metrics_port",
        source,
    );
    record_if_present(
        &layer.otlp_endpoint,
        provenance,
        "outputs.otlp_endpoint",
        source,
    );
    record_if_present(
        &layer.otel_service_name,
        provenance,
        "outputs.otel_service_name",
        source,
    );

    record_if_present(&layer.auto_focus, provenance, "focus.auto_focus", source);
    record_if_present(
        &layer.focus_source,
        provenance,
        "focus.focus_source",
        source,
    );
    record_if_present(
        &layer.foreground_window,
        provenance,
        "focus.foreground_window",
        source,
    );
    record_if_present(
        &layer.foreground_source,
        provenance,
        "focus.foreground_source",
        source,
    );
    record_if_present(
        &layer.foreground_poll_ms,
        provenance,
        "focus.foreground_poll_ms",
        source,
    );
    record_if_present(
        &layer.foreground_max_stale_ms,
        provenance,
        "focus.foreground_max_stale_ms",
        source,
    );
    record_if_present(
        &layer.foreground_include_title,
        provenance,
        "focus.foreground_include_title",
        source,
    );
    record_if_present(
        &layer.auto_focus_poll_ms,
        provenance,
        "focus.auto_focus_poll_ms",
        source,
    );
    record_if_present(
        &layer.auto_focus_min_confidence,
        provenance,
        "focus.auto_focus_min_confidence",
        source,
    );
    record_if_present(
        &layer.auto_focus_switch_cooldown_ms,
        provenance,
        "focus.auto_focus_switch_cooldown_ms",
        source,
    );
    record_if_present(
        &layer.auto_focus_switch_margin,
        provenance,
        "focus.auto_focus_switch_margin",
        source,
    );
    record_if_present(
        &layer.auto_focus_required_polls,
        provenance,
        "focus.auto_focus_required_polls",
        source,
    );
    record_if_present(
        &layer.auto_focus_max_roots,
        provenance,
        "focus.auto_focus_max_roots",
        source,
    );

    record_if_present(&layer.follow_exec, provenance, "safety.follow_exec", source);
    record_if_present(
        &layer.native_cgroup_filter,
        provenance,
        "safety.native_cgroup_filter",
        source,
    );

    record_if_present(&layer.watch_poll_ms, provenance, "watch.poll_ms", source);
    record_if_present(&layer.watch_timeout, provenance, "watch.timeout", source);

    record_if_present(
        &layer.alert_threshold_ns,
        provenance,
        "alerts.threshold_ns",
        source,
    );
    record_if_present(
        &layer.alert_webhook_url,
        provenance,
        "alerts.webhook_url",
        source,
    );

    record_if_present(&layer.csv_stream, provenance, "streams.csv", source);
    record_if_present(&layer.verbose, provenance, "streams.verbose", source);

    record_if_present(&layer.hwmon_root, provenance, "hwmon.root", source);
    record_if_present(&layer.hwmon_drm_card, provenance, "hwmon.drm_card", source);
    record_if_present(
        &layer.hwmon_render_node,
        provenance,
        "hwmon.render_node",
        source,
    );

    record_if_present(&layer.mangohud_log, provenance, "mangohud.log", source);
    record_if_present(
        &layer.mangohud_log_live,
        provenance,
        "mangohud.log_live",
        source,
    );

    record_if_present(&layer.tui, provenance, "ui.tui", source);

    record_if_present(
        &layer.cpu_perf_kernel,
        provenance,
        "cpu_perf.include_kernel",
        source,
    );
    record_if_present(
        &layer.cpu_perf_max_tasks,
        provenance,
        "cpu_perf.max_tasks",
        source,
    );
    record_if_present(
        &layer.cpu_perf_cache_refs,
        provenance,
        "cpu_perf.collect_cache_refs",
        source,
    );

    record_if_present(
        &layer.runtime_slices_max_tasks,
        provenance,
        "runtime_slices.max_tasks",
        source,
    );

    record_if_present(
        &layer.ringbuf_size_kb,
        provenance,
        "ebpf_sizing.ringbuf_size_kb",
        source,
    );
    record_if_present(
        &layer.wakeup_map_factor,
        provenance,
        "ebpf_sizing.wakeup_map_factor",
        source,
    );

    record_if_present(&layer.remote, provenance, "remote.endpoint", source);
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
    if let Some(value) = layer.retention_max_run_count {
        config.retention.max_run_count = value;
    }
    if let Some(value) = layer.retention_max_total_bytes {
        config.retention.max_total_bytes = value;
    }
    if let Some(value) = layer.retention_max_age_seconds {
        config.retention.max_age_seconds = value;
    }
    if let Some(value) = layer.retention_min_free_bytes {
        config.retention.min_free_bytes = value;
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

    fn last_source_for_field(
        provenance: &[FieldProvenance],
        field: &'static str,
    ) -> Option<ConfigSource> {
        provenance
            .iter()
            .rev()
            .find(|entry| entry.field == field)
            .map(|entry| entry.source)
    }

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
        assert_eq!(
            last_source_for_field(&effective.provenance, "probes.hwmon"),
            Some(ConfigSource::Cli)
        );
        assert_eq!(
            last_source_for_field(&effective.provenance, "probes.cpu_freq"),
            Some(ConfigSource::Cli)
        );
    }

    #[test]
    fn duplicated_layer_fields_record_provenance_for_each_config_field_they_apply() {
        let user = MonitorConfigLayer {
            hwmon: Some(true),
            json_stream: Some(true),
            cpu_perf: Some(true),
            runtime_slices: Some(true),
            ..Default::default()
        };
        let cli = MonitorConfigLayer {
            hwmon: Some(false),
            json_stream: Some(false),
            cpu_perf: Some(false),
            runtime_slices: Some(false),
            ..Default::default()
        };

        let effective =
            EffectiveMonitorConfig::from_layers(MonitorConfig::default(), Some(user), None, cli)
                .unwrap();

        assert!(!effective.config.probes.hwmon);
        assert!(!effective.config.hwmon.enabled);
        assert!(!effective.config.outputs.json_stream);
        assert!(!effective.config.streams.json_stream);
        assert!(!effective.config.probes.cpu_perf);
        assert!(!effective.config.cpu_perf.enabled);
        assert!(!effective.config.probes.runtime_slices);
        assert!(!effective.config.runtime_slices.enabled);

        for field in [
            "probes.hwmon",
            "hwmon.enabled",
            "outputs.json_stream",
            "streams.json_stream",
            "probes.cpu_perf",
            "cpu_perf.enabled",
            "probes.runtime_slices",
            "runtime_slices.enabled",
        ] {
            assert_eq!(
                last_source_for_field(&effective.provenance, field),
                Some(ConfigSource::Cli),
                "wrong provenance for {field}"
            );
        }
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
        assert_eq!(
            last_source_for_field(&effective.provenance, "timing.summary_period_ms"),
            Some(ConfigSource::Cli)
        );
        assert_eq!(
            last_source_for_field(&effective.provenance, "focus.focus_source"),
            Some(ConfigSource::Cli)
        );
        assert_eq!(
            last_source_for_field(&effective.provenance, "focus.foreground_source"),
            Some(ConfigSource::Cli)
        );
        assert_eq!(
            last_source_for_field(&effective.provenance, "focus.auto_focus_min_confidence"),
            Some(ConfigSource::Cli)
        );
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
        assert_eq!(
            last_source_for_field(&effective.provenance, "probes.block_io"),
            Some(ConfigSource::Cli)
        );

        let block_io_sources: Vec<_> = effective
            .provenance
            .iter()
            .filter(|entry| entry.field == "probes.block_io")
            .map(|entry| entry.source)
            .collect();

        assert!(block_io_sources.contains(&ConfigSource::Default));
        assert!(block_io_sources.contains(&ConfigSource::UserFile));
        assert!(block_io_sources.contains(&ConfigSource::Preset));
        assert!(block_io_sources.contains(&ConfigSource::Cli));
    }

    #[test]
    fn default_source_is_recorded_for_default_fields() {
        let effective = EffectiveMonitorConfig::from_layers(
            MonitorConfig::default(),
            None,
            None,
            MonitorConfigLayer::default(),
        )
        .unwrap();

        assert_eq!(
            last_source_for_field(&effective.provenance, "timing.summary_period_ms"),
            Some(ConfigSource::Default)
        );
        assert_eq!(
            last_source_for_field(&effective.provenance, "target.max_tasks"),
            Some(ConfigSource::Default)
        );
    }

    #[test]
    fn default_source_is_recorded_for_all_fields_applied_from_default_config() {
        let effective = EffectiveMonitorConfig::from_layers(
            MonitorConfig::default(),
            None,
            None,
            MonitorConfigLayer::default(),
        )
        .unwrap();

        for field in [
            "target.target_pids",
            "target.tree_pids",
            "target.cgroupv2",
            "target.exclude_tree_pids",
            "target.include_comm",
            "target.exclude_comm",
            "target.watch_process",
            "target.persistent",
            "target.keep_missing_pid",
            "target.max_tasks",
            "timing.summary_period_ms",
            "timing.epoch_period_ms",
            "timing.max_duration",
            "timing.spike_threshold_ns",
            "probes.irq_latency",
            "probes.irqs",
            "probes.hwmon",
            "hwmon.enabled",
            "probes.cpu_freq",
            "probes.faults",
            "probes.cpu_perf",
            "cpu_perf.enabled",
            "probes.block_io",
            "probes.stat_wait",
            "probes.runtime_slices",
            "runtime_slices.enabled",
            "recording.run_name",
            "recording.output_dir",
            "recording.retain_intervals",
            "recording.retention.max_run_count",
            "recording.retention.max_total_bytes",
            "recording.retention.max_age_seconds",
            "recording.retention.min_free_bytes",
            "outputs.json_stream",
            "streams.json_stream",
            "outputs.metrics_port",
            "outputs.otlp_endpoint",
            "outputs.otel_service_name",
            "focus.auto_focus",
            "focus.focus_source",
            "focus.foreground_window",
            "focus.foreground_source",
            "focus.foreground_poll_ms",
            "focus.foreground_max_stale_ms",
            "focus.foreground_include_title",
            "focus.auto_focus_poll_ms",
            "focus.auto_focus_min_confidence",
            "focus.auto_focus_switch_cooldown_ms",
            "focus.auto_focus_switch_margin",
            "focus.auto_focus_required_polls",
            "focus.auto_focus_max_roots",
            "safety.follow_exec",
            "safety.native_cgroup_filter",
            "watch.poll_ms",
            "watch.timeout",
            "alerts.threshold_ns",
            "alerts.webhook_url",
            "streams.csv",
            "streams.verbose",
            "hwmon.root",
            "hwmon.drm_card",
            "hwmon.render_node",
            "mangohud.log",
            "mangohud.log_live",
            "ui.tui",
            "cpu_perf.include_kernel",
            "cpu_perf.max_tasks",
            "cpu_perf.collect_cache_refs",
            "runtime_slices.max_tasks",
            "ebpf_sizing.ringbuf_size_kb",
            "ebpf_sizing.wakeup_map_factor",
            "remote.endpoint",
        ] {
            assert_eq!(
                last_source_for_field(&effective.provenance, field),
                Some(ConfigSource::Default),
                "missing default provenance for {field}"
            );
        }
    }

    #[test]
    fn resolve_monitor_config_sources_returns_full_precedence_provenance() {
        let user_file = crate::config_file::UserConfigFile {
            summary_period_ms: Some(250),
            ..Default::default()
        };
        let preset = crate::config::merge::PresetConfig {
            layer: MonitorConfigLayer {
                summary_period_ms: Some(500),
                ..Default::default()
            },
        };
        let api = crate::config::merge::ApiOverrides {
            layer: MonitorConfigLayer {
                summary_period_ms: Some(750),
                ..Default::default()
            },
        };

        let resolved = resolve_monitor_config_sources(crate::config::merge::ConfigSources {
            defaults: crate::config::merge::DefaultConfig::default(),
            user_file: Some(user_file),
            preset: Some(preset),
            overrides: api.into(),
        })
        .unwrap();

        assert_eq!(resolved.config.timing.summary_period_ms, 750);
        assert_eq!(
            last_source_for_field(&resolved.provenance, "timing.summary_period_ms"),
            Some(ConfigSource::Api)
        );

        let sources: Vec<_> = resolved
            .provenance
            .iter()
            .filter(|entry| entry.field == "timing.summary_period_ms")
            .map(|entry| entry.source)
            .collect();

        assert!(sources.contains(&ConfigSource::Default));
        assert!(sources.contains(&ConfigSource::UserFile));
        assert!(sources.contains(&ConfigSource::Preset));
        assert!(sources.contains(&ConfigSource::Api));
    }

    #[test]
    fn resolve_monitor_config_sources_returns_cli_precedence_provenance() {
        let user_file = crate::config_file::UserConfigFile {
            summary_period_ms: Some(250),
            ..Default::default()
        };
        let preset = crate::config::merge::PresetConfig {
            layer: MonitorConfigLayer {
                summary_period_ms: Some(500),
                ..Default::default()
            },
        };
        let cli = crate::config::merge::CliOverrides {
            layer: MonitorConfigLayer {
                summary_period_ms: Some(1_000),
                ..Default::default()
            },
        };

        let resolved = resolve_monitor_config_sources(crate::config::merge::ConfigSources {
            defaults: crate::config::merge::DefaultConfig::default(),
            user_file: Some(user_file),
            preset: Some(preset),
            overrides: cli.into(),
        })
        .unwrap();

        assert_eq!(resolved.config.timing.summary_period_ms, 1_000);
        assert_eq!(
            last_source_for_field(&resolved.provenance, "timing.summary_period_ms"),
            Some(ConfigSource::Cli)
        );

        let sources: Vec<_> = resolved
            .provenance
            .iter()
            .filter(|entry| entry.field == "timing.summary_period_ms")
            .map(|entry| entry.source)
            .collect();

        assert!(sources.contains(&ConfigSource::Default));
        assert!(sources.contains(&ConfigSource::UserFile));
        assert!(sources.contains(&ConfigSource::Preset));
        assert!(sources.contains(&ConfigSource::Cli));
    }

    #[test]
    fn resolve_monitor_config_sources_carries_user_file_diagnostics_and_provenance() {
        let mut user_file = crate::config_file::UserConfigFile {
            summary_period_ms: Some(250),
            ..Default::default()
        };
        user_file
            .diagnostics
            .push(crate::config::schema::ConfigDiagnostic::warning(
                crate::config::source::ConfigSource::UserFile,
                Some("summary_ms".to_owned()),
                "`summary_ms` is deprecated; use `summary_period_ms`",
            ));

        let resolved = resolve_monitor_config_sources(crate::config::merge::ConfigSources {
            defaults: crate::config::merge::DefaultConfig::default(),
            user_file: Some(user_file),
            preset: None,
            overrides: crate::config::merge::CliOverrides {
                layer: MonitorConfigLayer::default(),
            }
            .into(),
        })
        .unwrap();

        assert_eq!(resolved.config.timing.summary_period_ms, 250);
        assert_eq!(resolved.diagnostics.len(), 1);
        assert_eq!(resolved.diagnostics[0].field.as_deref(), Some("summary_ms"));
        assert_eq!(
            last_source_for_field(&resolved.provenance, "timing.summary_period_ms"),
            Some(ConfigSource::UserFile)
        );
    }
}
