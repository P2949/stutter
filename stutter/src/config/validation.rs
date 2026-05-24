use crate::config::{ConfigError, TARGET_PIDS_MAX, model::MonitorConfig};

pub(crate) fn validate_monitor_config(config: &MonitorConfig) -> Result<(), ConfigError> {
    require_nonzero("timing.summary_period_ms", config.timing.summary_period_ms)?;
    require_nonzero(
        "timing.spike_threshold_ns",
        config.timing.spike_threshold_ns,
    )?;

    if matches!(config.timing.epoch_period_ms, Some(0)) {
        return invalid(
            "timing.epoch_period_ms",
            "must be greater than zero when set",
        );
    }

    require_nonzero(
        "diagnosis.live_cluster_window_ms",
        config.diagnosis.live_cluster_window_ms,
    )?;
    require_nonzero_usize("target.max_tasks", config.target.max_tasks)?;

    if config.target.max_tasks > TARGET_PIDS_MAX {
        return invalid("target.max_tasks", "must not exceed TARGET_PIDS_MAX");
    }

    require_nonzero("focus.foreground_poll_ms", config.focus.foreground_poll_ms)?;
    require_nonzero("focus.auto_focus_poll_ms", config.focus.auto_focus_poll_ms)?;
    require_nonzero("watch.poll_ms", config.watch.poll_ms)?;
    require_nonzero_usize("cpu_perf.max_tasks", config.cpu_perf.max_tasks)?;
    require_nonzero_usize("runtime_slices.max_tasks", config.runtime_slices.max_tasks)?;

    if let Some(threshold_ns) = config.alerts.threshold_ns {
        require_nonzero("alerts.threshold_ns", threshold_ns)?;
    }

    if let Some(kb) = config.ebpf_sizing.ringbuf_size_kb
        && !(64..=16 * 1024).contains(&kb)
    {
        return invalid(
            "ebpf_sizing.ringbuf_size_kb",
            "must be between 64 and 16384",
        );
    }

    if let Some(factor) = config.ebpf_sizing.wakeup_map_factor
        && (factor == 0 || factor > 64)
    {
        return invalid("ebpf_sizing.wakeup_map_factor", "must be between 1 and 64");
    }

    if config.outputs.otel_service_name.trim().is_empty() {
        return invalid("outputs.otel_service_name", "must not be empty");
    }

    if matches!(
        config.outputs.otlp_endpoint.as_deref().map(str::trim),
        Some("")
    ) {
        return invalid("outputs.otlp_endpoint", "must not be empty when set");
    }

    Ok(())
}

fn require_nonzero(field: &'static str, value: u64) -> Result<(), ConfigError> {
    if value == 0 {
        invalid(field, "must be greater than zero")
    } else {
        Ok(())
    }
}

fn require_nonzero_usize(field: &'static str, value: usize) -> Result<(), ConfigError> {
    if value == 0 {
        invalid(field, "must be greater than zero")
    } else {
        Ok(())
    }
}

fn invalid<T>(field: &'static str, message: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::InvalidValue {
        field,
        message: message.into(),
    })
}
