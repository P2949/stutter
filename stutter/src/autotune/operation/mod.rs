//! Operation identity. A provider family is a configuration grouping; the safety, evidence and
//! recovery requirements attach to the operation a candidate actually performs, which is derived
//! from its payload, never from the provider's name.
use serde::{Deserialize, Serialize};

use crate::{
    actions::{
        cpu_power::CpuPowerAction, gpu_power::GpuPowerAction, irq_affinity::IrqAffinityAction,
        vm_knobs::VmKnobAction,
    },
    autotune::planning::candidate::CandidateAction,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationOperationKind {
    CpuAffinityProfile,
    Uclamp,
    Nice,
    IoPrio,
    CgroupPlacement,

    CpuPowerEpp,
    CpuPowerGovernor,

    GpuPowerProfile,
    GpuForcePerformance,

    VmSwappiness,
    VmDirtyBackgroundRatio,
    VmDirtyRatio,

    IrqKnownDevice,
    IrqStorageOrUnknown,

    /// Test-only or unrecognisable payloads. Never authorised by any contract.
    Unclassified,
}

impl MutationOperationKind {
    pub const ALL: [Self; 15] = [
        Self::CpuAffinityProfile,
        Self::Uclamp,
        Self::Nice,
        Self::IoPrio,
        Self::CgroupPlacement,
        Self::CpuPowerEpp,
        Self::CpuPowerGovernor,
        Self::GpuPowerProfile,
        Self::GpuForcePerformance,
        Self::VmSwappiness,
        Self::VmDirtyBackgroundRatio,
        Self::VmDirtyRatio,
        Self::IrqKnownDevice,
        Self::IrqStorageOrUnknown,
        Self::Unclassified,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::CpuAffinityProfile => "cpu_affinity_profile",
            Self::Uclamp => "uclamp",
            Self::Nice => "nice",
            Self::IoPrio => "io_prio",
            Self::CgroupPlacement => "cgroup_placement",
            Self::CpuPowerEpp => "cpu_power_epp",
            Self::CpuPowerGovernor => "cpu_power_governor",
            Self::GpuPowerProfile => "gpu_power_profile",
            Self::GpuForcePerformance => "gpu_force_performance",
            Self::VmSwappiness => "vm_swappiness",
            Self::VmDirtyBackgroundRatio => "vm_dirty_background_ratio",
            Self::VmDirtyRatio => "vm_dirty_ratio",
            Self::IrqKnownDevice => "irq_known_device",
            Self::IrqStorageOrUnknown => "irq_storage_or_unknown",
            Self::Unclassified => "unclassified",
        }
    }

    /// The product capability that governs this operation (`capability_tiers`). Several
    /// operations still share a coarse capability; the autonomous support registry, not the
    /// capability, decides which of them may run autonomously.
    pub fn capability_id(self) -> Option<&'static str> {
        match self {
            Self::CpuAffinityProfile => Some("cpu_affinity_experiment"),
            Self::Uclamp | Self::Nice | Self::IoPrio | Self::CgroupPlacement => {
                Some("process_priority_and_placement")
            }
            Self::CpuPowerEpp
            | Self::CpuPowerGovernor
            | Self::GpuPowerProfile
            | Self::GpuForcePerformance
            | Self::VmSwappiness
            | Self::VmDirtyBackgroundRatio
            | Self::VmDirtyRatio
            | Self::IrqKnownDevice
            | Self::IrqStorageOrUnknown => Some("system_wide_tuning"),
            Self::Unclassified => None,
        }
    }

    /// Whether the operation has a real prepared-before-mutation transaction (a verified restore
    /// snapshot durable before the first write). A Supported autonomous contract requires it;
    /// each operation flips to `true` when its transaction lands and passes the fault matrix.
    pub const fn prepared_transaction_implemented(self) -> bool {
        matches!(self, Self::CpuAffinityProfile)
    }

    /// Operations that stay manual (HighRisk) for the whole roadmap.
    pub fn is_manual_only(self) -> bool {
        matches!(
            self,
            Self::CpuPowerGovernor
                | Self::GpuForcePerformance
                | Self::VmDirtyBackgroundRatio
                | Self::VmDirtyRatio
                | Self::IrqStorageOrUnknown
                | Self::Unclassified
        )
    }
}

