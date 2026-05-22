use super::handler::{default_autotune_rollback_registry, rollback_restore_summary_from_registry_result};
use std::{mem, path::Path};
use anyhow::Context;
use crate::actions::*;
use super::{types::*, manual_command::*, helpers::*};

pub(super) const IOPRIO_WHO_PROCESS: libc::c_int = 1;
pub(super) const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
pub(super) const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
pub(super) const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;
pub(super) const SCHED_FLAG_UTIL_CLAMP_MAX: u64 = 0x40;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct SchedAttr {
    size: u32,
    sched_policy: u32,
    sched_flags: u64,
    sched_nice: i32,
    sched_priority: u32,
    sched_runtime: u64,
    sched_deadline: u64,
    sched_period: u64,
    sched_util_min: u32,
    sched_util_max: u32,
}

impl Default for SchedAttr {
    fn default() -> Self {
        Self {
            size: mem::size_of::<SchedAttr>() as u32,
            sched_policy: 0,
            sched_flags: 0,
            sched_nice: 0,
            sched_priority: 0,
            sched_runtime: 0,
            sched_deadline: 0,
            sched_period: 0,
            sched_util_min: 0,
            sched_util_max: 1024,
        }
    }
}
pub fn restore_rollback_token(token: &RollbackToken) -> anyhow::Result<RollbackRestoreSummary> {
    let result = default_autotune_rollback_registry().restore_token(token)?;
    Ok(rollback_restore_summary_from_registry_result(token, result))
}

pub fn restore_rollback_token_direct(token: &RollbackToken) -> anyhow::Result<RollbackRestoreSummary> {
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

pub(super) fn restore_nice_records(records: &[NiceRestoreRecord]) -> anyhow::Result<RollbackRestoreSummary> {
    restore_task_records(
        "nice-restore",
        records,
        |record| {
            let tid = record.tid();
            let rc = unsafe {
                libc::setpriority(
                    libc::PRIO_PROCESS,
                    tid as libc::id_t,
                    record.original_nice as libc::c_int,
                )
            };

            if rc == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        },
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
        |record| {
            let tid = record.tid();
            let rc = unsafe {
                libc::syscall(
                    libc::SYS_ioprio_set,
                    IOPRIO_WHO_PROCESS,
                    tid as libc::c_int,
                    record.original_ioprio as libc::c_int,
                )
            };

            if rc == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        },
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
            let tid = record.tid();
            let mut attr = SchedAttr {
                sched_flags: SCHED_FLAG_KEEP_POLICY
                    | SCHED_FLAG_KEEP_PARAMS
                    | SCHED_FLAG_UTIL_CLAMP_MIN
                    | SCHED_FLAG_UTIL_CLAMP_MAX,
                sched_util_min: record.original_util_min,
                sched_util_max: record.original_util_max,
                ..SchedAttr::default()
            };

            let rc = unsafe {
                libc::syscall(
                    libc::SYS_sched_setattr,
                    tid as libc::pid_t,
                    &mut attr as *mut SchedAttr,
                    0u32,
                )
            };

            if rc == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
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
    let mut restored = 0usize;

    for record in records {
        let tid = record.tid();
        let cgroup_procs = record.original_cgroup.join("cgroup.procs");
        write_sysfs_value(&cgroup_procs, &tid.to_string()).with_context(|| {
            format!(
                "failed to restore pid={} to cgroup {}",
                tid,
                record.original_cgroup.display()
            )
        })?;
        restored += 1;
    }

    Ok(RollbackRestoreSummary::success("cgroup-restore", restored))
}

pub(super) fn restore_cgroup_cpuset_record(record: &CgroupCpusetRestoreRecord) -> anyhow::Result<usize> {
    let mut restored = 0usize;
    if let Some(original) = &record.original_cpuset_cpus {
        write_sysfs_value(&record.cgroup_path.join("cpuset.cpus"), original).with_context(
            || {
                format!(
                    "failed to restore {}",
                    record.cgroup_path.join("cpuset.cpus").display()
                )
            },
        )?;
        restored += 1;
    }
    if let Some(original) = &record.original_cpuset_mems {
        write_sysfs_value(&record.cgroup_path.join("cpuset.mems"), original).with_context(
            || {
                format!(
                    "failed to restore {}",
                    record.cgroup_path.join("cpuset.mems").display()
                )
            },
        )?;
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

