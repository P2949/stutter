use super::*;

#[test]
fn target_pids_entries_must_cover_target_max_tasks() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            max_tasks: Some(256),
            target_pids_entries: Some(Some(128)),
            ..Default::default()
        },
        "ebpf_sizing.target_pids_entries",
    );
}

#[test]
fn ebpf_sizing_rejects_absurd_large_correlation_maps() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            block_start_entries: Some(Some(1_048_577)),
            ..Default::default()
        },
        "ebpf_sizing.block_start_entries",
    );
}

#[test]
fn ebpf_sizing_rejects_absurd_large_scaled_maps() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            target_pids_entries: Some(Some(2_048)),
            runnable_task_cpu_factor: Some(Some(1_000)),
            ..Default::default()
        },
        "ebpf_sizing.runnable_task_cpu_factor",
    );
}

#[test]
fn ebpf_sizing_block_start_entries_runtime_override_wins() {
    let user_file = crate::config_file::UserConfigFile {
        ebpf_sizing: Some(crate::config_file::EbpfSizingConfigFile {
            block_start_entries: Some(16_384),
            ..Default::default()
        }),
        ..Default::default()
    };
    let preset = PresetConfig {
        layer: MonitorConfigLayer {
            block_start_entries: Some(Some(32_768)),
            ..Default::default()
        },
    };
    let cli = CliOverrides {
        layer: MonitorConfigLayer {
            block_start_entries: Some(Some(65_536)),
            ..Default::default()
        },
    };

    let effective = merge_config_sources_effective_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: Some(preset),
        overrides: cli.into(),
    })
    .unwrap();

    assert_eq!(
        effective.config.ebpf_sizing.block_start_entries,
        Some(65_536)
    );
    assert_eq!(
        last_source_for_field(&effective.provenance, "ebpf_sizing.block_start_entries"),
        Some(ConfigSource::Cli)
    );
}

#[test]
fn ebpf_sizing_target_irqs_entries_user_config_is_preserved_without_override() {
    let user_file = crate::config_file::UserConfigFile {
        ebpf_sizing: Some(crate::config_file::EbpfSizingConfigFile {
            target_irqs_entries: Some(256),
            ..Default::default()
        }),
        ..Default::default()
    };

    let effective = merge_config_sources_effective_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: None,
        overrides: CliOverrides {
            layer: MonitorConfigLayer::default(),
        }
        .into(),
    })
    .unwrap();

    assert_eq!(effective.config.ebpf_sizing.target_irqs_entries, Some(256));
    assert_eq!(
        last_source_for_field(&effective.provenance, "ebpf_sizing.target_irqs_entries"),
        Some(ConfigSource::UserFile)
    );
}
