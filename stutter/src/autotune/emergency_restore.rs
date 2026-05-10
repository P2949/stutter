use std::{
    fs, mem,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::{
    actions::{
        CgroupRestoreRecord, CpuPowerRestoreRecord, GpuPowerRestoreRecord, IoPrioRestoreRecord,
        IrqAffinityRestoreRecord, NiceRestoreRecord, RollbackToken, SafetyClass,
        UclampRestoreRecord, VmKnobRestoreRecord,
    },
    audit::{AuditEvent, append_audit_event_to_path},
    autotune::{
        controller_journal::{
            ControllerJournalRecord, default_controller_journal_path, read_controller_journal,
            write_controller_journal_clean,
        },
        history::{
            AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneMode, ControllerPhase,
            ObservationSummary, SituationKind, append_autotune_history_event,
            default_autotune_history_path,
        },
    },
};

const IOPRIO_WHO_PROCESS: libc::c_int = 1;
const SCHED_FLAG_KEEP_POLICY: u64 = 0x08;
const SCHED_FLAG_KEEP_PARAMS: u64 = 0x10;
const SCHED_FLAG_UTIL_CLAMP_MIN: u64 = 0x20;
const SCHED_FLAG_UTIL_CLAMP_MAX: u64 = 0x40;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SchedAttr {
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

#[derive(Clone, Debug)]
pub struct AutotuneRestoreCommandInput {
    pub journal_path: Option<PathBuf>,
    pub audit_path: Option<PathBuf>,
    pub history_path: Option<PathBuf>,
    pub dry_run: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutotuneRestoreOutcome {
    pub status: AutotuneRestoreStatus,
    pub restored_actions: usize,
    pub failed_actions: usize,
    pub skipped_actions: usize,
    pub messages: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutotuneRestoreStatus {
    Clean,
    ApplyingWithoutRollbackToken,
    DryRun,
    Restored,
    Faulted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackRestoreSummary {
    pub rollback_kind: String,
    pub restored_items: usize,
    pub skipped_items: usize,
    pub messages: Vec<String>,
}

impl RollbackRestoreSummary {
    pub fn success(rollback_kind: impl Into<String>, restored_items: usize) -> Self {
        Self {
            rollback_kind: rollback_kind.into(),
            restored_items,
            skipped_items: 0,
            messages: Vec::new(),
        }
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.messages.push(message.into());
        self
    }
}

pub fn autotune_restore_command(input: AutotuneRestoreCommandInput) -> anyhow::Result<()> {
    let outcome = restore_known_autotune_actions(input)?;

    for message in &outcome.messages {
        println!("{message}");
    }

    if outcome.status == AutotuneRestoreStatus::Faulted {
        anyhow::bail!(
            "autotune emergency restore failed: restored_actions={} failed_actions={} skipped_actions={}",
            outcome.restored_actions,
            outcome.failed_actions,
            outcome.skipped_actions
        );
    }

    Ok(())
}

pub fn restore_known_autotune_actions(
    input: AutotuneRestoreCommandInput,
) -> anyhow::Result<AutotuneRestoreOutcome> {
    let journal_path = input
        .journal_path
        .unwrap_or_else(default_controller_journal_path);
    let audit_path = input
        .audit_path
        .unwrap_or_else(crate::audit::default_audit_log_path);
    let history_path = input
        .history_path
        .unwrap_or_else(default_autotune_history_path);

    let record = read_controller_journal(&journal_path)?;

    match record {
        ControllerJournalRecord::Clean { .. } => Ok(AutotuneRestoreOutcome {
            status: AutotuneRestoreStatus::Clean,
            restored_actions: 0,
            failed_actions: 0,
            skipped_actions: 0,
            messages: vec![format!(
                "autotune restore: no active autotune action in {}",
                journal_path.display()
            )],
        }),
        ControllerJournalRecord::Applying {
            experiment_id,
            action_id,
            ..
        } => Ok(AutotuneRestoreOutcome {
            status: AutotuneRestoreStatus::ApplyingWithoutRollbackToken,
            restored_actions: 0,
            failed_actions: 0,
            skipped_actions: 1,
            messages: vec![format!(
                "autotune restore: journal is applying without rollback_token experiment_id={} action_id={}; no automatic restore is possible",
                experiment_id, action_id
            )],
        }),
        ControllerJournalRecord::Applied {
            experiment_id,
            action_id,
            rollback_token,
            ..
        } => restore_applied_journal_record(
            &journal_path,
            &audit_path,
            &history_path,
            &experiment_id,
            &action_id,
            &rollback_token,
            input.dry_run,
        ),
    }
}

fn restore_applied_journal_record(
    journal_path: &Path,
    audit_path: &Path,
    history_path: &Path,
    experiment_id: &str,
    action_id: &str,
    rollback_token: &RollbackToken,
    dry_run: bool,
) -> anyhow::Result<AutotuneRestoreOutcome> {
    let manual_command = manual_restore_command_for_token(rollback_token);
    let rollback_kind = rollback_token_kind(rollback_token);

    if dry_run {
        return Ok(AutotuneRestoreOutcome {
            status: AutotuneRestoreStatus::DryRun,
            restored_actions: 0,
            failed_actions: 0,
            skipped_actions: 1,
            messages: vec![
                format!(
                    "autotune restore dry-run: would restore experiment_id={} action_id={} rollback_kind={}",
                    experiment_id, action_id, rollback_kind
                ),
                format!("manual_restore_command=\"{}\"", manual_command),
            ],
        });
    }

    match restore_rollback_token(rollback_token) {
        Ok(summary) => {
            write_controller_journal_clean(journal_path).with_context(|| {
                format!(
                    "failed to clear controller journal after emergency restore {}",
                    journal_path.display()
                )
            })?;

            write_emergency_restore_audit_event(
                audit_path,
                action_id,
                rollback_token,
                true,
                summary.restored_items,
                format!(
                    "autotune emergency restore succeeded experiment_id={} action_id={} rollback_kind={} restored_items={} skipped_items={} manual_restore_command=\"{}\"{}",
                    experiment_id,
                    action_id,
                    summary.rollback_kind,
                    summary.restored_items,
                    summary.skipped_items,
                    manual_command,
                    render_summary_messages(&summary.messages)
                ),
            )?;

            write_emergency_restore_history_event(
                history_path,
                ControllerPhase::Cooldown,
                "restored",
                experiment_id,
                action_id,
                true,
                format!(
                    "autotune emergency restore succeeded rollback_kind={} restored_items={} skipped_items={}",
                    summary.rollback_kind, summary.restored_items, summary.skipped_items
                ),
            )?;

            Ok(AutotuneRestoreOutcome {
                status: AutotuneRestoreStatus::Restored,
                restored_actions: 1,
                failed_actions: 0,
                skipped_actions: 0,
                messages: vec![format!(
                    "autotune restore: restored experiment_id={} action_id={} rollback_kind={} restored_items={}",
                    experiment_id, action_id, summary.rollback_kind, summary.restored_items
                )],
            })
        }
        Err(err) => {
            let reason = format!("{err:#}");
            write_emergency_restore_audit_event(
                audit_path,
                action_id,
                rollback_token,
                false,
                0,
                format!(
                    "autotune emergency restore failed experiment_id={} action_id={} rollback_kind={} error={} manual_restore_command=\"{}\"",
                    experiment_id, action_id, rollback_kind, reason, manual_command
                ),
            )?;

            write_emergency_restore_history_event(
                history_path,
                ControllerPhase::Faulted,
                "EmergencyRestoreFault",
                experiment_id,
                action_id,
                false,
                format!(
                    "autotune emergency restore failed rollback_kind={} error={} manual_restore_command=\"{}\"",
                    rollback_kind, reason, manual_command
                ),
            )?;

            Ok(AutotuneRestoreOutcome {
                status: AutotuneRestoreStatus::Faulted,
                restored_actions: 0,
                failed_actions: 1,
                skipped_actions: 0,
                messages: vec![
                    format!(
                        "autotune restore failed: experiment_id={} action_id={} rollback_kind={} error={}",
                        experiment_id, action_id, rollback_kind, reason
                    ),
                    format!("manual_restore_command=\"{}\"", manual_command),
                ],
            })
        }
    }
}

pub fn restore_rollback_token(token: &RollbackToken) -> anyhow::Result<RollbackRestoreSummary> {
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
        RollbackToken::CgroupRestore { records } => restore_cgroup_records(records),
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

fn restore_nice_records(records: &[NiceRestoreRecord]) -> anyhow::Result<RollbackRestoreSummary> {
    let mut restored = 0usize;

    for record in records {
        let rc = unsafe {
            libc::setpriority(
                libc::PRIO_PROCESS,
                record.tid as libc::id_t,
                record.original_nice as libc::c_int,
            )
        };

        if rc != 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "failed to restore nice={} for tid={}",
                    record.original_nice, record.tid
                )
            });
        }

        restored += 1;
    }

    Ok(RollbackRestoreSummary::success("nice-restore", restored))
}

fn restore_ioprio_records(
    records: &[IoPrioRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    let mut restored = 0usize;

    for record in records {
        let rc = unsafe {
            libc::syscall(
                libc::SYS_ioprio_set,
                IOPRIO_WHO_PROCESS,
                record.tid as libc::c_int,
                record.original_ioprio as libc::c_int,
            )
        };

        if rc != 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "failed to restore I/O priority={} for tid={}",
                    record.original_ioprio, record.tid
                )
            });
        }

        restored += 1;
    }

    Ok(RollbackRestoreSummary::success("ioprio-restore", restored))
}

