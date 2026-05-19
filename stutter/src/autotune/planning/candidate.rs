//! Core autotune candidate model.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::executable_plan::{
    CgroupPlacementActionPlan, CpuAffinityProfilePlan, CpuPowerActionPlan, FakeCandidatePlan,
    GpuPowerActionPlan, IoPrioActionPlan, IrqAffinityActionPlan, NiceActionPlan, UclampActionPlan,
    VmKnobActionPlan,
};
use crate::{
    actions::SafetyClass,
    autotune::{conflicts::ActionConflictGroup, objective::ObjectiveKind},
    daemon_policy::{ActionDescriptor, ActionEffectScope, RollbackRequirement},
    profiles::Profile,
};

pub type CandidateFamily = &'static str;
pub type ExecutablePlan = CandidateAction;

#[derive(Clone, Debug)]
pub enum CandidateAction {
    CpuAffinityProfile { plan: CpuAffinityProfilePlan },
    Nice { plan: NiceActionPlan },
    IoPrio { plan: IoPrioActionPlan },
    Uclamp { plan: UclampActionPlan },
    CgroupPlacement { plan: CgroupPlacementActionPlan },
    IrqAffinity { plan: IrqAffinityActionPlan },
    CpuPower { plan: CpuPowerActionPlan },
    GpuPower { plan: GpuPowerActionPlan },
    VmKnob { plan: VmKnobActionPlan },
    Fake { plan: FakeCandidatePlan },
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // Transitional: providers still emit CandidateAction while planner migration adopts this wrapper.
pub struct SuggestionCandidate {
    candidate: CandidateAction,
}

#[allow(dead_code)] // Transitional: providers still emit CandidateAction while planner migration adopts this wrapper.
impl SuggestionCandidate {
    pub fn new(candidate: CandidateAction) -> Self {
        Self { candidate }
    }

    pub fn candidate(&self) -> &CandidateAction {
        &self.candidate
    }

    pub fn into_candidate(self) -> CandidateAction {
        self.candidate
    }
}

#[derive(Clone, Debug)]
pub struct ApplyCandidate {
    candidate: CandidateAction,
    eligibility: ApplyEligibility,
}

impl ApplyCandidate {
    pub fn candidate(&self) -> &CandidateAction {
        &self.candidate
    }

    pub fn eligibility(&self) -> &ApplyEligibility {
        &self.eligibility
    }

