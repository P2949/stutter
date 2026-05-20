use std::collections::BTreeSet;

use crate::{
    actions::{
        ActionError, ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
        TuningAction,
        cgroup::CgroupPlacementAction,
        cpu_affinity::CpuAffinityProfileAction,
        cpu_power::{CpuPowerAction, CpuPowerPolicy},
        gpu_power::{GpuPowerAction, GpuPowerMode, GpuPowerPolicy},
        ioprio::IoPrioAction,
        irq_affinity::{IrqAffinityAction, IrqAffinityPolicy},
        nice::NiceAction,
        uclamp::UclampAction,
        vm_knobs::{VmKnobAction, VmKnobMode, VmKnobPolicy},
    },
    autotune::candidate::{CandidateAction, CandidateFamily, ExecutablePlan},
};

pub trait ActionFactory {
    fn family(&self) -> CandidateFamily;
    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError>;
}

#[derive(Default)]
pub struct ActionFactoryRegistry {
    factories: Vec<Box<dyn ActionFactory>>,
}

impl ActionFactoryRegistry {
    pub fn register<F>(&mut self, factory: F)
    where
        F: ActionFactory + 'static,
    {
        self.factories.push(Box::new(factory));
    }

    pub fn default_registry() -> Self {
        let mut registry = Self::default();
        registry.register(CpuAffinityActionFactory);
        registry.register(NiceActionFactory);
        registry.register(IoPrioActionFactory);
        registry.register(UclampActionFactory);
        registry.register(CgroupPlacementActionFactory);
        registry.register(IrqAffinityActionFactory);
        registry.register(CpuPowerActionFactory);
        registry.register(GpuPowerActionFactory);
        registry.register(VmKnobActionFactory);
        registry
    }

    #[cfg(test)]
    pub fn families(&self) -> Vec<CandidateFamily> {
        self.factories
            .iter()
            .map(|factory| factory.family())
            .collect()
    }

    pub fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let family = plan.action_kind();
        self.factories
            .iter()
            .find(|factory| factory.family() == family)
            .ok_or_else(|| {
                ActionError::preflight(format!(
                    "no action factory registered for candidate family {family}"
                ))
            })?
            .build(plan)
    }
}

pub fn default_action_factory_registry() -> ActionFactoryRegistry {
    ActionFactoryRegistry::default_registry()
}

struct CpuAffinityActionFactory;

impl ActionFactory for CpuAffinityActionFactory {
    fn family(&self) -> CandidateFamily {
        "cpu_affinity_profile"
    }

    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let CandidateAction::CpuAffinityProfile { plan } = plan else {
            return Err(ActionError::preflight("candidate family mismatch"));
        };
        Ok(Box::new(CpuAffinityProfileAction {
            tree_pid: plan.tree_pid,
            profile: plan.profile.clone(),
            force_restore_overwrite: false,
        }))
    }
}

struct NiceActionFactory;

impl ActionFactory for NiceActionFactory {
    fn family(&self) -> CandidateFamily {
        "nice"
    }

    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let CandidateAction::Nice { plan } = plan else {
            return Err(ActionError::preflight("candidate family mismatch"));
        };
        Ok(Box::new(NiceAction {
            targets: plan.action.targets.clone(),
            nice: plan.action.nice,
            policy: plan.action.policy.clone(),
        }))
    }
}

struct IoPrioActionFactory;

impl ActionFactory for IoPrioActionFactory {
    fn family(&self) -> CandidateFamily {
        "ionice"
    }

    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let CandidateAction::IoPrio { plan } = plan else {
            return Err(ActionError::preflight("candidate family mismatch"));
        };
        Ok(Box::new(IoPrioAction {
            targets: plan.action.targets.clone(),
            ioprio: plan.action.ioprio,
            policy: plan.action.policy.clone(),
        }))
    }
}

struct UclampActionFactory;

