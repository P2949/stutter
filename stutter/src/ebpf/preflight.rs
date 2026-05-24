use std::path::Path;

use serde::Serialize;
use stutter_common::tracepoint_offsets::{
    CPU_FREQUENCY_FIELDS, IRQ_HANDLER_FIELDS, SCHED_MIGRATE_TASK_FIELDS, SCHED_STAT_WAIT_FIELDS,
    SCHED_SWITCH_FIELDS, SCHED_WAKEUP_FIELDS,
};

use crate::{
    drm_fence_tracepoints::DrmFenceTracepointDiscovery,
    drm_tracepoints::KmsTracepointAvailability,
    ebpf::{
        tracepoint_format::{
            require_tracepoint_field, validate_optional_tracepoint_format_at,
            validate_tracepoint_format_at, validate_tracepoint_format_at_named,
        },
        tracepoints::{
            block_io::{BlockIoTracepointOffsets, validate_block_io_tracepoint_offsets},
            drm_fence::drm_fence_tracepoint_offsets,
        },
    },
};

#[derive(Debug, Clone)]
pub struct TracepointAvailability {
    pub sched_wakeup_new: bool,
    pub sched_migrate_task: bool,
    pub cpu_frequency: bool,
    pub sched_stat_wait: bool,
    pub irq_handler: bool,
    pub block_rq: bool,
    pub block_rq_has_rwbs: bool,
    pub block_rq_key_offset: Option<u32>,
    pub block_rq_issue_nr_sector_offset: Option<u32>,
    pub block_rq_issue_rwbs_offset: Option<u32>,
    pub block_rq_complete_nr_sector_offset: Option<u32>,
    pub block_rq_complete_rwbs_offset: Option<u32>,
    pub kms: KmsTracepointAvailability,
    pub drm_fence: Option<DrmFenceTracepointDiscovery>,
    pub sched_process_exit: bool,
    pub sched_process_exec: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TracepointPreflightReport {
    pub sched_wakeup: String,
    pub sched_switch: String,
    pub sched_wakeup_new: String,
    pub sched_wakeup_new_coverage: String,
    pub sched_migrate_task: String,
    pub cpu_frequency: String,
    pub sched_stat_wait: String,
    pub irq_handler: String,
    pub block_rq: String,
    pub block_io_correlation_basis: String,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

pub fn tracepoint_preflight(
    events_root: &Path,
    wants_cpu_freq: bool,
    wants_stat_wait: bool,
    wants_irq_latency: bool,
    wants_block_io: bool,
    wants_follow_exec: bool,
) -> TracepointPreflightReport {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    let sched_wakeup = required_tracepoint_status(
        &events_root.join("sched/sched_wakeup/format"),
        SCHED_WAKEUP_FIELDS,
        "sched_wakeup",
        &mut errors,
    );
    let sched_switch = required_tracepoint_status(
        &events_root.join("sched/sched_switch/format"),
        SCHED_SWITCH_FIELDS,
        "sched_switch",
        &mut errors,
    );
    let sched_wakeup_new = optional_tracepoint_status(
        &events_root.join("sched/sched_wakeup_new/format"),
        SCHED_WAKEUP_FIELDS,
        "sched_wakeup_new",
        true,
        &mut warnings,
    );
    let sched_migrate_task = optional_tracepoint_status(
        &events_root.join("sched/sched_migrate_task/format"),
        SCHED_MIGRATE_TASK_FIELDS,
        "sched_migrate_task",
        true,
        &mut warnings,
    );
    let cpu_frequency = optional_tracepoint_status(
        &events_root.join("power/cpu_frequency/format"),
        CPU_FREQUENCY_FIELDS,
        "cpu_frequency",
        wants_cpu_freq,
        &mut warnings,
    );
    let sched_stat_wait = optional_tracepoint_status(
        &events_root.join("sched/sched_stat_wait/format"),
        SCHED_STAT_WAIT_FIELDS,
        "sched_stat_wait",
        wants_stat_wait,
        &mut warnings,
    );

    let irq_entry = events_root.join("irq/irq_handler_entry/format");
    let irq_exit = events_root.join("irq/irq_handler_exit/format");
    let irq_handler = if !wants_irq_latency {
        "not_requested".to_owned()
    } else if irq_entry.exists() && irq_exit.exists() {
        let entry_ok = validate_tracepoint_format_at_named(
            &irq_entry,
            "irq_handler_entry",
            IRQ_HANDLER_FIELDS,
        )
        .is_ok();
        let exit_ok =
            validate_tracepoint_format_at_named(&irq_exit, "irq_handler_exit", IRQ_HANDLER_FIELDS)
                .is_ok()
                && require_tracepoint_field(&irq_exit, "ret").is_ok();
        if entry_ok && exit_ok {
            "ok".to_owned()
        } else {
            warnings.push("IRQ tracepoint formats are present but layouts differ".to_owned());
            "mismatch".to_owned()
        }
    } else {
        warnings.push("IRQ tracepoint formats are missing".to_owned());
        "missing".to_owned()
    };

    let (block_rq, block_io_correlation_basis) =
        block_tracepoint_preflight(events_root, wants_block_io, &mut warnings);

    let sched_wakeup_new_coverage =
        sched_wakeup_new_coverage_status(&sched_wakeup_new, &mut warnings);

    if wants_follow_exec {
        let exec_path = events_root.join("sched/sched_process_exec/format");
        if !exec_path.exists() {
            warnings.push(
                "sched_process_exec tracepoint missing; follow-exec cleanup may be degraded"
                    .to_owned(),
            );
        }
    }

    TracepointPreflightReport {
        sched_wakeup,
        sched_switch,
        sched_wakeup_new,
        sched_wakeup_new_coverage,
        sched_migrate_task,
        cpu_frequency,
        sched_stat_wait,
        irq_handler,
        block_rq,
        block_io_correlation_basis,
        warnings,
        errors,
    }
}

fn required_tracepoint_status(
    path: &Path,
    expected_offsets: &[(&str, usize)],
    name: &str,
    errors: &mut Vec<String>,
) -> String {
    match validate_tracepoint_format_at_named(path, name, expected_offsets) {
        Ok(()) => "ok".to_owned(),
        Err(err) => {
            errors.push(format!(
                "{name} tracepoint unavailable or incompatible: {err:#}"
            ));
            if path.exists() {
                "mismatch".to_owned()
            } else {
                "missing".to_owned()
            }
        }
    }
}

fn optional_tracepoint_status(
    path: &Path,
    expected_offsets: &[(&str, usize)],
    name: &str,
    wanted: bool,
    warnings: &mut Vec<String>,
) -> String {
    if !path.exists() {
        if wanted {
            warnings.push(format!("{name} tracepoint format is missing"));
        }
        return "missing".to_owned();
    }

    match validate_tracepoint_format_at_named(path, name, expected_offsets) {
        Ok(()) => "ok".to_owned(),
        Err(err) => {
            if wanted {
                warnings.push(format!("{name} tracepoint layout differs: {err:#}"));
            }
            "mismatch".to_owned()
        }
    }
}

pub(crate) fn sched_wakeup_new_coverage_status(
    sched_wakeup_new_status: &str,
    warnings: &mut Vec<String>,
) -> String {
    match sched_wakeup_new_status {
        "ok" => "full".to_owned(),
        "not_requested" => "not_requested".to_owned(),
        _ => {
            warnings.push(
                "optional sched_wakeup_new tracepoint unavailable; sched_wakeup remains required and usable, but wakeups for newly created tasks may have reduced coverage"
                    .to_owned(),
            );
            "reduced-new-task-wakeup-coverage".to_owned()
        }
    }
}

fn block_tracepoint_preflight(
    events_root: &Path,
    wants_block_io: bool,
    warnings: &mut Vec<String>,
) -> (String, String) {
    let (block_rq, block_io_correlation_basis) = if !wants_block_io {
        ("not_requested".to_owned(), "not_requested".to_owned())
    } else {
        let offsets = validate_block_io_tracepoint_offsets(events_root);
        if offsets.block_rq {
            let basis = if offsets.block_rq_key_offset.is_some() {
                "request-pointer"
            } else {
                warnings.push(
                    "block I/O request-pointer key unavailable; dev+sector correlation is approximate"
                        .to_owned(),
                );
                "dev+sector"
            };
            ("ok".to_owned(), basis.to_owned())
        } else {
            // Error/warning already logged by validate_block_io_tracepoint_offsets
            ("missing".to_owned(), "unavailable".to_owned())
        }
    };

    (block_rq, block_io_correlation_basis)
}

pub(crate) fn validate_tracepoint_formats(
    events_root: &Path,
    config: &crate::config::model::MonitorConfig,
) -> anyhow::Result<TracepointAvailability> {
    validate_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup/format"),
        SCHED_WAKEUP_FIELDS,
    )?;
    let sched_wakeup_new = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_wakeup_new/format"),
        "sched_wakeup_new",
        SCHED_WAKEUP_FIELDS,
        true,
    )?;
    validate_tracepoint_format_at_named(
        &events_root.join("sched/sched_switch/format"),
        "sched_switch",
        SCHED_SWITCH_FIELDS,
    )?;

