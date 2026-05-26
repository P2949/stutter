use crate::{TARGET_PIDS_MAX, error::ConfigError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticMonitorConfig<'a> {
    pub timing: StaticTimingConfig,
    pub diagnosis: StaticDiagnosisConfig,
    pub target: StaticTargetConfig,
    pub focus: StaticFocusConfig,
    pub watch: StaticWatchConfig,
    pub alerts: StaticAlertConfig,
    pub mangohud: StaticMangoHudConfig,
    pub outputs: StaticOutputConfig<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticTimingConfig {
    pub summary_period_ms: u64,
    pub epoch_period_ms: Option<u64>,
    pub spike_threshold_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticDiagnosisConfig {
    pub live_cluster_window_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticTargetConfig {
    pub max_tasks: usize,
}

impl StaticTargetConfig {
    pub const fn new(max_tasks: usize) -> Self {
        Self { max_tasks }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticFocusConfig {
    pub foreground_poll_ms: u64,
    pub auto_focus_poll_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticWatchConfig {
    pub poll_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticAlertConfig {
    pub threshold_ns: Option<u64>,
    pub desktop_timeout_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticMangoHudConfig {
    pub tail_idle_sleep_ms: u64,
    pub alignment_poll_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticOutputConfig<'a> {
    pub otel_service_name: &'a str,
    pub otlp_endpoint: Option<&'a str>,
}

pub fn validate_static_config(config: &StaticMonitorConfig<'_>) -> Result<(), ConfigError> {
    require_nonzero("timing.summary_period_ms", config.timing.summary_period_ms)?;
    require_nonzero(
        "timing.spike_threshold_ns",
        config.timing.spike_threshold_ns,
    )?;

    if matches!(config.timing.epoch_period_ms, Some(0)) {
        return invalid(
            "timing.epoch_period_ms",
            0,
            "must be greater than zero when set",
        );
    }

    require_nonzero(
        "diagnosis.live_cluster_window_ms",
        config.diagnosis.live_cluster_window_ms,
    )?;
    validate_target_max_tasks(config.target.max_tasks)?;

    require_nonzero("focus.foreground_poll_ms", config.focus.foreground_poll_ms)?;
    require_nonzero("focus.auto_focus_poll_ms", config.focus.auto_focus_poll_ms)?;
    require_nonzero("watch.poll_ms", config.watch.poll_ms)?;

    if let Some(threshold_ns) = config.alerts.threshold_ns {
        require_nonzero("alerts.threshold_ns", threshold_ns)?;
    }

    require_nonzero(
        "alerts.desktop_timeout_ms",
        config.alerts.desktop_timeout_ms,
    )?;
    require_at_most(
        "alerts.desktop_timeout_ms",
        config.alerts.desktop_timeout_ms,
        120_000,
    )?;

    require_nonzero(
        "mangohud.tail_idle_sleep_ms",
        config.mangohud.tail_idle_sleep_ms,
    )?;
    require_at_most(
        "mangohud.tail_idle_sleep_ms",
        config.mangohud.tail_idle_sleep_ms,
        5_000,
    )?;
    require_nonzero(
        "mangohud.alignment_poll_ms",
        config.mangohud.alignment_poll_ms,
    )?;
    require_at_most(
        "mangohud.alignment_poll_ms",
        config.mangohud.alignment_poll_ms,
        10_000,
    )?;

    if config.outputs.otel_service_name.trim().is_empty() {
        return invalid(
            "outputs.otel_service_name",
            config.outputs.otel_service_name,
            "must not be empty",
        );
    }

    if matches!(config.outputs.otlp_endpoint.map(str::trim), Some("")) {
        return invalid("outputs.otlp_endpoint", "", "must not be empty when set");
    }

    Ok(())
}

pub fn validate_target_max_tasks(max_tasks: usize) -> Result<(), ConfigError> {
    if max_tasks == 0 {
        return invalid("target.max_tasks", max_tasks, "must be greater than zero");
    }

    if max_tasks > TARGET_PIDS_MAX {
        return invalid(
            "target.max_tasks",
            max_tasks,
            "must not exceed TARGET_PIDS_MAX",
        );
    }

    Ok(())
}

fn require_nonzero(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value == 0 {
        invalid(field, value, "must be greater than zero")
    } else {
        Ok(())
    }
}

fn require_at_most(field: &'static str, value: u64, max: u64) -> Result<(), ConfigError> {
    if value > max {
        invalid(field, value, format!("must be <= {max}"))
    } else {
        Ok(())
    }
}

fn invalid<T>(
    field: &'static str,
    value: impl ToString,
    reason: impl Into<String>,
) -> Result<T, ConfigError> {
    Err(ConfigError::invalid_value(
        field,
        value.to_string(),
        reason.into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        StaticAlertConfig, StaticDiagnosisConfig, StaticFocusConfig, StaticMangoHudConfig,
        StaticMonitorConfig, StaticOutputConfig, StaticTargetConfig, StaticTimingConfig,
        StaticWatchConfig, validate_static_config, validate_target_max_tasks,
    };
    use crate::{TARGET_PIDS_MAX, error::ConfigError};

    fn valid_static_config() -> StaticMonitorConfig<'static> {
        StaticMonitorConfig {
            timing: StaticTimingConfig {
                summary_period_ms: 1_000,
                epoch_period_ms: None,
                spike_threshold_ns: 1_000_000,
            },
            diagnosis: StaticDiagnosisConfig {
                live_cluster_window_ms: 5,
            },
            target: StaticTargetConfig::new(TARGET_PIDS_MAX),
            focus: StaticFocusConfig {
                foreground_poll_ms: 500,
                auto_focus_poll_ms: 1_000,
            },
            watch: StaticWatchConfig { poll_ms: 2_000 },
            alerts: StaticAlertConfig {
                threshold_ns: None,
                desktop_timeout_ms: 10_000,
            },
            mangohud: StaticMangoHudConfig {
                tail_idle_sleep_ms: 75,
                alignment_poll_ms: 500,
            },
            outputs: StaticOutputConfig {
                otel_service_name: "stutter",
                otlp_endpoint: None,
            },
        }
    }

    fn invalid_field(error: ConfigError) -> String {
        match error {
            ConfigError::InvalidValue { field, .. } => field,
            other => panic!("expected invalid value error, got {other:?}"),
        }
    }

    #[test]
    fn static_config_accepts_default_like_values() {
        assert!(validate_static_config(&valid_static_config()).is_ok());
    }

    #[test]
    fn target_max_tasks_rejects_zero_and_shared_limit_overflow() {
        assert_eq!(
            invalid_field(validate_target_max_tasks(0).unwrap_err()),
            "target.max_tasks"
        );
        assert_eq!(
            invalid_field(validate_target_max_tasks(TARGET_PIDS_MAX + 1).unwrap_err()),
            "target.max_tasks"
        );
    }

    #[test]
    fn static_config_rejects_pure_scalar_invalid_values() {
        let mut config = valid_static_config();
        config.timing.summary_period_ms = 0;
        assert_eq!(
            invalid_field(validate_static_config(&config).unwrap_err()),
            "timing.summary_period_ms"
        );

        let mut config = valid_static_config();
        config.outputs.otel_service_name = " ";
        assert_eq!(
            invalid_field(validate_static_config(&config).unwrap_err()),
            "outputs.otel_service_name"
        );

        let mut config = valid_static_config();
        config.mangohud.alignment_poll_ms = 10_001;
        assert_eq!(
            invalid_field(validate_static_config(&config).unwrap_err()),
            "mangohud.alignment_poll_ms"
        );
    }
}
