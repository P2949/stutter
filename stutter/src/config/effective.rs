mod appliers;
mod provenance;

use crate::config::{
    ConfigError,
    layer::MonitorConfigLayer,
    merge::{self, ConfigSources},
    model::MonitorConfig,
    schema::ConfigDiagnostic,
    source::{ConfigSource, FieldProvenance},
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

        provenance::record_layer_provenance(
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
    provenance::record_layer_provenance(&layer, source, provenance);
    appliers::apply_config_layer(config, &layer);
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
