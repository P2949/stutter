#![allow(unused_imports)] // Transitional observation split facade while old paths are preserved.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) use super::{
    ActiveAffinitySnapshot, ActiveCgroupSnapshot, ActiveConfigSnapshot, ActiveCpuPowerSnapshot,
    ActiveGpuPowerSnapshot, ActiveIoPrioSnapshot, ActiveIrqSnapshot, ActiveNiceSnapshot,
    ActiveTaskSnapshot, ActiveUclampSnapshot, ActiveVmSnapshot, AutotuneObservation,
    CpuPolicyRuntimeState, GpuPowerRuntimeState, ProtectedTask, WorkloadIdentity,
};
