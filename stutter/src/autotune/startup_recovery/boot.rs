//! Boot policy for durable recovery tokens.
use super::*;

/// Every token has an explicit boot policy. System resources delegate to their typed token
/// identity; unknown/legacy task state is never assumed to belong to this boot.
pub(super) fn task_boot_superseded(
    token: &RollbackToken,
    recorded: Option<&str>,
) -> anyhow::Result<bool> {
    match token {
        RollbackToken::CpuAffinityRestoreFile { .. }
        | RollbackToken::NiceRestore { .. }
        | RollbackToken::UclampRestore { .. }
        | RollbackToken::CgroupRestore { .. }
        | RollbackToken::IoPrioRestore { .. } => {
            let recorded =
                recorded.context("task rollback lacks boot identity; automatic replay refused")?;
            anyhow::ensure!(
                stutter_core::boot::valid_boot_id(recorded),
                "malformed rollback boot identity"
            );
            Ok(recorded != stutter_core::boot::current_boot_id()?)
        }
        RollbackToken::CpuPowerRestore { .. }
        | RollbackToken::IrqAffinityRestore { .. }
        | RollbackToken::GpuPowerRestore { .. }
        | RollbackToken::VmKnobRestore { .. }
        | RollbackToken::PmQosRelease { .. }
        | RollbackToken::ResctrlGroupRemove { .. }
        | RollbackToken::DamosTeardown { .. }
        | RollbackToken::CgroupControlRestore { .. }
        | RollbackToken::BlockSchedulerRestore { .. }
        | RollbackToken::ScxSchedulerRestore { .. }
        | RollbackToken::NumaMigration { .. }
        | RollbackToken::SysfsRestore { .. } => Ok(false),
    }
}
