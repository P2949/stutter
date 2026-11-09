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

    /// A system-wide CPU wake-latency request (`/dev/cpu_dma_latency`), held as a lease.
    PmQosCpuLatency,

    /// An L3 cache allocation group (resctrl CAT), owned by Stutter and removed on restore.
    ResctrlCacheAllocation,

    /// A DAMOS scheme on one kdamond. Reclaim and LRU actions are irreversible in effect.
    DamosScheme,

    /// Typed cgroup resource-control knobs, each its own operation. None inherits the maturity of
    /// cgroup placement.
    CgroupCpuWeight,
    CgroupCpuIdle,
    CgroupCpuMax,
    CgroupCpusetCpus,
    CgroupCpusetMems,
    CgroupMemoryMin,
    CgroupMemoryLow,
    CgroupMemoryHigh,
    CgroupIoWeight,

    /// A device-wide block I/O scheduler selection (BFQ, Kyber, mq-deadline, none) on an
    /// explicitly authorized, unprotected device.
    BlockIoScheduler,

    /// A global sched_ext scheduler switch through `org.scx.Loader`, restored to the exact prior
    /// scheduler and mode. Never autonomous.
    SchedExtSchedulerStrategy,

    /// Moving one process's pages between NUMA nodes. Irreversible in effect; never autonomous.
    NumaMigratePages,

    /// Test-only or unrecognisable payloads. Never authorised by any contract.
    Unclassified,
}