impl ActionFactory for UclampActionFactory {
    fn family(&self) -> CandidateFamily {
        "uclamp"
    }

    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let CandidateAction::Uclamp { plan } = plan else {
            return Err(ActionError::preflight("candidate family mismatch"));
        };
        Ok(Box::new(UclampAction {
            targets: plan.action.targets.clone(),
            values: plan.action.values,
        }))
    }
}

struct CgroupPlacementActionFactory;

impl ActionFactory for CgroupPlacementActionFactory {
    fn family(&self) -> CandidateFamily {
        "cgroup_placement"
    }

    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let CandidateAction::CgroupPlacement { plan } = plan else {
            return Err(ActionError::preflight("candidate family mismatch"));
        };
        Ok(Box::new(CgroupPlacementAction {
            cgroup_root: plan.action.cgroup_root.clone(),
            target_cgroup: plan.action.target_cgroup.clone(),
            targets: plan.action.targets.clone(),
            cpuset_cpus: plan.action.cpuset_cpus.clone(),
            cpuset_mems: plan.action.cpuset_mems.clone(),
        }))
    }
}

struct IrqAffinityActionFactory;

impl ActionFactory for IrqAffinityActionFactory {
    fn family(&self) -> CandidateFamily {
        "irq_affinity"
    }

    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let CandidateAction::IrqAffinity { plan } = plan else {
            return Err(ActionError::preflight("candidate family mismatch"));
        };
        let action = IrqAffinityAction {
            irq: plan.action.irq,
            device_hint: plan.action.device_hint.clone(),
            smp_affinity: plan.action.smp_affinity.clone(),
            risk: plan.action.risk,
            evidence: plan.action.evidence.clone(),
            irq_root: plan.action.irq_root.clone(),
        };
        let policy = IrqAffinityPolicy {
            allow_irq_affinity_changes: true,
            allow_high_risk_devices: false,
            require_strong_irq_evidence: true,
            require_stable_irq_identity: true,
            require_known_device_mapping: true,
        };
        Ok(Box::new(PolicyBackedIrqAffinityAction { action, policy }))
    }
}

struct CpuPowerActionFactory;

impl ActionFactory for CpuPowerActionFactory {
    fn family(&self) -> CandidateFamily {
        "cpu_power"
    }

    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let CandidateAction::CpuPower { plan } = plan else {
            return Err(ActionError::preflight("candidate family mismatch"));
        };
        let action = CpuPowerAction {
            sysfs_root: plan.action.sysfs_root.clone(),
            cpus: plan.action.cpus.clone(),
            scaling_governor: plan.action.scaling_governor.clone(),
            energy_performance_preference: plan.action.energy_performance_preference.clone(),
        };
        let policy = CpuPowerPolicy {
            allow_cpu_power_changes: true,
            allowed_cpus: action.cpus.iter().copied().collect::<BTreeSet<_>>(),
            allow_governor_changes: action.scaling_governor.is_some(),
            allow_epp_changes: action.energy_performance_preference.is_some(),
            ..CpuPowerPolicy::default()
        };
        Ok(Box::new(PolicyBackedCpuPowerAction { action, policy }))
    }
}

struct GpuPowerActionFactory;

impl ActionFactory for GpuPowerActionFactory {
    fn family(&self) -> CandidateFamily {
        "gpu_power"
    }

    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let CandidateAction::GpuPower { plan } = plan else {
            return Err(ActionError::preflight("candidate family mismatch"));
        };
        let action = GpuPowerAction {
            sysfs_root: plan.action.sysfs_root.clone(),
            drm_card: plan.action.drm_card.clone(),
            power_dpm_force_performance_level: plan
                .action
                .power_dpm_force_performance_level
                .clone(),
            pp_power_profile_mode: plan.action.pp_power_profile_mode.clone(),
        };
        let policy = GpuPowerPolicy {
            allow_gpu_power_changes: true,
            mode: GpuPowerMode::ManualApply,
            allowed_drm_cards: [action.drm_card.clone()].into_iter().collect(),
            allow_force_performance_level: action.power_dpm_force_performance_level.is_some(),
            allow_power_profile_mode: action.pp_power_profile_mode.is_some(),
            ..GpuPowerPolicy::default()
        };
        Ok(Box::new(PolicyBackedGpuPowerAction { action, policy }))
    }
}

