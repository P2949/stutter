use stutter_common::ebpf_capacity::{
    DEFAULT_PREV_FAULTS_PER_TARGET_MULTIPLIER, DEFAULT_RUNNABLE_TASK_CPU_PER_TARGET_MULTIPLIER,
    DEFAULT_TARGET_PIDS_MAP_MAX_ENTRIES,
};

use crate::config::{ConfigError, model::MonitorConfig};

const MAX_OPERATOR_MAP_ENTRIES: u32 = 1_048_576;
const MAX_TARGET_IRQS_ENTRIES: u32 = 65_536;
const MAX_TARGET_CGROUP_IDS_ENTRIES: u32 = 65_536;

pub(crate) fn validate_monitor_config(config: &MonitorConfig) -> Result<(), ConfigError> {
    validate_static_config(config)?;
    validate_runtime_config(config)?;

    Ok(())
}

fn validate_static_config(config: &MonitorConfig) -> Result<(), ConfigError> {
    stutter_config::validate_static_config(config).map_err(ConfigError::from)
}

fn validate_runtime_config(config: &MonitorConfig) -> Result<(), ConfigError> {
    require_nonzero_usize("cpu_perf.max_tasks", config.cpu_perf.max_tasks)?;
    require_nonzero_usize("runtime_slices.max_tasks", config.runtime_slices.max_tasks)?;

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

    validate_extended_ebpf_sizing(config)?;

    Ok(())
}

fn validate_extended_ebpf_sizing(config: &MonitorConfig) -> Result<(), ConfigError> {
    let sizing = &config.ebpf_sizing;

    require_optional_nonzero_u32(
        "ebpf_sizing.target_pids_entries",
        sizing.target_pids_entries,
    )?;
    require_optional_nonzero_u32(
        "ebpf_sizing.target_cgroup_ids_entries",
        sizing.target_cgroup_ids_entries,
    )?;
    require_optional_nonzero_u32(
        "ebpf_sizing.target_irqs_entries",
        sizing.target_irqs_entries,
    )?;
    require_optional_nonzero_u32(
        "ebpf_sizing.runnable_task_cpu_factor",
        sizing.runnable_task_cpu_factor,
    )?;
    require_optional_nonzero_u32("ebpf_sizing.prev_faults_factor", sizing.prev_faults_factor)?;
    require_optional_nonzero_u32("ebpf_sizing.irq_start_entries", sizing.irq_start_entries)?;
    require_optional_nonzero_u32(
        "ebpf_sizing.block_start_entries",
        sizing.block_start_entries,
    )?;
    require_optional_nonzero_u32(
        "ebpf_sizing.kms_flip_start_entries",
        sizing.kms_flip_start_entries,
    )?;
    require_optional_nonzero_u32(
        "ebpf_sizing.drm_fence_wait_start_entries",
        sizing.drm_fence_wait_start_entries,
    )?;
    require_optional_nonzero_u32(
        "ebpf_sizing.drm_fence_signal_entries",
        sizing.drm_fence_signal_entries,
    )?;

    require_optional_u32_at_most(
        "ebpf_sizing.target_pids_entries",
        sizing.target_pids_entries,
        MAX_OPERATOR_MAP_ENTRIES,
    )?;
    require_optional_u32_at_most(
        "ebpf_sizing.target_cgroup_ids_entries",
        sizing.target_cgroup_ids_entries,
        MAX_TARGET_CGROUP_IDS_ENTRIES,
    )?;
    require_optional_u32_at_most(
        "ebpf_sizing.target_irqs_entries",
        sizing.target_irqs_entries,
        MAX_TARGET_IRQS_ENTRIES,
    )?;
    require_optional_u32_at_most(
        "ebpf_sizing.irq_start_entries",
        sizing.irq_start_entries,
        MAX_OPERATOR_MAP_ENTRIES,
    )?;
    require_optional_u32_at_most(
        "ebpf_sizing.block_start_entries",
        sizing.block_start_entries,
        MAX_OPERATOR_MAP_ENTRIES,
    )?;
    require_optional_u32_at_most(
        "ebpf_sizing.kms_flip_start_entries",
        sizing.kms_flip_start_entries,
        MAX_OPERATOR_MAP_ENTRIES,
    )?;
    require_optional_u32_at_most(
        "ebpf_sizing.drm_fence_wait_start_entries",
        sizing.drm_fence_wait_start_entries,
        MAX_OPERATOR_MAP_ENTRIES,
    )?;
    require_optional_u32_at_most(
        "ebpf_sizing.drm_fence_signal_entries",
        sizing.drm_fence_signal_entries,
        MAX_OPERATOR_MAP_ENTRIES,
    )?;

    let max_tasks = u32::try_from(config.target.max_tasks).unwrap_or(u32::MAX);
    if let Some(target_pids_entries) = sizing.target_pids_entries
        && target_pids_entries < max_tasks
    {
        return invalid(
            "ebpf_sizing.target_pids_entries",
            "must be >= target.max_tasks",
        );
    }

    let target_entries = sizing
        .target_pids_entries
        .unwrap_or(DEFAULT_TARGET_PIDS_MAP_MAX_ENTRIES)
        .max(max_tasks);
    require_scaled_map_entries_at_most(
        "ebpf_sizing.runnable_task_cpu_factor",
        target_entries,
        sizing.runnable_task_cpu_factor,
        DEFAULT_RUNNABLE_TASK_CPU_PER_TARGET_MULTIPLIER,
    )?;
    require_scaled_map_entries_at_most(
        "ebpf_sizing.prev_faults_factor",
        target_entries,
        sizing.prev_faults_factor,
        DEFAULT_PREV_FAULTS_PER_TARGET_MULTIPLIER,
    )?;

    Ok(())
}

fn require_nonzero_usize(field: &'static str, value: usize) -> Result<(), ConfigError> {
    if value == 0 {
        invalid(field, "must be greater than zero")
    } else {
        Ok(())
    }
}

fn require_optional_nonzero_u32(
    field: &'static str,
    value: Option<u32>,
) -> Result<(), ConfigError> {
    if matches!(value, Some(0)) {
        invalid(field, "must be greater than zero")
    } else {
        Ok(())
    }
}

fn require_optional_u32_at_most(
    field: &'static str,
    value: Option<u32>,
    max: u32,
) -> Result<(), ConfigError> {
    if let Some(value) = value
        && value > max
    {
        invalid(field, format!("must be <= {max}"))
    } else {
        Ok(())
    }
}

fn require_scaled_map_entries_at_most(
    field: &'static str,
    target_entries: u32,
    configured_factor: Option<u32>,
    default_factor: u32,
) -> Result<(), ConfigError> {
    let factor = configured_factor.unwrap_or(default_factor);
    let entries = u64::from(target_entries).saturating_mul(u64::from(factor));

    if entries > u64::from(MAX_OPERATOR_MAP_ENTRIES) {
        invalid(
            field,
            format!("target_pids_entries * factor must be <= {MAX_OPERATOR_MAP_ENTRIES}"),
        )
    } else {
        Ok(())
    }
}

fn invalid<T>(field: &'static str, message: impl Into<String>) -> Result<T, ConfigError> {
    Err(ConfigError::InvalidValue {
        field: field.to_owned(),
        message: message.into(),
    })
}
