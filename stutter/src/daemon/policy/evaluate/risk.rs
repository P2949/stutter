use super::super::{ActionDescriptor, ActionEffectScope, DaemonMode, DaemonPolicy};
use crate::actions::SafetyClass;

pub(super) fn medium_risk_system_scope_allowed(
    policy: &DaemonPolicy,
    descriptor: &ActionDescriptor,
) -> bool {
    if policy.mode != DaemonMode::ApplyMediumRisk
        || descriptor.safety_class > SafetyClass::ReversibleMediumRisk
    {
        return false;
    }

    match descriptor.effect_scope {
        ActionEffectScope::Irq => true,
        ActionEffectScope::CpuPower | ActionEffectScope::GpuPower => {
            policy.allow_gpu_power_in_autotune
        }
        ActionEffectScope::VmKnob => policy.allow_vm_knobs_in_autotune,
        _ => false,
    }
}