    pub fn into_candidate(self) -> CandidateAction {
        self.candidate
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyEligibility {
    pub policy_passed: bool,
    pub safety_passed: bool,
    pub rollback_available: bool,
    pub mode_passed: bool,
    pub data_quality_passed: bool,
    pub target_scope_passed: bool,
    pub capability_passed: bool,
    pub denial_reason: Option<String>,
}

impl ApplyEligibility {
    pub fn approved() -> Self {
        Self {
            policy_passed: true,
            safety_passed: true,
            rollback_available: true,
            mode_passed: true,
            data_quality_passed: true,
            target_scope_passed: true,
            capability_passed: true,
            denial_reason: None,
        }
    }

    #[cfg(test)]
    pub fn denied(reason: impl Into<String>) -> Self {
        Self {
            denial_reason: Some(reason.into()),
            ..Self::approved()
        }
    }

    pub fn is_applyable(&self) -> bool {
        self.policy_passed
            && self.safety_passed
            && self.rollback_available
            && self.mode_passed
            && self.data_quality_passed
            && self.target_scope_passed
            && self.capability_passed
            && self.denial_reason.is_none()
    }

    pub fn denial_message(&self) -> String {
        self.denial_reason
            .clone()
            .unwrap_or_else(|| "apply eligibility gates did not all pass".to_owned())
    }
}

pub fn try_promote_to_apply_candidate(
    candidate: CandidateAction,
    eligibility: ApplyEligibility,
) -> Result<ApplyCandidate, ApplyEligibility> {
    if eligibility.is_applyable() {
        Ok(ApplyCandidate {
            candidate,
            eligibility,
        })
    } else {
        Err(eligibility)
    }
}

impl CandidateAction {
    pub fn cpu_affinity_profile(profile: Profile, tree_pid: u32) -> Self {
        Self::CpuAffinityProfile {
            plan: CpuAffinityProfilePlan {
                profile_name: profile.name.clone(),
                profile,
                tree_pid,
            },
        }
    }

    pub fn fake(action_id: crate::actions::ActionId, safety_class: SafetyClass) -> Self {
        Self::Fake {
            plan: FakeCandidatePlan {
                action_id,
                safety_class,
            },
        }
    }

    pub fn candidate_name(&self) -> &str {
        self.plan().candidate_name()
    }

    pub fn profile_name(&self) -> &str {
        self.candidate_name()
    }

    pub fn target_root_pid(&self) -> Option<u32> {
        self.plan().target_root_pid()
    }

    pub fn tree_pid(&self) -> u32 {
        self.target_root_pid().unwrap_or(0)
    }

    pub fn action_kind(&self) -> &'static str {
        self.plan().action_kind()
    }

    pub fn safety_class(&self) -> crate::actions::SafetyClass {
        self.plan().safety_class()
    }

    pub fn action_id(&self) -> crate::actions::ActionId {
        self.plan().action_id()
    }

    pub fn descriptor(&self) -> ActionDescriptor {
        ActionDescriptor {
            action_id: self.action_id(),
            action_kind: self.action_kind().to_owned(),
            safety_class: self.safety_class(),
            effect_scope: self.effect_scope(),
            rollback: RollbackRequirement::RequiredBeforeApply,
            persistent_effect: false,
            touches_system_wide_state: matches!(
                self.effect_scope(),
                ActionEffectScope::Irq
                    | ActionEffectScope::Sysfs
                    | ActionEffectScope::CpuPower
                    | ActionEffectScope::GpuPower
                    | ActionEffectScope::VmKnob
                    | ActionEffectScope::SystemWide
            ),
            requires_explicit_target: self.target_root_pid().is_some(),
            confidence: None,
        }
    }

    pub fn effect_scope(&self) -> ActionEffectScope {
        self.plan().effect_scope()
    }

    pub fn evidence(&self) -> &[CandidateEvidence] {
        self.plan().evidence()
    }

    pub fn cooldown_key(&self) -> String {
        format!("{}:{}", self.action_kind(), self.candidate_name())
    }

    pub fn conflict_group(&self) -> ActionConflictGroup {
        self.plan().conflict_group()
    }

    pub fn conflicts_with(&self, other: &CandidateAction) -> bool {
        self.conflict_group().conflicts_with(other.conflict_group())
    }

    pub fn cgroup_target_path(&self) -> Option<&Path> {
        match self {
            Self::CgroupPlacement { plan } => Some(plan.action.target_cgroup.as_path()),
            _ => None,
        }
    }

    pub fn objective(&self) -> ObjectiveKind {
        self.plan().objective()
    }

    pub fn describe(&self) -> String {
        self.plan().describe()
    }

    fn plan(&self) -> &dyn CandidatePlan {
        match self {
            Self::CpuAffinityProfile { plan } => plan,
            Self::Nice { plan } => plan,
            Self::IoPrio { plan } => plan,
            Self::Uclamp { plan } => plan,
            Self::CgroupPlacement { plan } => plan,
            Self::IrqAffinity { plan } => plan,
            Self::CpuPower { plan } => plan,
            Self::GpuPower { plan } => plan,
            Self::VmKnob { plan } => plan,
            Self::Fake { plan } => plan,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateEvidence {
    pub signal: String,
    pub value: String,
    pub weight: f32,
}

impl CandidateEvidence {
    pub fn new(signal: impl Into<String>, value: impl Into<String>, weight: f32) -> Self {
        Self {
            signal: signal.into(),
            value: value.into(),
            weight: weight.clamp(0.0, 1.0),
        }
    }
}

pub trait CandidatePlan {
    fn candidate_name(&self) -> &str;
    fn action_kind(&self) -> &'static str;
    fn target_root_pid(&self) -> Option<u32>;
    fn action_id(&self) -> crate::actions::ActionId;
    fn safety_class(&self) -> SafetyClass;
    fn effect_scope(&self) -> ActionEffectScope;
    fn evidence(&self) -> &[CandidateEvidence];
    fn objective(&self) -> ObjectiveKind;
    fn conflict_group(&self) -> ActionConflictGroup;
    fn describe(&self) -> String;
}
