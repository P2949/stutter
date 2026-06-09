use std::path::Path;

use anyhow::Context;

use super::{
    handler::{default_autotune_rollback_registry, rollback_restore_summary_from_registry_result},
    helpers::*,
    manual_command::*,
    types::*,
};
use crate::actions::{self, *};

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

pub fn restore_rollback_token(token: &RollbackToken) -> anyhow::Result<RollbackRestoreSummary> {
    let result = default_autotune_rollback_registry().restore_token(token)?;
    Ok(rollback_restore_summary_from_registry_result(token, result))
}

pub fn restore_rollback_token_direct(
    token: &RollbackToken,
) -> anyhow::Result<RollbackRestoreSummary> {
    match token {
        RollbackToken::CpuAffinityRestoreFile { path, .. } => {
            if crate::profile_restore::load_restore_state(path).is_ok() {
                let summary = crate::profile_restore::restore_saved(path).with_context(|| {
                    format!("failed to restore profile state from {}", path.display())
                })?;

                return Ok(RollbackRestoreSummary {
                    rollback_kind: rollback_token_kind(token).to_owned(),
                    restored_items: summary.restored_total(),
                    skipped_items: summary.skipped_dead + summary.skipped_identity_mismatch,
                    skipped_missing: summary.skipped_dead,
                    skipped_identity_mismatch: summary.skipped_identity_mismatch,
                    failed_items: summary.errors,
                    messages: vec![format!(
                        "affinity={} nice={} ionice={} skipped_dead={} skipped_identity_mismatch={} errors={}",
                        summary.affinity,
                        summary.nice,
                        summary.ionice,
                        summary.skipped_dead,
                        summary.skipped_identity_mismatch,
                        summary.errors
                    )],
                });
            }

            let summary = crate::affinity::restore_saved(path).with_context(|| {
                format!("failed to restore CPU affinity from {}", path.display())
            })?;

            Ok(RollbackRestoreSummary {
                rollback_kind: rollback_token_kind(token).to_owned(),
                restored_items: summary.restored,
                skipped_items: summary.skipped_dead
                    + summary.skipped_identity_mismatch
                    + summary.legacy_unverified,
                skipped_missing: summary.skipped_dead,
                skipped_identity_mismatch: summary.skipped_identity_mismatch,
                failed_items: summary.errors,
                messages: vec![format!(
                    "restored={} skipped_dead={} skipped_identity_mismatch={} legacy_unverified={} errors={}",
                    summary.restored,
                    summary.skipped_dead,
                    summary.skipped_identity_mismatch,
                    summary.legacy_unverified,
                    summary.errors
                )],
            })
        }
        RollbackToken::NiceRestore { records } => restore_nice_records(records),
        RollbackToken::IoPrioRestore { records } => restore_ioprio_records(records),
        RollbackToken::UclampRestore { records } => restore_uclamp_records(records),
        RollbackToken::IrqAffinityRestore { records } => {
            restore_irq_affinity_records_at(Path::new("/proc/irq"), records)
        }
        RollbackToken::CgroupRestore { records, cpuset } => {
            restore_cgroup_token(records, cpuset.as_ref())
        }
        RollbackToken::CpuPowerRestore { records } => restore_cpu_power_records(records),
        RollbackToken::VmKnobRestore { records } => restore_vm_knob_records(records),
        RollbackToken::GpuPowerRestore { records } => restore_gpu_power_records(records),
        RollbackToken::SysfsRestore {
            path,
            original_value,
        } => {
            write_sysfs_value(path, original_value).with_context(|| {
                format!(
                    "failed to restore sysfs value {} to {:?}",
                    path.display(),
                    original_value
                )
            })?;

            Ok(RollbackRestoreSummary::success(
                rollback_token_kind(token),
                1,
            ))
        }
    }
}