struct VmKnobActionFactory;

impl ActionFactory for VmKnobActionFactory {
    fn family(&self) -> CandidateFamily {
        "vm_knob"
    }

    fn build(&self, plan: &ExecutablePlan) -> Result<Box<dyn TuningAction>, ActionError> {
        let CandidateAction::VmKnob { plan } = plan else {
            return Err(ActionError::preflight("candidate family mismatch"));
        };
        let action = VmKnobAction {
            root: plan.action.root.clone(),
            changes: plan.action.changes.clone(),
        };
        let policy = VmKnobPolicy {
            allow_vm_knob_changes: true,
            mode: VmKnobMode::ManualApply,
            allowed_paths: action
                .changes
                .iter()
                .map(|change| change.path.clone())
                .collect(),
            require_latency_cliff_evidence: true,
            latency_cliff_evidence: true,
        };
        Ok(Box::new(PolicyBackedVmKnobAction { action, policy }))
    }
}

struct PolicyBackedIrqAffinityAction {
    action: IrqAffinityAction,
    policy: IrqAffinityPolicy,
}

impl TuningAction for PolicyBackedIrqAffinityAction {
    fn id(&self) -> ActionId {
        self.action.id()
    }

    fn describe(&self) -> String {
        self.action.describe()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.action.preflight_with_policy(&self.policy)
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.action.dry_run_with_policy(&self.policy)
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.action
            .apply_with_policy(&self.policy)
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.action.verify_with_policy(&self.policy)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

struct PolicyBackedCpuPowerAction {
    action: CpuPowerAction,
    policy: CpuPowerPolicy,
}

impl TuningAction for PolicyBackedCpuPowerAction {
    fn id(&self) -> ActionId {
        self.action.id()
    }

    fn describe(&self) -> String {
        self.action.describe()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.action.preflight_with_policy(&self.policy)
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.action.dry_run_with_policy(&self.policy)
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.action
            .apply_with_policy(&self.policy)
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.action.verify_with_policy(&self.policy)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

struct PolicyBackedGpuPowerAction {
    action: GpuPowerAction,
    policy: GpuPowerPolicy,
}

impl TuningAction for PolicyBackedGpuPowerAction {
    fn id(&self) -> ActionId {
        self.action.id()
    }

    fn describe(&self) -> String {
        self.action.describe()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.action.preflight_with_policy(&self.policy)
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.action.dry_run_with_policy(&self.policy)
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.action
            .apply_with_policy(&self.policy)
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.action.verify_with_policy(&self.policy)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

struct PolicyBackedVmKnobAction {
    action: VmKnobAction,
    policy: VmKnobPolicy,
}

impl TuningAction for PolicyBackedVmKnobAction {
    fn id(&self) -> ActionId {
        self.action.id()
    }

    fn describe(&self) -> String {
        self.action.describe()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.action.preflight_with_policy(&self.policy)
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.action.dry_run_with_policy(&self.policy)
    }

    fn apply(&self) -> crate::actions::ApplyResult {
        self.action
            .apply_with_policy(&self.policy)
            .map_err(Into::into)
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.action.verify_with_policy(&self.policy)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_factory_registry_covers_concrete_candidate_families() {
        let families = default_action_factory_registry().families();

        for family in [
            "cpu_affinity_profile",
            "nice",
            "ionice",
            "uclamp",
            "cgroup_placement",
            "irq_affinity",
            "cpu_power",
            "gpu_power",
            "vm_knob",
        ] {
            assert!(
                families.contains(&family),
                "missing action factory for {family}"
            );
        }
    }
}
