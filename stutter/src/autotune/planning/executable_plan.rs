//! Executable autotune candidate plan payloads.

use serde::{Deserialize, Serialize};

use super::candidate::{CandidateAction, CandidateEvidence, CandidatePlan};
use crate::{
    actions::{
        SafetyClass, TuningAction, cgroup::CgroupPlacementAction, cpu_power::CpuPowerAction,
        gpu_power::GpuPowerAction, ioprio::IoPrioAction, irq_affinity::IrqAffinityAction,
        nice::NiceAction, uclamp::UclampAction, vm_knobs::VmKnobAction,
    },
    autotune::{conflicts::ActionConflictGroup, objective::ObjectiveKind},
    daemon_policy::ActionEffectScope,
    profiles::Profile,
};

#[derive(Clone, Debug)]
pub struct CpuAffinityProfilePlan {
    pub profile_name: String,
    pub profile: Profile,
    pub tree_pid: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CandidateExecutablePlan {
    Nice { plan: NiceActionPlan },
    IoPrio { plan: IoPrioActionPlan },
    Uclamp { plan: UclampActionPlan },
    CgroupPlacement { plan: CgroupPlacementActionPlan },
    IrqAffinity { plan: IrqAffinityActionPlan },
    CpuPower { plan: CpuPowerActionPlan },
    GpuPower { plan: GpuPowerActionPlan },
    VmKnob { plan: VmKnobActionPlan },
}

impl CandidateExecutablePlan {
    pub fn from_candidate(candidate: &CandidateAction) -> Option<Self> {
        match candidate {
            CandidateAction::Nice { plan } => Some(Self::Nice { plan: plan.clone() }),
            CandidateAction::IoPrio { plan } => Some(Self::IoPrio { plan: plan.clone() }),
            CandidateAction::Uclamp { plan } => Some(Self::Uclamp { plan: plan.clone() }),
            CandidateAction::CgroupPlacement { plan } => {
                Some(Self::CgroupPlacement { plan: plan.clone() })
            }
            CandidateAction::IrqAffinity { plan }
                if plan.safety_class() < SafetyClass::HighRisk =>
            {
                Some(Self::IrqAffinity { plan: plan.clone() })
            }
            CandidateAction::CpuPower { plan } if plan.safety_class() < SafetyClass::HighRisk => {
                Some(Self::CpuPower { plan: plan.clone() })
            }
            CandidateAction::GpuPower { plan } if plan.safety_class() < SafetyClass::HighRisk => {
                Some(Self::GpuPower { plan: plan.clone() })
            }
            CandidateAction::VmKnob { plan } if plan.safety_class() < SafetyClass::HighRisk => {
                Some(Self::VmKnob { plan: plan.clone() })
            }
            CandidateAction::CpuAffinityProfile { .. }
            | CandidateAction::IrqAffinity { .. }
            | CandidateAction::CpuPower { .. }
            | CandidateAction::GpuPower { .. }
            | CandidateAction::VmKnob { .. }
            | CandidateAction::Fake { .. } => None,
        }
    }

