use super::*;
use crate::config::{FocusSource, ForegroundSource, TARGET_PIDS_MAX};

fn last_source_for_field(
    provenance: &[stutter_config::source::FieldProvenance],
    field: &'static str,
) -> Option<ConfigSource> {
    provenance
        .iter()
        .rev()
        .find(|entry| entry.field == field)
        .map(|entry| entry.source)
}

fn trace_for_field<'a>(
    trace: &'a [stutter_config::source::ConfigMergeTrace],
    field: &'static str,
) -> Option<&'a stutter_config::source::ConfigMergeTrace> {
    trace.iter().find(|entry| entry.field == field)
}

fn assert_api_override_invalid_field(layer: MonitorConfigLayer, expected_field: &'static str) {
    let err = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: None,
        preset: None,
        overrides: ApiOverrides { layer }.into(),
    })
    .unwrap_err();

    match err {
        ConfigError::InvalidValue { field, .. } => assert_eq!(field, expected_field),
        other => panic!("expected InvalidValue for {expected_field}, got {other:?}"),
    }
}

mod validation;

mod ebpf_sizing;

mod precedence;

mod effective;

mod clearable;

mod mangohud;
