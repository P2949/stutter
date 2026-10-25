//! Typed experiment contract: what decides keep/revert, what must additionally be observed, and
//! what may veto. The primary workload outcome alone decides keep/revert; mechanism evidence
//! proves the action did what it claimed and can only withhold a keep, never create one.
use serde::{Deserialize, Serialize};

use super::ObjectiveKind;
use crate::autotune::operation::MutationOperationKind;

/// Physical evidence that the action did what it claimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanismEvidence {
    /// The requested CPU placement/mask was read back from the kernel.
    PlacementObserved,
    /// The requested scheduling attribute (nice, uclamp, io priority) was read back.
    SchedulingAttributeObserved,
    /// The task was observed in the requested cgroup.
    CgroupMembershipObserved,
    /// The power/performance attribute was read back from sysfs.
    PowerAttributeObserved,
    /// The IRQ's effective affinity was read back.
    IrqPlacementObserved,
    /// The sysctl value was read back.
    SysctlValueObserved,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentContract {
    /// The only outcome that decides keep/revert.
    pub primary: ObjectiveKind,
    pub mechanism: MechanismEvidence,
    /// Guardrail signal names that veto an apparently beneficial action.
    pub guardrails: Vec<String>,
}

impl ExperimentContract {
    pub fn for_operation(operation: MutationOperationKind, primary: ObjectiveKind) -> Self {
        Self {
            primary,
            mechanism: mechanism_for(operation),
            guardrails: primary.descriptor().guardrails,
        }
    }

    /// Keep requires the primary outcome to have improved and the mechanism to have been
    /// observed. Mechanism evidence never substitutes for the primary outcome.
    pub fn keep_allowed(&self, primary_improved: bool, mechanism_observed: bool) -> bool {
        primary_improved && (mechanism_observed || self.mechanism == MechanismEvidence::None)
    }
}

fn mechanism_for(operation: MutationOperationKind) -> MechanismEvidence {
    use MutationOperationKind::*;
    match operation {
        CpuAffinityProfile => MechanismEvidence::PlacementObserved,
        Uclamp | Nice | IoPrio => MechanismEvidence::SchedulingAttributeObserved,
        CgroupPlacement => MechanismEvidence::CgroupMembershipObserved,
        CpuPowerEpp | CpuPowerGovernor | GpuPowerProfile | GpuForcePerformance => {
            MechanismEvidence::PowerAttributeObserved
        }
        VmSwappiness | VmDirtyBackgroundRatio | VmDirtyRatio => {
            MechanismEvidence::SysctlValueObserved
        }
        IrqKnownDevice | IrqStorageOrUnknown => MechanismEvidence::IrqPlacementObserved,
        Unclassified => MechanismEvidence::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_declares_its_mechanism_and_inherits_the_objectives_guardrails() {
        for operation in MutationOperationKind::ALL {
            let contract =
                ExperimentContract::for_operation(operation, ObjectiveKind::GameRunnableLatency);
            assert_eq!(contract.primary, ObjectiveKind::GameRunnableLatency);
            assert!(!contract.guardrails.is_empty());
            if operation != MutationOperationKind::Unclassified {
                assert_ne!(contract.mechanism, MechanismEvidence::None, "{operation:?}");
            }
        }
    }

    #[test]
    fn mechanism_alone_never_keeps_an_action() {
        let contract = ExperimentContract::for_operation(
            MutationOperationKind::CpuAffinityProfile,
            ObjectiveKind::GameRunnableLatency,
        );
        assert!(!contract.keep_allowed(false, true));
        assert!(!contract.keep_allowed(true, false));
        assert!(contract.keep_allowed(true, true));
    }
}
