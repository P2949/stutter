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
            ManualOnly,
            "legacy task identity accepted by manual nice restoration",
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
            ManualOnly,
            "automatic task/cgroup resource identity and crash recovery unqualified",
        ),
        RollbackToken::IrqAffinityRestore { .. } => (
            ManualOnly,
            "automatic IRQ resource identity and crash recovery unqualified",
        ),
        RollbackToken::CpuPowerRestore { .. } => (
            ManualOnly,
            "automatic CPU-power resource identity and crash recovery unqualified",
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