impl MutationOperationKind {
    pub const ALL: [Self; 30] = [
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
        Self::PmQosCpuLatency,
        Self::ResctrlCacheAllocation,
        Self::DamosScheme,
        Self::CgroupCpuWeight,
        Self::CgroupCpuIdle,
        Self::CgroupCpuMax,
        Self::CgroupCpusetCpus,
        Self::CgroupCpusetMems,
        Self::CgroupMemoryMin,
        Self::CgroupMemoryLow,
        Self::CgroupMemoryHigh,
        Self::CgroupIoWeight,
        Self::BlockIoScheduler,
        Self::SchedExtSchedulerStrategy,
        Self::NumaMigratePages,
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
            Self::PmQosCpuLatency => "pm_qos_cpu_latency",
            Self::ResctrlCacheAllocation => "resctrl_cache_allocation",
            Self::DamosScheme => "damos_scheme",
            Self::CgroupCpuWeight => "cgroup_cpu_weight",
            Self::CgroupCpuIdle => "cgroup_cpu_idle",
            Self::CgroupCpuMax => "cgroup_cpu_max",
            Self::CgroupCpusetCpus => "cgroup_cpuset_cpus",
            Self::CgroupCpusetMems => "cgroup_cpuset_mems",
            Self::CgroupMemoryMin => "cgroup_memory_min",
            Self::CgroupMemoryLow => "cgroup_memory_low",
            Self::CgroupMemoryHigh => "cgroup_memory_high",
            Self::CgroupIoWeight => "cgroup_io_weight",
            Self::BlockIoScheduler => "block_io_scheduler",
            Self::SchedExtSchedulerStrategy => "sched_ext_scheduler_strategy",
            Self::NumaMigratePages => "numa_migrate_pages",
            Self::Unclassified => "unclassified",
        }
    }

    /// The product capability that governs this operation (`capability_tiers`). Several
    /// operations still share a coarse capability; the autonomous support registry, not the
    /// capability, decides which of them may run autonomously.
    pub fn capability_id(self) -> Option<&'static str> {
        match self {
            Self::CpuAffinityProfile => Some("cpu_affinity_experiment"),
            Self::Uclamp => Some("uclamp_experiment"),
            Self::Nice => Some("nice_experiment"),
            Self::IoPrio => Some("ioprio_experiment"),
            Self::CgroupPlacement => Some("cgroup_placement_experiment"),
            Self::CpuPowerEpp => Some("cpu_power_epp"),
            Self::CpuPowerGovernor => Some("cpu_power_governor"),
            Self::GpuPowerProfile => Some("gpu_power_profile"),
            Self::GpuForcePerformance => Some("gpu_force_performance"),
            Self::VmSwappiness => Some("vm_swappiness"),
            Self::VmDirtyBackgroundRatio => Some("vm_dirty_background_ratio"),
            Self::VmDirtyRatio => Some("vm_dirty_ratio"),
            Self::IrqKnownDevice => Some("irq_affinity_known_device"),
            Self::IrqStorageOrUnknown => Some("irq_affinity_storage_or_unknown"),
            Self::PmQosCpuLatency => Some("pm_qos_cpu_latency"),
            Self::ResctrlCacheAllocation => Some("resctrl_cache_allocation"),
            Self::DamosScheme => Some("damos_scheme"),
            Self::CgroupCpuWeight => Some("cgroup_cpu_weight"),
            Self::CgroupCpuIdle => Some("cgroup_cpu_idle"),
            Self::CgroupCpuMax => Some("cgroup_cpu_max"),
            Self::CgroupCpusetCpus => Some("cgroup_cpuset_cpus"),
            Self::CgroupCpusetMems => Some("cgroup_cpuset_mems"),
            Self::CgroupMemoryMin => Some("cgroup_memory_min"),
            Self::CgroupMemoryLow => Some("cgroup_memory_low"),
            Self::CgroupMemoryHigh => Some("cgroup_memory_high"),
            Self::CgroupIoWeight => Some("cgroup_io_weight"),
            Self::BlockIoScheduler => Some("block_io_scheduler"),
            Self::SchedExtSchedulerStrategy => Some("sched_ext_scheduler_strategy"),
            Self::NumaMigratePages => Some("numa_migrate_pages"),
            Self::Unclassified => None,
        }
    }

    /// Whether the operation has a real prepared-before-mutation transaction (a verified restore
    /// snapshot durable before the first write). A Supported autonomous contract requires it;
    /// each operation flips to `true` when its transaction lands and passes the fault matrix.
    pub const fn prepared_transaction_implemented(self) -> bool {
        matches!(
            self,
            Self::CpuAffinityProfile
                | Self::Uclamp
                | Self::Nice
                | Self::CgroupPlacement
                | Self::CpuPowerEpp
        )
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
                | Self::PmQosCpuLatency
                | Self::ResctrlCacheAllocation
                | Self::DamosScheme
                | Self::CgroupCpuWeight
                | Self::CgroupCpuIdle
                | Self::CgroupCpuMax
                | Self::CgroupCpusetCpus
                | Self::CgroupCpusetMems
                | Self::CgroupMemoryMin
                | Self::CgroupMemoryLow
                | Self::CgroupMemoryHigh
                | Self::CgroupIoWeight
                | Self::BlockIoScheduler
                | Self::SchedExtSchedulerStrategy
                | Self::NumaMigratePages
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
            Self::PmQos { .. } => MutationOperationKind::PmQosCpuLatency,
            Self::Resctrl { .. } => MutationOperationKind::ResctrlCacheAllocation,
            Self::Damos { .. } => MutationOperationKind::DamosScheme,
            Self::CgroupControl { plan } => cgroup_control_operation(&plan.action.control),
            Self::BlockScheduler { .. } => MutationOperationKind::BlockIoScheduler,
            Self::ScxStrategy { .. } => MutationOperationKind::SchedExtSchedulerStrategy,
            Self::NumaMigration { .. } => MutationOperationKind::NumaMigratePages,
            Self::Fake { .. } => MutationOperationKind::Unclassified,
        }
    }
}

fn cgroup_control_operation(
    control: &crate::actions::cgroup::control::CgroupControl,
) -> MutationOperationKind {
    use crate::actions::cgroup::control::CgroupControl as C;
    match control {
        C::CpuWeight(_) => MutationOperationKind::CgroupCpuWeight,
        C::CpuIdle(_) => MutationOperationKind::CgroupCpuIdle,
        C::CpuMax { .. } => MutationOperationKind::CgroupCpuMax,
        C::CpusetCpus(_) => MutationOperationKind::CgroupCpusetCpus,
        C::CpusetMems(_) => MutationOperationKind::CgroupCpusetMems,
        C::MemoryMin(_) => MutationOperationKind::CgroupMemoryMin,
        C::MemoryLow(_) => MutationOperationKind::CgroupMemoryLow,
        C::MemoryHigh(_) => MutationOperationKind::CgroupMemoryHigh,
        C::IoWeight(_) => MutationOperationKind::CgroupIoWeight,
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