pub(super) fn restore_task_records<T, Apply, Describe>(
    rollback_kind: &'static str,
    records: &[T],
    mut apply: Apply,
    mut describe_failure: Describe,
) -> anyhow::Result<RollbackRestoreSummary>
where
    Apply: FnMut(&T) -> Result<(), std::io::Error>,
    Describe: FnMut(&T) -> String,
{
    let mut restored = 0usize;
    let mut skipped_missing = 0usize;

    for record in records {
        match apply(record) {
            Ok(()) => {
                restored += 1;
            }
            Err(err) if is_missing_task_error(&err) => {
                skipped_missing += 1;
            }
            Err(err) => {
                return Err(err).with_context(|| describe_failure(record));
            }
        }
    }

    Ok(restore_summary_with_missing_skips(
        rollback_kind,
        restored,
        skipped_missing,
    ))
}

pub(super) fn restore_nice_records(
    records: &[NiceRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    restore_task_records(
        "nice-restore",
        records,
        |record| crate::actions::syscalls::setpriority_process(record.tid(), record.original_nice),
        |record| {
            format!(
                "failed to restore nice={} for tid={}",
                record.original_nice,
                record.tid()
            )
        },
    )
}

pub(super) fn restore_ioprio_records(
    records: &[IoPrioRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    restore_task_records(
        "ioprio-restore",
        records,
        |record| crate::actions::syscalls::ioprio_set_process(record.tid(), record.original_ioprio),
        |record| {
            format!(
                "failed to restore I/O priority={} for tid={}",
                record.original_ioprio,
                record.tid()
            )
        },
    )
}

pub(super) fn restore_uclamp_records(
    records: &[UclampRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    restore_task_records(
        "uclamp-restore",
        records,
        |record| {
            crate::actions::syscalls::sched_setattr(
                record.tid(),
                crate::actions::syscalls::SchedUclamp {
                    util_min: record.original_util_min,
                    util_max: record.original_util_max,
                },
            )
        },
        |record| {
            format!(
                "failed to restore uclamp min={} max={} for tid={}",
                record.original_util_min,
                record.original_util_max,
                record.tid()
            )
        },
    )
}

pub(super) fn restore_irq_affinity_records_at(
    irq_root: &Path,
    records: &[IrqAffinityRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    let mut restored = 0usize;
    let mut skipped = 0usize;
    let mut messages = Vec::new();

    for record in records {
        let irq_dir = irq_root.join(record.irq.to_string());
        if !irq_dir.is_dir() {
            skipped += 1;
            messages.push(format!("IRQ {} directory disappeared", record.irq));
            continue;
        }

        let actual_device_hint = read_irq_device_hint(&irq_dir).with_context(|| {
            format!(
                "failed to read current IRQ device hint for IRQ {} during emergency restore",
                record.irq
            )
        })?;

        if actual_device_hint != record.device_hint {
            skipped += 1;
            messages.push(format!(
                "IRQ {} device mapping changed: expected {:?}, actual {:?}",
                record.irq, record.device_hint, actual_device_hint
            ));
            continue;
        }

        let path = irq_dir.join("smp_affinity");
        write_sysfs_value(&path, &record.original_smp_affinity).with_context(|| {
            format!(
                "failed to restore IRQ {} smp_affinity via {}",
                record.irq,
                path.display()
            )
        })?;
        restored += 1;
    }

    Ok(RollbackRestoreSummary {
        rollback_kind: "irq-affinity-restore".to_owned(),
        restored_items: restored,
        skipped_items: skipped,
        skipped_missing: 0,
        skipped_identity_mismatch: skipped,
        failed_items: 0,
        messages,
    })
}

pub(super) fn restore_cgroup_token(
    records: &[CgroupRestoreRecord],
    cpuset: Option<&CgroupCpusetRestoreRecord>,
) -> anyhow::Result<RollbackRestoreSummary> {
    let mut summary = restore_cgroup_records(records)?;
    if let Some(cpuset) = cpuset {
        let restored_cpuset_files = restore_cgroup_cpuset_record(cpuset)?;
        summary.restored_items = summary.restored_items.saturating_add(restored_cpuset_files);
        if restored_cpuset_files > 0 {
            summary.messages.push(format!(
                "restored {restored_cpuset_files} cgroup cpuset file(s)"
            ));
        }
    }
    Ok(summary)
}

pub(super) fn restore_cgroup_records(
    records: &[CgroupRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    restore_cgroup_records_at(Path::new(CGROUP_ROOT), records)
}

pub(super) fn restore_cgroup_records_at(
    cgroup_root: &Path,
    records: &[CgroupRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    let mut restored = 0usize;

    for record in records {
        let tid = record.tid();
        let original_cgroup = actions::cgroup::cgroup_fs_path(cgroup_root, &record.original_cgroup)
            .with_context(|| {
                format!(
                    "failed to resolve original cgroup path {}",
                    record.original_cgroup.display()
                )
            })?;
        let cgroup_procs = original_cgroup.join("cgroup.procs");
        write_sysfs_value(&cgroup_procs, &tid.to_string()).with_context(|| {
            format!(
                "failed to restore pid={} to cgroup {}",
                tid,
                original_cgroup.display()
            )
        })?;
        restored += 1;
    }

    Ok(RollbackRestoreSummary::success("cgroup-restore", restored))
}

pub(super) fn restore_cgroup_cpuset_record(
    record: &CgroupCpusetRestoreRecord,
) -> anyhow::Result<usize> {
    restore_cgroup_cpuset_record_at(Path::new(CGROUP_ROOT), record)
}

pub(super) fn restore_cgroup_cpuset_record_at(
    cgroup_root: &Path,
    record: &CgroupCpusetRestoreRecord,
) -> anyhow::Result<usize> {
    let cgroup_path = actions::cgroup::cgroup_fs_path(cgroup_root, &record.cgroup_path)
        .with_context(|| {
            format!(
                "failed to resolve cgroup cpuset restore path {}",
                record.cgroup_path.display()
            )
        })?;
    let mut restored = 0usize;
    if let Some(original) = &record.original_cpuset_cpus {
        write_sysfs_value(&cgroup_path.join("cpuset.cpus"), original).with_context(|| {
            format!(
                "failed to restore {}",
                cgroup_path.join("cpuset.cpus").display()
            )
        })?;
        restored += 1;
    }
    if let Some(original) = &record.original_cpuset_mems {
        write_sysfs_value(&cgroup_path.join("cpuset.mems"), original).with_context(|| {
            format!(
                "failed to restore {}",
                cgroup_path.join("cpuset.mems").display()
            )
        })?;
        restored += 1;
    }
    Ok(restored)
}

pub(super) fn restore_cpu_power_records(
    records: &[CpuPowerRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    let mut restored = 0usize;

    for record in records {
        write_sysfs_value(&record.path, &record.original_value).with_context(|| {
            format!(
                "failed to restore CPU power sysfs value {} to {:?}",
                record.path.display(),
                record.original_value
            )
        })?;
        restored += 1;
    }

    Ok(RollbackRestoreSummary::success(
        "cpu-power-restore",
        restored,
    ))
}

pub(super) fn restore_vm_knob_records(
    records: &[VmKnobRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    let mut restored = 0usize;

    for record in records {
        write_sysfs_value(&record.path, &record.original_value).with_context(|| {
            format!(
                "failed to restore VM knob {} to {:?}",
                record.path.display(),
                record.original_value
            )
        })?;
        restored += 1;
    }

    Ok(RollbackRestoreSummary::success("vm-knob-restore", restored))
}

pub(super) fn restore_gpu_power_records(
    records: &[GpuPowerRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    let mut restored = 0usize;

    for record in records {
        write_sysfs_value(&record.path, &record.original_value).with_context(|| {
            format!(
                "failed to restore GPU power sysfs value {} to {:?}",
                record.path.display(),
                record.original_value
            )
        })?;
        restored += 1;
    }

    Ok(RollbackRestoreSummary::success(
        "gpu-power-restore",
        restored,
    ))
}