fn restore_uclamp_records(
    records: &[UclampRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    let mut restored = 0usize;

    for record in records {
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
                record.tid as libc::pid_t,
                &mut attr as *mut SchedAttr,
                0u32,
            )
        };

        if rc != 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "failed to restore uclamp min={} max={} for tid={}",
                    record.original_util_min, record.original_util_max, record.tid
                )
            });
        }

        restored += 1;
    }

    Ok(RollbackRestoreSummary::success("uclamp-restore", restored))
}

fn restore_irq_affinity_records_at(
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
        messages,
    })
}

fn restore_cgroup_records(
    records: &[CgroupRestoreRecord],
) -> anyhow::Result<RollbackRestoreSummary> {
    let mut restored = 0usize;

    for record in records {
        let cgroup_procs = record.original_cgroup.join("cgroup.procs");
        write_sysfs_value(&cgroup_procs, &record.pid.to_string()).with_context(|| {
            format!(
                "failed to restore pid={} to cgroup {}",
                record.pid,
                record.original_cgroup.display()
            )
        })?;
        restored += 1;
    }

    Ok(RollbackRestoreSummary::success("cgroup-restore", restored))
}

fn restore_cpu_power_records(
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

fn restore_vm_knob_records(
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

fn restore_gpu_power_records(
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

fn write_sysfs_value(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value).with_context(|| format!("failed to write {}", path.display()))
}

fn read_irq_device_hint(irq_dir: &Path) -> anyhow::Result<String> {
    let actions_path = irq_dir.join("actions");
    if let Ok(value) = read_trimmed(&actions_path)
        && !value.is_empty()
    {
        return Ok(value);
    }

    let name_path = irq_dir.join("name");
    if let Ok(value) = read_trimmed(&name_path)
        && !value.is_empty()
    {
        return Ok(value);
    }

    anyhow::bail!(
        "neither {} nor {} contained a device hint",
        actions_path.display(),
        name_path.display()
    )
}

fn read_trimmed(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .with_context(|| format!("failed to read {}", path.display()))
}

fn write_emergency_restore_audit_event(
    audit_path: &Path,
    action_id: &str,
    rollback_token: &RollbackToken,
    success: bool,
    affected_tasks: usize,
    message: String,
) -> anyhow::Result<()> {
    let event = AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "autotune emergency restore".to_owned(),
        action_id: Some(action_id.to_owned()),
        safety_class: Some(safety_class_for_rollback_token(rollback_token)),
        dry_run: false,
        success,
        affected_tasks,
        restore_path: rollback_token.restore_path().cloned(),
        action_phase: None,
        error_category: None,
        message,
    };

    append_audit_event_to_path(audit_path, &event).with_context(|| {
        format!(
            "failed to write emergency restore audit event to {}",
            audit_path.display()
        )
    })
}

fn write_emergency_restore_history_event(
    history_path: &Path,
    phase: ControllerPhase,
    decision: &str,
    experiment_id: &str,
    action_id: &str,
    rollback_performed: bool,
    reason: String,
) -> anyhow::Result<()> {
    let event = AutotuneHistoryEvent::new(
        "emergency-restore",
        phase,
        AutotuneMode::ApplyLowRisk,
        None,
        SituationKind::Unknown,
        empty_observation_summary(),
        AutotuneDecisionSummary {
            decision: decision.to_owned(),
            candidate_name: candidate_name_from_action_id(action_id),
            action_kind: Some(action_kind_from_action_id(action_id)),
            eligible: rollback_performed,
            rollback_policy: "emergency-restore".to_owned(),
        },
        reason,
    )
    .with_experiment_id(experiment_id.to_owned())
    .with_action_id(action_id.to_owned())
    .with_rollback_performed(rollback_performed);

    append_autotune_history_event(history_path, &event).with_context(|| {
        format!(
            "failed to write emergency restore history event to {}",
            history_path.display()
        )
    })
}

fn empty_observation_summary() -> ObservationSummary {
    ObservationSummary {
        target_present: false,
        active_target_count: 0,
        scored_task_count: 0,
        interval_count: 0,
        scored_samples: 0,
        score_total: 0,
        over_1ms: 0,
        over_2ms: 0,
        over_5ms: 0,
        frame_p99_ms: 0.0,
        frame_max_ms: 0.0,
        drop_counter_total: 0,
        data_quality: "Unknown".to_owned(),
    }
}

pub fn manual_restore_command_for_token(token: &RollbackToken) -> String {
    match token {
        RollbackToken::CpuAffinityRestoreFile { path, .. } => {
            let default_path = crate::affinity::default_restore_path();
            if path == &default_path {
                "stutter restore".to_owned()
            } else {
                format!(
                    "cp -- {} {} && stutter restore",
                    shell_quote_path(path),
                    shell_quote_path(&default_path)
                )
            }
        }
        RollbackToken::NiceRestore { records } => records
            .iter()
            .map(|record| format!("sudo renice -n {} -p {}", record.original_nice, record.tid))
            .collect::<Vec<_>>()
            .join(" && "),
        RollbackToken::IoPrioRestore { records } => records
            .iter()
            .map(|record| {
                format!(
                    "sudo python3 -c 'import os; os.syscall({},{},{},{})'",
                    libc::SYS_ioprio_set,
                    IOPRIO_WHO_PROCESS,
                    record.tid,
                    record.original_ioprio
                )
            })
            .collect::<Vec<_>>()
            .join(" && "),
        RollbackToken::UclampRestore { .. } => {
            "rerun stutter autotune restore; manual uclamp restore requires sched_setattr"
                .to_owned()
        }
        RollbackToken::IrqAffinityRestore { records } => records
            .iter()
            .map(|record| {
                format!(
                    "printf '%s' {} | sudo tee /proc/irq/{}/smp_affinity >/dev/null",
                    shell_quote_value(&record.original_smp_affinity),
                    record.irq
                )
            })
            .collect::<Vec<_>>()
            .join(" && "),
        RollbackToken::CgroupRestore { records } => records
            .iter()
            .map(|record| {
                format!(
                    "printf '%s' {} | sudo tee {}/cgroup.procs >/dev/null",
                    record.pid,
                    shell_quote_path(&record.original_cgroup)
                )
            })
            .collect::<Vec<_>>()
            .join(" && "),
        RollbackToken::CpuPowerRestore { records } => sysfs_manual_commands(
            records
                .iter()
                .map(|record| (&record.path, record.original_value.as_str())),
        ),
        RollbackToken::VmKnobRestore { records } => sysfs_manual_commands(
            records
                .iter()
                .map(|record| (&record.path, record.original_value.as_str())),
        ),
        RollbackToken::GpuPowerRestore { records } => sysfs_manual_commands(
            records
                .iter()
                .map(|record| (&record.path, record.original_value.as_str())),
        ),
        RollbackToken::SysfsRestore {
            path,
            original_value,
        } => format!(
            "printf '%s' {} | sudo tee {} >/dev/null",
            shell_quote_value(original_value),
            shell_quote_path(path)
        ),
    }
}

fn sysfs_manual_commands<'a>(records: impl Iterator<Item = (&'a PathBuf, &'a str)>) -> String {
    records
        .map(|(path, original_value)| {
            format!(
                "printf '%s' {} | sudo tee {} >/dev/null",
                shell_quote_value(original_value),
                shell_quote_path(path)
            )
        })
        .collect::<Vec<_>>()
        .join(" && ")
}