    let sched_migrate_task = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_migrate_task/format"),
        "sched_migrate_task",
        SCHED_MIGRATE_TASK_FIELDS,
        true,
    )?;
    let cpu_frequency = if config.probes.cpu_freq {
        validate_optional_tracepoint_format_at(
            &events_root.join("power/cpu_frequency/format"),
            "cpu_frequency",
            CPU_FREQUENCY_FIELDS,
            true,
        )?
    } else {
        false
    };
    let sched_stat_wait = if config.probes.stat_wait {
        validate_optional_tracepoint_format_at(
            &events_root.join("sched/sched_stat_wait/format"),
            "sched_stat_wait",
            SCHED_STAT_WAIT_FIELDS,
            true,
        )?
    } else {
        false
    };

    let irq_entry = events_root.join("irq/irq_handler_entry/format");
    let irq_exit = events_root.join("irq/irq_handler_exit/format");
    let irq_handler = if config.probes.irq_latency && irq_entry.exists() && irq_exit.exists() {
        validate_tracepoint_format_at_named(&irq_entry, "irq_handler_entry", IRQ_HANDLER_FIELDS)?;
        validate_tracepoint_format_at_named(&irq_exit, "irq_handler_exit", IRQ_HANDLER_FIELDS)?;

        // Validation-only for now. The eBPF program does not currently read
        // irq_handler_exit.ret, but the field must exist for kernels where IRQ exit
        // semantics are expected by the IRQ tracing path.
        let _ret_offset = require_tracepoint_field(&irq_exit, "ret")?;

        true
    } else {
        false
    };
    if !irq_handler && config.probes.irq_latency {
        log::warn!("IRQ tracepoint formats missing; continuing without IRQ latency probe");
    }

    let block_io = if config.probes.block_io {
        validate_block_io_tracepoint_offsets(events_root)
    } else {
        BlockIoTracepointOffsets::default()
    };

    let block_rq = block_io.block_rq;
    let block_rq_has_rwbs = block_io.block_rq_has_rwbs;
    let block_rq_key_offset = block_io.block_rq_key_offset;
    let block_rq_issue_nr_sector_offset = block_io.block_rq_issue_nr_sector_offset;
    let block_rq_issue_rwbs_offset = block_io.block_rq_issue_rwbs_offset;
    let block_rq_complete_nr_sector_offset = block_io.block_rq_complete_nr_sector_offset;
    let block_rq_complete_rwbs_offset = block_io.block_rq_complete_rwbs_offset;

    let kms = if config.probes.kms_timing {
        crate::drm_tracepoints::discover_kms_tracepoints(events_root)
    } else {
        KmsTracepointAvailability::unavailable()
    };
    if config.probes.kms_timing && !kms.has_selected_tracepoints() {
        log::warn!("KMS timing tracepoints missing; continuing without KMS pageflip timing probe");
    } else if config.probes.kms_timing && !kms.selected_provider_has_required_fields() {
        log::warn!(
            "KMS timing tracepoints found, but selected provider fields are not sufficient for pageflip/vblank correlation"
        );
    }

    let drm_fence = if config.probes.drm_fence_latency {
        Some(crate::drm_fence_tracepoints::discover_drm_fence_tracepoints(events_root))
    } else {
        None
    };
    if config.probes.drm_fence_latency
        && !drm_fence
            .as_ref()
            .and_then(drm_fence_tracepoint_offsets)
            .is_some()
    {
        log::warn!(
            "DRM fence tracepoints missing or lack stable identity fields; continuing without DRM fence latency probe"
        );
    }

    let sched_process_exit = validate_optional_tracepoint_format_at(
        &events_root.join("sched/sched_process_exit/format"),
        "sched_process_exit",
        &[],
        true,
    )?;

    let sched_process_exec = events_root.join("sched/sched_process_exec/format");
    let sched_process_exec = if config.safety.follow_exec && sched_process_exec.exists() {
        validate_tracepoint_format_at_named(&sched_process_exec, "sched_process_exec", &[])?;
        true
    } else {
        false
    };

    Ok(TracepointAvailability {
        sched_wakeup_new,
        sched_migrate_task,
        cpu_frequency,
        sched_stat_wait,
        irq_handler,
        block_rq,
        block_rq_has_rwbs,
        block_rq_key_offset,
        block_rq_issue_nr_sector_offset,
        block_rq_issue_rwbs_offset,
        block_rq_complete_nr_sector_offset,
        block_rq_complete_rwbs_offset,
        kms,
        drm_fence,
        sched_process_exit,
        sched_process_exec,
    })
}
