use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionConflictGroup {
    CpuPlacement,
    CpuPriority,
    IoPriority,
    CgroupPlacement,
    IrqPlacement,
    CpuPower,
    GpuPower,
    VmMemory,
    ThermalRecovery,
    #[default]
    None,
}

impl ActionConflictGroup {
    pub fn conflicts_with(self, other: Self) -> bool {
        use ActionConflictGroup::*;

        if self == None || other == None {
            return false;
        }
        if self == other {
            return true;
        }

        matches!(
            (self, other),
            (CpuPlacement, CgroupPlacement)
                | (CgroupPlacement, CpuPlacement)
                | (CpuPriority, CgroupPlacement)
                | (CgroupPlacement, CpuPriority)
                | (IrqPlacement, CpuPlacement)
                | (CpuPlacement, IrqPlacement)
                | (CpuPower, ThermalRecovery)
                | (ThermalRecovery, CpuPower)
                | (GpuPower, ThermalRecovery)
                | (ThermalRecovery, GpuPower)
                | (VmMemory, ThermalRecovery)
                | (ThermalRecovery, VmMemory)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::ActionConflictGroup::*;

    #[test]
    fn placement_and_thermal_conflict_rules_are_symmetric() {
        assert!(CpuPlacement.conflicts_with(CgroupPlacement));
        assert!(CgroupPlacement.conflicts_with(CpuPlacement));
        assert!(ThermalRecovery.conflicts_with(CpuPower));
        assert!(GpuPower.conflicts_with(ThermalRecovery));
        assert!(!CpuPlacement.conflicts_with(IoPriority));
        assert!(!None.conflicts_with(CpuPower));
    }
}