fn safety_class_for_rollback_token(token: &RollbackToken) -> SafetyClass {
    match token {
        RollbackToken::CpuAffinityRestoreFile { .. } => SafetyClass::ReversibleLowRisk,
        RollbackToken::NiceRestore { .. }
        | RollbackToken::IoPrioRestore { .. }
        | RollbackToken::UclampRestore { .. }
        | RollbackToken::IrqAffinityRestore { .. } => SafetyClass::ReversibleMediumRisk,
        RollbackToken::CgroupRestore { .. }
        | RollbackToken::CpuPowerRestore { .. }
        | RollbackToken::VmKnobRestore { .. }
        | RollbackToken::GpuPowerRestore { .. }
        | RollbackToken::SysfsRestore { .. } => SafetyClass::HighRisk,
    }
}

fn rollback_token_kind(token: &RollbackToken) -> &'static str {
    match token {
        RollbackToken::CpuAffinityRestoreFile { .. } => "cpu-affinity-restore-file",
        RollbackToken::NiceRestore { .. } => "nice-restore",
        RollbackToken::IrqAffinityRestore { .. } => "irq-affinity-restore",
        RollbackToken::IoPrioRestore { .. } => "ioprio-restore",
        RollbackToken::UclampRestore { .. } => "uclamp-restore",
        RollbackToken::CgroupRestore { .. } => "cgroup-restore",
        RollbackToken::CpuPowerRestore { .. } => "cpu-power-restore",
        RollbackToken::VmKnobRestore { .. } => "vm-knob-restore",
        RollbackToken::GpuPowerRestore { .. } => "gpu-power-restore",
        RollbackToken::SysfsRestore { .. } => "sysfs-restore",
    }
}

