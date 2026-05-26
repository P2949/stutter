use std::path::PathBuf;

use anyhow::{Context, Result};
use stutter_config::source::ConfigSource;

use super::*;
use crate::{
    config::{
        FocusSource, ForegroundSource,
        schema::{CURRENT_CONFIG_VERSION, ConfigDiagnostic, ParsedUserConfigFile, RawConfigFile},
    },
    error::ConfigError,
};

pub fn load_user_config() -> Result<Option<UserConfigFile>> {
    let Some(path) = resolve_user_config_path() else {
        return Ok(None);
    };

    if !path.exists() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;

    let parsed = parse_user_config_toml_versioned(&contents)
        .map_err(|err| anyhow::anyhow!(err))
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    validate_daemon_user_config(&parsed.file)
        .with_context(|| format!("failed to validate daemon config in {}", path.display()))?;

    for diagnostic in &parsed.diagnostics {
        log::warn!(
            "config_file_diagnostic source={:?} level={:?} field={:?} message={}",
            diagnostic.source,
            diagnostic.level,
            diagnostic.field,
            diagnostic.message
        );
    }

    Ok(Some(parsed.file))
}

pub fn parse_focus_source_value(value: &str) -> std::result::Result<FocusSource, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "heuristic" => Ok(FocusSource::Heuristic),
        "foreground" => Ok(FocusSource::Foreground),
        "hybrid" => Ok(FocusSource::Hybrid),
        other => Err(ConfigError::InvalidValue {
            field: "focus_source".to_owned(),
            message: format!("got {other:?}; valid values are heuristic, foreground, hybrid"),
        }),
    }
}

pub fn parse_foreground_source_value(
    value: &str,
) -> std::result::Result<ForegroundSource, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(ForegroundSource::Auto),
        "sway" => Ok(ForegroundSource::Sway),
        "hyprland" => Ok(ForegroundSource::Hyprland),
        "x11" => Ok(ForegroundSource::X11),
        other => Err(ConfigError::InvalidValue {
            field: "foreground_source".to_owned(),
            message: format!("got {other:?}; valid values are auto, sway, hyprland, x11"),
        }),
    }
}

#[cfg(test)]
pub fn parse_user_config_toml(contents: &str) -> Result<UserConfigFile> {
    Ok(parse_user_config_toml_versioned(contents)?.file)
}

pub fn parse_user_config_toml_versioned(
    contents: &str,
) -> std::result::Result<ParsedUserConfigFile, ConfigError> {
    let raw = raw_user_config_file(contents)?;
    let version = raw.config_version.unwrap_or(1);

    if version > CURRENT_CONFIG_VERSION {
        return Err(ConfigError::UnsupportedConfigVersion {
            version,
            current: CURRENT_CONFIG_VERSION,
        });
    }

    let file = toml::from_str::<UserConfigFile>(contents)
        .map_err(|err| ConfigError::InvalidUserConfigToml(anyhow::Error::new(err)))?;

    let mut diagnostics = schema_diagnostics(&raw);
    let mut file = migrate_user_config_file(version, file, &mut diagnostics)?;
    diagnostics.extend(workload_policy_config_diagnostics(&file));
    file.config_version = Some(version);
    file.diagnostics = diagnostics.clone();

    Ok(ParsedUserConfigFile::new(version, file, diagnostics))
}

pub(super) fn raw_user_config_file(
    contents: &str,
) -> std::result::Result<RawConfigFile, ConfigError> {
    let flattened = toml::from_str::<toml::Value>(contents)
        .map_err(|err| ConfigError::InvalidUserConfigToml(anyhow::Error::new(err)))?;
    let config_version = config_version_from_raw_value(&flattened)?;

    Ok(RawConfigFile {
        config_version,
        flattened,
    })
}

pub(super) fn config_version_from_raw_value(
    value: &toml::Value,
) -> std::result::Result<Option<u32>, ConfigError> {
    let Some(table) = value.as_table() else {
        return Err(ConfigError::InvalidConfigVersion {
            message: "config file root must be a TOML table".to_owned(),
        });
    };

    let config_version = optional_config_version_field(table, "config_version")?;
    let schema_version = optional_config_version_field(table, "schema_version")?;

    match (config_version, schema_version) {
        (Some(config_version), Some(schema_version)) if config_version != schema_version => {
            Err(ConfigError::InvalidConfigVersion {
                message: format!(
                    "config_version ({config_version}) and schema_version ({schema_version}) must match"
                ),
            })
        }
        (Some(version), _) | (_, Some(version)) => Ok(Some(version)),
        (None, None) => Ok(None),
    }
}

