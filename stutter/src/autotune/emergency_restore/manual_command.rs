use super::executors::IOPRIO_WHO_PROCESS;
use std::path::{Path, PathBuf};
use crate::actions::{RollbackToken, SafetyClass};

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
            .map(|record| {
                format!(
                    "sudo renice -n {} -p {}",
                    record.original_nice,
                    record.tid()
                )
            })
            .collect::<Vec<_>>()
            .join(" && "),
        RollbackToken::IoPrioRestore { records } => records
            .iter()
            .map(|record| {
                format!(
                    "sudo python3 -c 'import os; os.syscall({},{},{},{})'",
                    libc::SYS_ioprio_set,
                    IOPRIO_WHO_PROCESS,
                    record.tid(),
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
        RollbackToken::CgroupRestore { records, cpuset } => {
            let mut commands = records
                .iter()
                .map(|record| {
                    format!(
                        "printf '%s' {} | sudo tee {}/cgroup.procs >/dev/null",
                        record.tid(),
                        shell_quote_path(&record.original_cgroup)
                    )
                })
                .collect::<Vec<_>>();
            if let Some(cpuset) = cpuset {
                if let Some(original) = &cpuset.original_cpuset_cpus {
                    commands.push(format!(
                        "printf '%s' {} | sudo tee {}/cpuset.cpus >/dev/null",
                        shell_quote_value(original),
                        shell_quote_path(&cpuset.cgroup_path)
                    ));
                }
                if let Some(original) = &cpuset.original_cpuset_mems {
                    commands.push(format!(
                        "printf '%s' {} | sudo tee {}/cpuset.mems >/dev/null",
                        shell_quote_value(original),
                        shell_quote_path(&cpuset.cgroup_path)
                    ));
                }
            }
            commands.join(" && ")
        }
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

pub(super) fn sysfs_manual_commands<'a>(records: impl Iterator<Item = (&'a PathBuf, &'a str)>) -> String {
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

pub(super) fn safety_class_for_rollback_token(token: &RollbackToken) -> SafetyClass {
    match token {
        RollbackToken::CpuAffinityRestoreFile { .. } => SafetyClass::ReversibleLowRisk,
        RollbackToken::NiceRestore { .. }
        | RollbackToken::IoPrioRestore { .. }
        | RollbackToken::UclampRestore { .. }
        | RollbackToken::IrqAffinityRestore { .. }
        | RollbackToken::CgroupRestore { .. } => SafetyClass::ReversibleMediumRisk,
        RollbackToken::CpuPowerRestore { .. }
        | RollbackToken::VmKnobRestore { .. }
        | RollbackToken::GpuPowerRestore { .. }
        | RollbackToken::SysfsRestore { .. } => SafetyClass::HighRisk,
    }
}

pub(super) fn rollback_token_kind(token: &RollbackToken) -> &'static str {
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

pub(super) fn action_kind_from_action_id(action_id: &str) -> String {
    let kind = action_id
        .split_once(':')
        .map(|(kind, _)| kind)
        .unwrap_or(action_id);

    kind.replace('-', "_")
}

pub(super) fn candidate_name_from_action_id(action_id: &str) -> Option<String> {
    action_id
        .split_once(':')
        .map(|(_, candidate)| candidate.to_owned())
        .filter(|candidate| !candidate.trim().is_empty())
}

pub(super) fn render_summary_messages(messages: &[String]) -> String {
    if messages.is_empty() {
        String::new()
    } else {
        format!(" messages={:?}", messages)
    }
}

pub(super) fn shell_quote_path(path: &Path) -> String {
    shell_quote_value(&path.display().to_string())
}

pub(super) fn shell_quote_value(value: &str) -> String {
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
