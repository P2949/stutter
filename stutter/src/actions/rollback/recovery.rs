//! Startup permission is independent of manual handler availability.
use crate::actions::RollbackToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomaticRecoveryCapability {
    VerifiedAutomatic,
    ManualOnly,
    NotRecoverable,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RollbackRecoveryDescriptor {
    pub token_kind: &'static str,
    pub automatic_recovery: AutomaticRecoveryCapability,
    pub identity_contract: &'static str,
}

pub(crate) fn recovery_descriptor(token: &RollbackToken) -> RollbackRecoveryDescriptor {
    use AutomaticRecoveryCapability::*;
    // No wildcard: adding a token requires an explicit recovery decision.
    let (automatic_recovery, identity_contract) = match token {
        RollbackToken::CpuAffinityRestoreFile { .. } => (
            VerifiedAutomatic,
            "supported restore layout; complete nonzero process/task IDs and start times; entire batch validated before mutation; stale identities skipped",
        ),
        RollbackToken::NiceRestore { .. } => (
            VerifiedAutomatic,
            "complete process/task IDs and task start times from the prepared snapshot; each task re-verified immediately before its write; exited or replaced tasks skipped; legacy records without a start time are never restored automatically",
        ),
        RollbackToken::IoPrioRestore { .. } => (
            ManualOnly,
            "legacy task identity accepted by manual I/O-priority restoration",
        ),
        RollbackToken::UclampRestore { .. } => (
            VerifiedAutomatic,
            "complete process/task IDs and task start times from the prepared snapshot; each task re-verified immediately before its write; exited or replaced tasks skipped; legacy records without a start time are never restored automatically",
        ),
        RollbackToken::CgroupRestore { .. } => (
            VerifiedAutomatic,
            "complete process/task IDs and task start times plus the inode of every original cgroup, from the prepared snapshot; each task re-verified immediately before its write; exited or replaced tasks and recreated cgroups skipped; legacy records are never restored automatically",
        ),
        RollbackToken::IrqAffinityRestore { .. } => (
            ManualOnly,
            "automatic IRQ resource identity and crash recovery unqualified",
        ),
        RollbackToken::CpuPowerRestore { .. } => (
            VerifiedAutomatic,
            "the cpufreq policy, its related CPU set, driver, controlled attribute, legal values and boot are rediscovered and compared before any write; a record without that identity, from another boot, or whose original value is no longer legal is never replayed",
        ),
        RollbackToken::GpuPowerRestore { .. } => (
            ManualOnly,
            "automatic GPU-power resource identity and crash recovery unqualified",
        ),
        RollbackToken::VmKnobRestore { .. } => (
            ManualOnly,
            "automatic VM-knob resource identity and crash recovery unqualified",
        ),
        RollbackToken::SysfsRestore { .. } => (
            NotRecoverable,
            "arbitrary path token has no automatic resource identity contract",
        ),
        RollbackToken::PmQosRelease { .. } => (
            VerifiedAutomatic,
            "the lease ends with its holder descriptor; recovery only reads the aggregate target and never writes, and reports a target still equal to the request as unproven",
        ),
        RollbackToken::ResctrlGroupRemove { .. } => (
            VerifiedAutomatic,
            "the recorded group path must still name the same kernel object (device and inode); a replaced or absent group is never removed and an absent one is reported as already gone",
        ),
        RollbackToken::DamosTeardown { .. } => (
            ManualOnly,
            "configuration teardown is exact but page reclaim or migration already done is irreversible, so recovery is never automatic and never claims the memory state was restored",
        ),
        RollbackToken::CgroupControlRestore { .. } => (
            VerifiedAutomatic,
            "the recorded group must still be the same kernel object (device and inode) and the control file must still hold the value this transaction wrote; a replaced group or a foreign value is never overwritten, and a baseline that cannot be written back exactly is never recorded",
        ),
        RollbackToken::BlockSchedulerRestore { .. } => (
            VerifiedAutomatic,
            "the recorded device must still have the recorded major:minor number and still have the applied scheduler active; a different device or another owner's choice is never overwritten",
        ),
        RollbackToken::ScxSchedulerRestore { .. } => (
            ManualOnly,
            "restoring the prior sched_ext scheduler needs the org.scx.Loader bus and an offline campaign authorization; recovery is never automatic and a prior state that cannot be put back exactly is never recorded",
        ),
        RollbackToken::NumaMigration { .. } => (
            ManualOnly,
            "page migration is irreversible in effect: pages reclaimed, pinned or faulted in later do not return, so recovery is never automatic and never claims the memory layout was restored",
        ),
        RollbackToken::NetSysctlRestore { .. } => (
            VerifiedAutomatic,
            "the recorded sysctl path must belong to the same network namespace (read from /proc/self/ns/net) and still hold the applied value; another namespace or a foreign value is never overwritten",
        ),
    };
    RollbackRecoveryDescriptor {
        token_kind: token.kind(),
        automatic_recovery,
        identity_contract,
    }
}

pub(crate) fn require_automatic_recovery(token: &RollbackToken) -> anyhow::Result<()> {
    let descriptor = recovery_descriptor(token);
    anyhow::ensure!(
        descriptor.automatic_recovery == AutomaticRecoveryCapability::VerifiedAutomatic,
        "startup_recovery_not_authorized: token_kind={} capability={:?}; {}; explicit manual restoration required",
        descriptor.token_kind,
        descriptor.automatic_recovery,
        descriptor.identity_contract,
    );
    Ok(())
}