fn action_kind_from_action_id(action_id: &str) -> String {
    let kind = action_id
        .split_once(':')
        .map(|(kind, _)| kind)
        .unwrap_or(action_id);

    kind.replace('-', "_")
}

fn candidate_name_from_action_id(action_id: &str) -> Option<String> {
    action_id
        .split_once(':')
        .map(|(_, candidate)| candidate.to_owned())
        .filter(|candidate| !candidate.trim().is_empty())
}

fn render_summary_messages(messages: &[String]) -> String {
    if messages.is_empty() {
        String::new()
    } else {
        format!(" messages={:?}", messages)
    }
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote_value(&path.display().to_string())
}

fn shell_quote_value(value: &str) -> String {
    if value.is_empty() {
        return "''".to_owned();
    }

    if value.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-' | '.' | ':' | '+' | ',' | '=')
    }) {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::{
            ActionId, CpuPowerRestoreRecord, GpuPowerRestoreRecord, IrqAffinityRestoreRecord,
            NiceRestoreRecord, VmKnobRestoreRecord,
        },
        autotune::controller_journal::{
            read_controller_journal, write_controller_journal_applied,
            write_controller_journal_applying, write_controller_journal_clean,
        },
    };

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-emergency-restore-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn command_input_for_dir(dir: &Path, dry_run: bool) -> AutotuneRestoreCommandInput {
        AutotuneRestoreCommandInput {
            journal_path: Some(dir.join("controller_journal.json")),
            audit_path: Some(dir.join("audit.jsonl")),
            history_path: Some(dir.join("history.jsonl")),
            dry_run,
        }
    }

    #[test]
    fn clean_journal_reports_no_active_action() {
        let dir = temp_dir("clean");
        let input = command_input_for_dir(&dir, false);
        write_controller_journal_clean(input.journal_path.as_deref().unwrap()).unwrap();

        let outcome = restore_known_autotune_actions(input).unwrap();

        assert_eq!(outcome.status, AutotuneRestoreStatus::Clean);
        assert_eq!(outcome.restored_actions, 0);
        assert!(outcome.messages[0].contains("no active autotune action"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn applying_journal_reports_no_rollback_token() {
        let dir = temp_dir("applying");
        let input = command_input_for_dir(&dir, false);
        write_controller_journal_applying(
            input.journal_path.as_deref().unwrap(),
            "experiment-1",
            "cpu-affinity-profile:game-main",
        )
        .unwrap();

        let outcome = restore_known_autotune_actions(input).unwrap();

        assert_eq!(
            outcome.status,
            AutotuneRestoreStatus::ApplyingWithoutRollbackToken
        );
        assert_eq!(outcome.skipped_actions, 1);
        assert!(outcome.messages[0].contains("without rollback_token"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn dry_run_for_applied_journal_does_not_clean_journal() {
        let dir = temp_dir("dry-run");
        let input = command_input_for_dir(&dir, true);
        let journal_path = input.journal_path.clone().unwrap();
        write_controller_journal_applied(
            &journal_path,
            "experiment-1",
            "nice:set:5:targets:1",
            RollbackToken::NiceRestore {
                records: vec![NiceRestoreRecord {
                    tid: 123,
                    original_nice: 0,
                }],
            },
        )
        .unwrap();

        let outcome = restore_known_autotune_actions(input).unwrap();

        assert_eq!(outcome.status, AutotuneRestoreStatus::DryRun);
        assert_eq!(outcome.skipped_actions, 1);
        assert!(
            outcome
                .messages
                .iter()
                .any(|message| { message.contains("sudo renice -n 0 -p 123") })
        );
        assert!(!read_controller_journal(&journal_path).unwrap().is_clean());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sysfs_restore_token_restores_file_and_cleans_journal_and_writes_logs() {
        let dir = temp_dir("sysfs");
        let target = dir.join("sysfs-knob");
        fs::write(&target, "changed").unwrap();

        let input = command_input_for_dir(&dir, false);
        let journal_path = input.journal_path.clone().unwrap();
        let audit_path = input.audit_path.clone().unwrap();
        let history_path = input.history_path.clone().unwrap();

        write_controller_journal_applied(
            &journal_path,
            "experiment-1",
            "sysfs-restore:test",
            RollbackToken::SysfsRestore {
                path: target.clone(),
                original_value: "original".to_owned(),
            },
        )
        .unwrap();

        let outcome = restore_known_autotune_actions(input).unwrap();

        assert_eq!(outcome.status, AutotuneRestoreStatus::Restored);
        assert_eq!(outcome.restored_actions, 1);
        assert_eq!(fs::read_to_string(&target).unwrap(), "original");
        assert!(read_controller_journal(&journal_path).unwrap().is_clean());

        let audit = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(audit.len(), 1);
        assert!(audit[0].success);
        assert_eq!(audit[0].command, "autotune emergency restore");

        let history =
            crate::autotune::history::read_autotune_history_events(&history_path).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].phase, ControllerPhase::Cooldown);
        assert_eq!(history[0].decision.decision, "restored");
        assert!(history[0].rollback_performed);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sysfs_restore_failure_keeps_journal_and_writes_fault_logs() {
        let dir = temp_dir("sysfs-failure");
        let missing_parent = dir.join("missing");
        let target = missing_parent.join("knob");

        let input = command_input_for_dir(&dir, false);
        let journal_path = input.journal_path.clone().unwrap();
        let audit_path = input.audit_path.clone().unwrap();
        let history_path = input.history_path.clone().unwrap();

        write_controller_journal_applied(
            &journal_path,
            "experiment-1",
            "sysfs-restore:test",
            RollbackToken::SysfsRestore {
                path: target.clone(),
                original_value: "original".to_owned(),
            },
        )
        .unwrap();

        let outcome = restore_known_autotune_actions(input).unwrap();

        assert_eq!(outcome.status, AutotuneRestoreStatus::Faulted);
        assert_eq!(outcome.failed_actions, 1);
        assert!(!read_controller_journal(&journal_path).unwrap().is_clean());

        let audit = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(audit.len(), 1);
        assert!(!audit[0].success);

        let history =
            crate::autotune::history::read_autotune_history_events(&history_path).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].phase, ControllerPhase::Faulted);
        assert_eq!(history[0].decision.decision, "EmergencyRestoreFault");
        assert!(!history[0].rollback_performed);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn restore_rollback_token_supports_all_sysfs_record_collections() {
        let dir = temp_dir("record-collections");
        let cpu_path = dir.join("cpu");
        let vm_path = dir.join("vm");
        let gpu_path = dir.join("gpu");
        fs::write(&cpu_path, "bad").unwrap();
        fs::write(&vm_path, "bad").unwrap();
        fs::write(&gpu_path, "bad").unwrap();

        restore_rollback_token(&RollbackToken::CpuPowerRestore {
            records: vec![CpuPowerRestoreRecord {
                path: cpu_path.clone(),
                original_value: "cpu-original".to_owned(),
            }],
        })
        .unwrap();
        restore_rollback_token(&RollbackToken::VmKnobRestore {
            records: vec![VmKnobRestoreRecord {
                path: vm_path.clone(),
                original_value: "vm-original".to_owned(),
            }],
        })
        .unwrap();
        restore_rollback_token(&RollbackToken::GpuPowerRestore {
            records: vec![GpuPowerRestoreRecord {
                path: gpu_path.clone(),
                original_value: "gpu-original".to_owned(),
            }],
        })
        .unwrap();

        assert_eq!(fs::read_to_string(cpu_path).unwrap(), "cpu-original");
        assert_eq!(fs::read_to_string(vm_path).unwrap(), "vm-original");
        assert_eq!(fs::read_to_string(gpu_path).unwrap(), "gpu-original");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn irq_affinity_restore_skips_when_irq_identity_changes() {
        let dir = temp_dir("irq-skip");
        let irq_root = dir.join("proc_irq");
        let irq_dir = irq_root.join("44");
        fs::create_dir_all(&irq_dir).unwrap();
        fs::write(irq_dir.join("actions"), "nvme\n").unwrap();
        fs::write(irq_dir.join("smp_affinity"), "00000002\n").unwrap();

        let records = vec![IrqAffinityRestoreRecord {
            irq: 44,
            device_hint: "amdgpu".to_owned(),
            original_smp_affinity: "00000001".to_owned(),
        }];

        let summary = restore_irq_affinity_records_at(&irq_root, &records).unwrap();

        assert_eq!(summary.restored_items, 0);
        assert_eq!(summary.skipped_items, 1);
        assert!(summary.messages[0].contains("device mapping changed"));
        assert_eq!(
            fs::read_to_string(irq_dir.join("smp_affinity")).unwrap(),
            "00000002\n"
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn manual_commands_cover_non_cpu_affinity_tokens() {
        let nice = manual_restore_command_for_token(&RollbackToken::NiceRestore {
            records: vec![NiceRestoreRecord {
                tid: 7,
                original_nice: 3,
            }],
        });
        assert_eq!(nice, "sudo renice -n 3 -p 7");

        let sysfs = manual_restore_command_for_token(&RollbackToken::SysfsRestore {
            path: PathBuf::from("/tmp/example knob"),
            original_value: "auto".to_owned(),
        });
        assert!(sysfs.contains("'/tmp/example knob'"));
        assert!(sysfs.contains("auto"));
    }

    #[test]
    fn action_id_helpers_extract_kind_and_candidate_name() {
        assert_eq!(
            action_kind_from_action_id("cpu-affinity-profile:game-main"),
            "cpu_affinity_profile"
        );
        assert_eq!(
            candidate_name_from_action_id("cpu-affinity-profile:game-main"),
            Some("game-main".to_owned())
        );
        assert_eq!(candidate_name_from_action_id("sysfs-restore"), None);
        assert_eq!(ActionId("test".to_owned()).0, "test");
    }
}
