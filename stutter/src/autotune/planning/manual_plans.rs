//! Manual-only candidate payloads (PM QoS, resctrl, DAMOS). They are classified operations with
//! their own safety class, but are never converted to an executable autotune plan.
use super::candidate::{CandidateEvidence, CandidatePlan};
use crate::{
    actions::{SafetyClass, TuningAction},
    autotune::{conflicts::ActionConflictGroup, objective::ObjectiveKind},
    daemon_policy::ActionEffectScope,
};

/// A manual-only resctrl cache-allocation group. Never converted to an executable plan.
#[derive(Clone, Debug)]
pub struct ResctrlActionPlan {
    pub name: String,
    pub action: crate::actions::resctrl_action::ResctrlCatAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

impl ResctrlActionPlan {
    pub fn manual(name: &str, mask: u64, objective: ObjectiveKind) -> Self {
        Self {
            name: format!("resctrl_cat_{name}"),
            action: crate::actions::resctrl_action::ResctrlCatAction::new(name, mask),
            evidence: Vec::new(),
            objective,
        }
    }
}

/// A manual-only DAMOS scheme. Never converted to an executable plan.
#[derive(Clone, Debug)]
pub struct DamosActionPlan {
    pub name: String,
    pub action: crate::actions::damos_action::DamosSchemeAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

impl DamosActionPlan {
    pub fn manual(
        request: crate::actions::damos_scheme::DamosRequest,
        objective: ObjectiveKind,
    ) -> Self {
        Self {
            name: format!("damos_scheme_task_{}", request.task),
            action: crate::actions::damos_action::DamosSchemeAction::new(request),
            evidence: Vec::new(),
            objective,
        }
    }
}

/// A manual-only PM QoS request. It is never converted to an executable plan: HighRisk.
#[derive(Clone, Debug)]
pub struct PmQosActionPlan {
    pub name: String,
    pub action: crate::actions::pm_qos_action::PmQosCpuLatencyAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

impl PmQosActionPlan {
    /// A candidate for the standard device; the action still needs an explicit allow policy.
    pub fn manual(latency_us: u32, objective: ObjectiveKind) -> Self {
        Self {
            name: format!("pm_qos_cpu_latency_{latency_us}us"),
            action: crate::actions::pm_qos_action::PmQosCpuLatencyAction::new(latency_us),
            evidence: Vec::new(),
            objective,
        }
    }
}

impl CandidatePlan for ResctrlActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "resctrl_cache_allocation"
    }

    fn target_root_pid(&self) -> Option<u32> {
        None
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::SystemWide
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &self.evidence
    }

    fn objective(&self) -> ObjectiveKind {
        self.objective
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::None
    }

    fn describe(&self) -> String {
        self.action.describe()
    }
}

impl CandidatePlan for DamosActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "damos_scheme"
    }

    fn target_root_pid(&self) -> Option<u32> {
        Some(self.action.request.task)
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::SystemWide
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &self.evidence
    }

    fn objective(&self) -> ObjectiveKind {
        self.objective
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::VmMemory
    }

    fn describe(&self) -> String {
        self.action.describe()
    }
}

impl CandidatePlan for PmQosActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "pm_qos_cpu_latency"
    }

    fn target_root_pid(&self) -> Option<u32> {
        None
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::SystemWide
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &self.evidence
    }

    fn objective(&self) -> ObjectiveKind {
        self.objective
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::CpuPower
    }

    fn describe(&self) -> String {
        self.action.describe()
    }
}

/// A manual-only typed cgroup resource-control write. Never converted to an executable plan.
#[derive(Clone, Debug)]
pub struct CgroupControlActionPlan {
    pub name: String,
    pub action: crate::actions::cgroup::control_action::CgroupControlAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

impl CgroupControlActionPlan {
    pub fn manual(
        dir: std::path::PathBuf,
        control: crate::actions::cgroup::control::CgroupControl,
        objective: ObjectiveKind,
    ) -> Self {
        Self {
            name: format!("cgroup_control_{}", control.file_and_controller().0),
            action: crate::actions::cgroup::control_action::CgroupControlAction { dir, control },
            evidence: Vec::new(),
            objective,
        }
    }
}

impl CandidatePlan for CgroupControlActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "cgroup_resource_control"
    }

    fn target_root_pid(&self) -> Option<u32> {
        None
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::SystemWide
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &self.evidence
    }

    fn objective(&self) -> ObjectiveKind {
        self.objective
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::CgroupPlacement
    }

    fn describe(&self) -> String {
        self.action.describe()
    }
}