impl CandidateAction {
    /// The operation this candidate performs, derived from its payload. Payloads that touch a
    /// riskier attribute than the family's default classify as the riskier operation.
    pub fn operation_kind(&self) -> MutationOperationKind {
        match self {
            // A profile that also changes nice/ionice is a different, riskier operation with the
            // legacy transaction; it is not the graduated affinity operation.
            Self::CpuAffinityProfile { plan } => {
                if crate::profiles::profile_uses_priority_actions(&plan.profile) {
                    MutationOperationKind::Unclassified
                } else {
                    MutationOperationKind::CpuAffinityProfile
                }
            }
            Self::Nice { .. } => MutationOperationKind::Nice,
            Self::IoPrio { .. } => MutationOperationKind::IoPrio,
            Self::Uclamp { .. } => MutationOperationKind::Uclamp,
            Self::CgroupPlacement { .. } => MutationOperationKind::CgroupPlacement,
            Self::IrqAffinity { plan } => irq_operation(&plan.action),
            Self::CpuPower { plan } => cpu_power_operation(&plan.action),
            Self::GpuPower { plan } => gpu_power_operation(&plan.action),
            Self::VmKnob { plan } => vm_operation(&plan.action),
            Self::Fake { .. } => MutationOperationKind::Unclassified,
        }
    }
}

fn irq_operation(action: &IrqAffinityAction) -> MutationOperationKind {
    if action.evidence.known_device_mapping
        && action.risk == crate::actions::irq_affinity::IrqAffinityRisk::ReversibleMediumRisk
    {
        MutationOperationKind::IrqKnownDevice
    } else {
        MutationOperationKind::IrqStorageOrUnknown
    }
}

fn cpu_power_operation(action: &CpuPowerAction) -> MutationOperationKind {
    if action.scaling_governor.is_some() {
        MutationOperationKind::CpuPowerGovernor
    } else if action.energy_performance_preference.is_some() {
        MutationOperationKind::CpuPowerEpp
    } else {
        MutationOperationKind::Unclassified
    }
}

fn gpu_power_operation(action: &GpuPowerAction) -> MutationOperationKind {
    if action.power_dpm_force_performance_level.is_some() {
        MutationOperationKind::GpuForcePerformance
    } else if action.pp_power_profile_mode.is_some() {
        MutationOperationKind::GpuPowerProfile
    } else {
        MutationOperationKind::Unclassified
    }
}

fn vm_operation(action: &VmKnobAction) -> MutationOperationKind {
    let mut operation = None;
    for change in &action.changes {
        let this = match change.path.file_name().and_then(|name| name.to_str()) {
            Some("swappiness") => MutationOperationKind::VmSwappiness,
            Some("dirty_background_ratio") => MutationOperationKind::VmDirtyBackgroundRatio,
            Some("dirty_ratio") => MutationOperationKind::VmDirtyRatio,
            _ => return MutationOperationKind::Unclassified,
        };
        // The riskiest knob in the set decides; the lowest-risk knob is swappiness.
        operation = Some(match (operation, this) {
            (Some(MutationOperationKind::VmDirtyRatio), _)
            | (_, MutationOperationKind::VmDirtyRatio) => MutationOperationKind::VmDirtyRatio,
            (Some(MutationOperationKind::VmDirtyBackgroundRatio), _)
            | (_, MutationOperationKind::VmDirtyBackgroundRatio) => {
                MutationOperationKind::VmDirtyBackgroundRatio
            }
            _ => MutationOperationKind::VmSwappiness,
        });
    }
    operation.unwrap_or(MutationOperationKind::Unclassified)
}

#[cfg(test)]
mod tests;