fn optional_config_version_field(
    table: &toml::map::Map<String, toml::Value>,
    field: &'static str,
) -> std::result::Result<Option<u32>, ConfigError> {
    match table.get(field) {
        None => Ok(None),
        Some(toml::Value::Integer(value)) if (1..=u32::MAX as i64).contains(value) => {
            Ok(Some(*value as u32))
        }
        Some(toml::Value::Integer(value)) => Err(ConfigError::InvalidConfigVersion {
            message: format!("{field} must be a positive u32, got {value}"),
        }),
        Some(value) => Err(ConfigError::InvalidConfigVersion {
            message: format!("{field} must be an integer, got {value:?}"),
        }),
    }
}

fn migrate_user_config_file(
    version: u32,
    file: UserConfigFile,
    _diagnostics: &mut Vec<ConfigDiagnostic>,
) -> std::result::Result<UserConfigFile, ConfigError> {
    match version {
        1 => Ok(file),
        _ => Err(ConfigError::UnsupportedConfigVersion {
            version,
            current: CURRENT_CONFIG_VERSION,
        }),
    }
}

pub(super) fn schema_diagnostics(raw: &RawConfigFile) -> Vec<ConfigDiagnostic> {
    let Some(table) = raw.flattened.as_table() else {
        return Vec::new();
    };

    let mut diagnostics = Vec::new();

    if table.contains_key("summary_ms") {
        diagnostics.push(ConfigDiagnostic::warning(
            ConfigSource::UserFile,
            Some("summary_ms".to_owned()),
            "`summary_ms` is deprecated; use `summary_period_ms`",
        ));
    }

    if table.contains_key("spike_us") {
        diagnostics.push(ConfigDiagnostic::warning(
            ConfigSource::UserFile,
            Some("spike_us".to_owned()),
            "`spike_us` is deprecated; use `spike_threshold_ns`",
        ));
    }

    for key in table.keys() {
        if !known_top_level_user_config_field(key) {
            diagnostics.push(ConfigDiagnostic::warning(
                ConfigSource::UserFile,
                Some(key.clone()),
                format!("unknown top-level config field `{key}` will be ignored"),
            ));
        }
    }

    diagnostics
}

pub(super) fn known_top_level_user_config_field(field: &str) -> bool {
    matches!(
        field,
        "config_version"
            | "schema_version"
            | "experimental"
            | "summary_ms"
            | "summary_period_ms"
            | "spike_us"
            | "spike_threshold_ns"
            | "hwmon"
            | "cpu_freq"
            | "no_cpu_freq"
            | "include_comm"
            | "exclude_comm"
            | "max_tasks"
            | "retain_intervals"
            | "retention_max_run_count"
            | "retention_max_total_bytes"
            | "retention_max_age_seconds"
            | "retention_min_free_bytes"
            | "foreground_window"
            | "focus_source"
            | "foreground_source"
            | "foreground_poll_ms"
            | "foreground_max_stale_ms"
            | "foreground_include_title"
            | "dmabuf_tracking"
            | "dmabuf_log"
            | "gpu_engine_sampling"
            | "display_topology"
            | "daemon_preset"
            | "daemon_enabled_action_families"
            | "daemon_denied_action_families"
            | "daemon_interactive_cgroup"
            | "daemon_background_cgroup"
            | "daemon_game_cgroup"
            | "daemon_compile_cgroup"
            | "daemon_min_confidence"
            | "daemon_min_suggest_confidence"
            | "daemon_min_apply_low_risk_confidence"
            | "daemon_min_apply_medium_risk_confidence"
            | "daemon_min_high_risk_suggestion_confidence"
            | "daemon_max_cpu_temp_celsius"
            | "daemon_max_gpu_temp_celsius"
            | "daemon_min_disk_available_bytes"
            | "daemon_max_memory_pressure_some_avg10_percent"
            | "daemon_allow_system_wide_suggestions"
            | "daemon_allow_system_wide_apply"
            | "daemon_allow_high_risk"
            | "daemon_allow_medium_risk_apply"
            | "system_wide_allowlist"
            | "autotune"
            | "ebpf_sizing"
            | "mangohud"
            | "alerts"
            | "community_rules"
            | "agent"
    )
}

pub fn resolve_user_config_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("STUTTER_CONFIG")
        && !path.trim().is_empty()
    {
        return Some(PathBuf::from(path));
    }

    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.trim().is_empty()
    {
        return Some(PathBuf::from(xdg).join("stutter").join("config.toml"));
    }

    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return Some(
            PathBuf::from(home)
                .join(".config")
                .join("stutter")
                .join("config.toml"),
        );
    }

    None
}