/// A manual-only device-wide block scheduler selection. Never converted to an executable plan.
#[derive(Clone, Debug)]
pub struct BlockSchedulerActionPlan {
    pub name: String,
    pub action: crate::actions::block_scheduler_action::BlockSchedulerAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

impl BlockSchedulerActionPlan {
    pub fn manual(
        request: crate::actions::block_scheduler::BlockSchedulerRequest,
        objective: ObjectiveKind,
    ) -> Self {
        Self {
            name: format!("block_scheduler_{}_{}", request.device, request.scheduler),
            action: crate::actions::block_scheduler_action::BlockSchedulerAction::new(request),
            evidence: Vec::new(),
            objective,
        }
    }
}

impl CandidatePlan for BlockSchedulerActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "block_io_scheduler"
    }

    fn target_root_pid(&self) -> Option<u32> {
        None
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::SystemWide
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &self.evidence
    }

    fn objective(&self) -> ObjectiveKind {
        self.objective
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::IoPriority
    }

    fn describe(&self) -> String {
        self.action.describe()
    }
}

/// A manual-only sched_ext scheduler switch through `org.scx.Loader`. It carries the validated
/// switch plan (exact restore included) but is never converted to an executable plan: execution
/// needs the loader bus and an offline campaign authorization.
#[derive(Clone, Debug)]
pub struct ScxStrategyPlan {
    pub name: String,
    pub switch: crate::integrations::scx_loader::SwitchPlan,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

impl ScxStrategyPlan {
    pub fn manual(
        switch: crate::integrations::scx_loader::SwitchPlan,
        objective: ObjectiveKind,
    ) -> Self {
        Self {
            name: format!("scx_strategy_{}_{}", switch.to, switch.mode),
            switch,
            evidence: Vec::new(),
            objective,
        }
    }
}

impl CandidatePlan for ScxStrategyPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "sched_ext_scheduler_strategy"
    }

    fn target_root_pid(&self) -> Option<u32> {
        None
    }

    fn action_id(&self) -> crate::actions::ActionId {
        crate::actions::ActionId::new(format!(
            "scx:switch:{}:mode={}",
            self.switch.to, self.switch.mode
        ))
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::HighRisk
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::SystemWide
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &self.evidence
    }

    fn objective(&self) -> ObjectiveKind {
        self.objective
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::None
    }

    fn describe(&self) -> String {
        format!(
            "switch the sched_ext scheduler to {} (mode {}) via org.scx.Loader",
            self.switch.to, self.switch.mode
        )
    }
}

/// A manual-only NUMA page migration (irreversible in effect). Never an executable plan.
#[derive(Clone, Debug)]
pub struct NumaMigrationActionPlan {
    pub name: String,
    pub action: crate::actions::numa_migration::NumaMigrationAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

impl NumaMigrationActionPlan {
    pub fn manual(
        action: crate::actions::numa_migration::NumaMigrationAction,
        objective: ObjectiveKind,
    ) -> Self {
        Self {
            name: format!("numa_migrate_task_{}", action.task),
            action,
            evidence: Vec::new(),
            objective,
        }
    }
}

impl CandidatePlan for NumaMigrationActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "numa_migrate_pages"
    }

    fn target_root_pid(&self) -> Option<u32> {
        Some(self.action.task)
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::SystemWide
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &self.evidence
    }

    fn objective(&self) -> ObjectiveKind {
        self.objective
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::VmMemory
    }

    fn describe(&self) -> String {
        self.action.describe()
    }
}