    pub fn into_candidate(self) -> CandidateAction {
        match self {
            Self::Nice { plan } => CandidateAction::Nice { plan },
            Self::IoPrio { plan } => CandidateAction::IoPrio { plan },
            Self::Uclamp { plan } => CandidateAction::Uclamp { plan },
            Self::CgroupPlacement { plan } => CandidateAction::CgroupPlacement { plan },
            Self::IrqAffinity { plan } => CandidateAction::IrqAffinity { plan },
            Self::CpuPower { plan } => CandidateAction::CpuPower { plan },
            Self::GpuPower { plan } => CandidateAction::GpuPower { plan },
            Self::VmKnob { plan } => CandidateAction::VmKnob { plan },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NiceActionPlan {
    pub name: String,
    pub action: NiceAction,
    pub target_root_pid: Option<u32>,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IoPrioActionPlan {
    pub name: String,
    pub action: IoPrioAction,
    pub target_root_pid: Option<u32>,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UclampActionPlan {
    pub name: String,
    pub action: UclampAction,
    pub target_root_pid: Option<u32>,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CgroupPlacementActionPlan {
    pub name: String,
    pub action: CgroupPlacementAction,
    pub target_root_pid: Option<u32>,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IrqAffinityActionPlan {
    pub name: String,
    pub action: IrqAffinityAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CpuPowerActionPlan {
    pub name: String,
    pub action: CpuPowerAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GpuPowerActionPlan {
    pub name: String,
    pub action: GpuPowerAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VmKnobActionPlan {
    pub name: String,
    pub action: VmKnobAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug)]
pub struct FakeCandidatePlan {
    pub action_id: crate::actions::ActionId,
    pub safety_class: SafetyClass,
}

impl CandidatePlan for FakeCandidatePlan {
    fn candidate_name(&self) -> &str {
        "fake-profile"
    }

    fn action_kind(&self) -> &'static str {
        "fake"
    }

    fn target_root_pid(&self) -> Option<u32> {
        None
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action_id.clone()
    }

    fn safety_class(&self) -> SafetyClass {
        self.safety_class.clone()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::ObserveOnly
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &[]
    }

    fn objective(&self) -> ObjectiveKind {
        ObjectiveKind::StutterScore
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::None
    }

    fn describe(&self) -> String {
        format!("fake action {}", self.action_id.as_str())
    }
}

impl CandidatePlan for CpuAffinityProfilePlan {
    fn candidate_name(&self) -> &str {
        &self.profile_name
    }

    fn action_kind(&self) -> &'static str {
        "cpu_affinity_profile"
    }

    fn target_root_pid(&self) -> Option<u32> {
        Some(self.tree_pid)
    }

    fn action_id(&self) -> crate::actions::ActionId {
        crate::actions::ActionId::new(format!("cpu-affinity-profile:{}", self.profile_name))
    }

    fn safety_class(&self) -> SafetyClass {
        if crate::profiles::profile_uses_priority_actions(&self.profile) {
            SafetyClass::ReversibleMediumRisk
        } else {
            SafetyClass::ReversibleLowRisk
        }
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::LocalProcessTree
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &[]
    }

    fn objective(&self) -> ObjectiveKind {
        ObjectiveKind::StutterScore
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::CpuPlacement
    }

    fn describe(&self) -> String {
        format!(
            "apply CPU affinity profile '{}' to process tree {}",
            self.profile_name, self.tree_pid
        )
    }
}

impl CandidatePlan for NiceActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "nice"
    }

    fn target_root_pid(&self) -> Option<u32> {
        self.target_root_pid
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::LocalProcessTree
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &self.evidence
    }

    fn objective(&self) -> ObjectiveKind {
        self.objective
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::CpuPriority
    }

    fn describe(&self) -> String {
        self.action.describe()
    }
}

impl CandidatePlan for IoPrioActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "ionice"
    }

    fn target_root_pid(&self) -> Option<u32> {
        self.target_root_pid
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::LocalProcessTree
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

impl CandidatePlan for UclampActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "uclamp"
    }

    fn target_root_pid(&self) -> Option<u32> {
        self.target_root_pid
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::LocalProcessTree
    }

    fn evidence(&self) -> &[CandidateEvidence] {
        &self.evidence
    }

    fn objective(&self) -> ObjectiveKind {
        self.objective
    }

    fn conflict_group(&self) -> ActionConflictGroup {
        ActionConflictGroup::CpuPriority
    }

    fn describe(&self) -> String {
        self.action.describe()
    }
}

impl CandidatePlan for CgroupPlacementActionPlan {
    fn candidate_name(&self) -> &str {
        &self.name
    }

    fn action_kind(&self) -> &'static str {
        "cgroup_placement"
    }

    fn target_root_pid(&self) -> Option<u32> {
        self.target_root_pid
    }

    fn action_id(&self) -> crate::actions::ActionId {
        self.action.id()
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn effect_scope(&self) -> ActionEffectScope {
        ActionEffectScope::Cgroup
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

macro_rules! impl_system_candidate_plan {
    ($ty:ty, $kind:literal, $scope:expr, $conflict:expr) => {
        impl CandidatePlan for $ty {
            fn candidate_name(&self) -> &str {
                &self.name
            }

            fn action_kind(&self) -> &'static str {
                $kind
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
                $scope
            }

            fn evidence(&self) -> &[CandidateEvidence] {
                &self.evidence
            }

            fn objective(&self) -> ObjectiveKind {
                self.objective
            }

            fn conflict_group(&self) -> ActionConflictGroup {
                $conflict
            }

            fn describe(&self) -> String {
                self.action.describe()
            }
        }
    };
}

impl_system_candidate_plan!(
    IrqAffinityActionPlan,
    "irq_affinity",
    ActionEffectScope::Irq,
    ActionConflictGroup::IrqPlacement
);
impl_system_candidate_plan!(
    CpuPowerActionPlan,
    "cpu_power",
    ActionEffectScope::CpuPower,
    ActionConflictGroup::CpuPower
);
impl_system_candidate_plan!(
    GpuPowerActionPlan,
    "gpu_power",
    ActionEffectScope::GpuPower,
    ActionConflictGroup::GpuPower
);
impl_system_candidate_plan!(
    VmKnobActionPlan,
    "vm_knob",
    ActionEffectScope::VmKnob,
    ActionConflictGroup::VmMemory
);
