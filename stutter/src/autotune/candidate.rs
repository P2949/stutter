use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    actions::{
        ActionState, ActionWarning, SafetyClass, TuningAction, cgroup::CgroupPlacementAction,
        cpu_affinity::CpuAffinityProfileAction, cpu_power::CpuPowerAction,
        gpu_power::GpuPowerAction, ioprio::IoPrioAction, irq_affinity::IrqAffinityAction,
        nice::NiceAction, uclamp::UclampAction, vm_knobs::VmKnobAction,
    },
    autotune::{conflicts::ActionConflictGroup, objective::ObjectiveKind},
    daemon_policy::{
        ActionDescriptor, ActionEffectScope, ActionSource, DaemonMode, DaemonPolicy, PolicyIntent,
        RollbackRequirement,
    },
    process_tree::{CompiledPattern, TaskClass},
    profiles::{Profile, ProfileRule},
    topology::{CoreInfo, TopologyModel, cpu_mask_to_vec, cpus_to_mask, sorted_unique},
};

#[derive(Clone, Debug)]
pub enum CandidateAction {
    CpuAffinityProfile {
        plan: CpuAffinityProfilePlan,
    },
    Nice {
        plan: NiceActionPlan,
    },
    IoPrio {
        plan: IoPrioActionPlan,
    },
    Uclamp {
        plan: UclampActionPlan,
    },
    CgroupPlacement {
        plan: CgroupPlacementActionPlan,
    },
    IrqAffinity {
        plan: IrqAffinityActionPlan,
    },
    CpuPower {
        plan: CpuPowerActionPlan,
    },
    GpuPower {
        plan: GpuPowerActionPlan,
    },
    VmKnob {
        plan: VmKnobActionPlan,
    },
    Fake {
        action_id: crate::actions::ActionId,
        safety_class: crate::actions::SafetyClass,
    },
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

    pub fn candidate_name(&self) -> &str {
        self.plan_metadata()
            .map(CandidatePlan::candidate_name)
            .unwrap_or("fake-profile")
    }

    pub fn profile_name(&self) -> &str {
        self.candidate_name()
    }

    pub fn target_root_pid(&self) -> Option<u32> {
        self.plan_metadata()
            .and_then(CandidatePlan::target_root_pid)
    }

    pub fn tree_pid(&self) -> u32 {
        self.target_root_pid().unwrap_or(0)
    }

    pub fn action_kind(&self) -> &'static str {
        self.plan_metadata()
            .map(CandidatePlan::action_kind)
            .unwrap_or("fake")
    }

    pub fn safety_class(&self) -> crate::actions::SafetyClass {
        self.plan_metadata()
            .map(CandidatePlan::safety_class)
            .unwrap_or_else(|| match self {
                Self::Fake { safety_class, .. } => safety_class.clone(),
                _ => SafetyClass::ObserveOnly,
            })
    }

    pub fn action_id(&self) -> crate::actions::ActionId {
        self.plan_metadata()
            .map(CandidatePlan::action_id)
            .unwrap_or_else(|| match self {
                Self::Fake { action_id, .. } => action_id.clone(),
                _ => crate::actions::ActionId("unknown".to_owned()),
            })
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
        self.plan_metadata()
            .map(CandidatePlan::effect_scope)
            .unwrap_or(ActionEffectScope::ObserveOnly)
    }

    pub fn evidence(&self) -> &[CandidateEvidence] {
        self.plan_metadata()
            .map(CandidatePlan::evidence)
            .unwrap_or(&[])
    }

    pub fn cooldown_key(&self) -> String {
        format!("{}:{}", self.action_kind(), self.candidate_name())
    }

    pub fn conflict_group(&self) -> ActionConflictGroup {
        self.plan_metadata()
            .map(CandidatePlan::conflict_group)
            .unwrap_or(ActionConflictGroup::None)
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
        self.plan_metadata()
            .map(CandidatePlan::objective)
            .unwrap_or(ObjectiveKind::StutterScore)
    }

    pub fn describe(&self) -> String {
        self.plan_metadata()
            .map(CandidatePlan::describe)
            .unwrap_or_else(|| match self {
                Self::Fake { action_id, .. } => format!("fake action {}", action_id.0),
                _ => "unknown candidate".to_owned(),
            })
    }

    fn plan_metadata(&self) -> Option<&dyn CandidatePlan> {
        match self {
            Self::CpuAffinityProfile { plan } => Some(plan),
            Self::Nice { plan } => Some(plan),
            Self::IoPrio { plan } => Some(plan),
            Self::Uclamp { plan } => Some(plan),
            Self::CgroupPlacement { plan } => Some(plan),
            Self::IrqAffinity { plan } => Some(plan),
            Self::CpuPower { plan } => Some(plan),
            Self::GpuPower { plan } => Some(plan),
            Self::VmKnob { plan } => Some(plan),
            Self::Fake { .. } => None,
        }
    }

    pub fn is_high_risk_system_adjacent(&self) -> bool {
        matches!(
            self,
            Self::IrqAffinity { .. }
                | Self::CpuPower { .. }
                | Self::GpuPower { .. }
                | Self::VmKnob { .. }
        ) || self.descriptor().touches_system_wide_state
            || self.safety_class() == SafetyClass::HighRisk
    }

    pub fn manual_only_reason(&self) -> Option<String> {
        self.is_high_risk_system_adjacent().then(|| {
            format!(
                "manual-only high-risk/system-adjacent candidate; autonomous apply is disabled for action_kind={}",
                self.action_kind()
            )
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateEvidence {
    pub signal: String,
    pub value: String,
    pub weight: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidatePlanFile {
    pub schema_version: u32,
    pub candidate: CandidatePlanSummary,
    pub descriptor: ActionDescriptor,
    pub objective: ObjectiveKind,
    pub evidence: Vec<CandidateEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_apply_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_only_reason: Option<String>,
    pub executable: Option<CandidateExecutablePlan>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidatePlanSummary {
    pub candidate_name: String,
    pub action_kind: String,
    pub affected_tasks: Option<usize>,
    pub reason: Option<String>,
}

impl CandidatePlanFile {
    pub const SCHEMA_VERSION: u32 = 1;

    pub fn from_suggestion(suggestion: &CandidateSuggestion) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            candidate: CandidatePlanSummary {
                candidate_name: suggestion.candidate_name.clone(),
                action_kind: suggestion.action_kind.clone(),
                affected_tasks: Some(suggestion.affected_tasks),
                reason: Some(suggestion.reason.clone()),
            },
            descriptor: suggestion.descriptor.clone(),
            objective: suggestion.objective,
            evidence: suggestion.evidence.clone(),
            manual_apply_command: suggestion.manual_apply_command.clone(),
            manual_only_reason: suggestion.manual_only_reason.clone(),
            executable: None,
        }
    }

    pub fn from_candidate(candidate: &CandidateAction, affected_tasks: Option<usize>) -> Self {
        let (manual_apply_command, manual_only_reason) = candidate_plan_manual_metadata(candidate);
        Self {
            schema_version: Self::SCHEMA_VERSION,
            candidate: CandidatePlanSummary {
                candidate_name: candidate.candidate_name().to_owned(),
                action_kind: candidate.action_kind().to_owned(),
                affected_tasks,
                reason: Some(candidate.describe()),
            },
            descriptor: candidate.descriptor(),
            objective: candidate.objective(),
            evidence: candidate.evidence().to_vec(),
            manual_apply_command,
            manual_only_reason,
            executable: CandidateExecutablePlan::from_candidate(candidate),
        }
    }
}

fn candidate_plan_manual_metadata(candidate: &CandidateAction) -> (Option<String>, Option<String>) {
    match candidate {
        CandidateAction::CpuAffinityProfile { plan } => (
            Some(format!(
                "stutter apply-profile --tree-pid {} --profile <generated-or-existing-profile>",
                plan.tree_pid
            )),
            Some("cpu-affinity profiles use apply-profile, not candidate-plan apply".to_owned()),
        ),
        _ => (None, candidate.manual_only_reason()),
    }
}

pub fn default_candidate_plan_dir() -> PathBuf {
    let mut path = crate::autotune::history::default_autotune_history_path();
    path.pop();
    path.push("candidate_plans");
    path
}

pub fn candidate_plan_path(candidate: &CandidateAction, plan_dir: &Path) -> PathBuf {
    plan_dir.join(format!(
        "{}-{}.json",
        sanitize_candidate_plan_component(candidate.action_kind()),
        sanitize_candidate_plan_component(candidate.candidate_name())
    ))
}

pub fn write_candidate_plan_file(
    path: &Path,
    candidate: &CandidateAction,
    affected_tasks: Option<usize>,
) -> anyhow::Result<CandidatePlanFile> {
    let plan = CandidatePlanFile::from_candidate(candidate, affected_tasks);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let bytes = serde_json::to_vec_pretty(&plan)?;
    fs::write(path, bytes)?;

    Ok(plan)
}

fn sanitize_candidate_plan_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            sanitized.push(ch);
        } else {
            sanitized.push('-');
        }
    }

    let sanitized = sanitized.trim_matches('-');
    if sanitized.is_empty() {
        "candidate".to_owned()
    } else {
        sanitized.to_owned()
    }
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

#[derive(Clone, Debug)]
pub struct IrqAffinityActionPlan {
    pub name: String,
    pub action: IrqAffinityAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug)]
pub struct CpuPowerActionPlan {
    pub name: String,
    pub action: CpuPowerAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug)]
pub struct GpuPowerActionPlan {
    pub name: String,
    pub action: GpuPowerAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
}

#[derive(Clone, Debug)]
pub struct VmKnobActionPlan {
    pub name: String,
    pub action: VmKnobAction,
    pub evidence: Vec<CandidateEvidence>,
    pub objective: ObjectiveKind,
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
        crate::actions::ActionId(format!("cpu-affinity-profile:{}", self.profile_name))
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

#[derive(Clone, Debug)]
pub struct GeneratedProfileCandidatePlan {
    pub optimization_candidates: Vec<CandidateAction>,
    pub recovery_fallback: Option<CandidateAction>,
    pub rejected: Vec<RejectedCandidateProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedCpuSetPolicy {
    pub allowed_cpus: Option<crate::affinity::CpuMask>,
    pub denied_cpus: Option<crate::affinity::CpuMask>,
    pub min_render_cpus: usize,
    pub min_game_cpus: usize,
    pub min_compositor_cpus: usize,
    pub min_background_cpus: usize,
}

impl Default for GeneratedCpuSetPolicy {
    fn default() -> Self {
        Self {
            allowed_cpus: None,
            denied_cpus: None,
            min_render_cpus: 1,
            min_game_cpus: 1,
            min_compositor_cpus: 1,
            min_background_cpus: 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GeneratedTopologyProfilePlan {
    pub profiles: Vec<Profile>,
    pub rejected: Vec<RejectedCandidateProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedCandidateProfile {
    pub profile_name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateProfileStatus {
    pub matched_tasks: usize,
    pub dry_run_tasks: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CandidateDryRunRecord {
    pub candidate_name: String,
    pub affected_tasks: usize,
    pub warnings: Vec<ActionWarning>,
    pub safety_class: SafetyClass,
    pub eligible: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CandidateSuggestion {
    pub candidate_name: String,
    pub action_kind: String,
    pub descriptor: ActionDescriptor,
    pub objective: ObjectiveKind,
    pub evidence: Vec<CandidateEvidence>,
    pub affected_tasks: usize,
    pub safety: SafetyClass,
    pub reason: String,
    pub dry_run_command: Option<String>,
    pub manual_apply_command: Option<String>,
    pub required_mode: DaemonMode,
    pub required_safety_class: SafetyClass,
    pub manual_only_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateManualCommands {
    pub dry_run_command: Option<String>,
    pub manual_apply_command: Option<String>,
    pub required_mode: DaemonMode,
    pub required_safety_class: SafetyClass,
    pub manual_only_reason: Option<String>,
}

impl CandidateAction {
    pub fn manual_commands(
        &self,
        plan_path: &Path,
        policy: &DaemonPolicy,
    ) -> CandidateManualCommands {
        let required_mode = required_mode_for_safety_class(&self.safety_class());
        let descriptor = self.descriptor();
        let manual_only_reason = self.manual_only_reason();
        let dry_run_command = Some(format!(
            "stutter autotune apply-candidate --candidate-json {} --dry-run",
            plan_path.display()
        ));
        let manual_apply_command = if manual_only_reason.is_none()
            && policy
                .check_action(PolicyIntent::Apply, &descriptor)
                .is_ok()
        {
            Some(format!(
                "stutter autotune apply-candidate --candidate-json {}",
                plan_path.display()
            ))
        } else {
            None
        };

        CandidateManualCommands {
            dry_run_command,
            manual_apply_command,
            required_mode,
            required_safety_class: self.safety_class(),
            manual_only_reason,
        }
    }
}

pub fn suggestion_from_candidate_dry_run_record(
    candidate: &CandidateAction,
    record: &CandidateDryRunRecord,
    plan_path: &Path,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: impl Into<String>,
) -> Option<CandidateSuggestion> {
    if !record.eligible {
        return None;
    }

    if record.safety_class > max_safety_class {
        return None;
    }

    if let CandidateAction::CpuAffinityProfile { .. } = candidate {
        return cpu_affinity_suggestion_from_dry_run_record(
            record,
            candidate.tree_pid(),
            profile_path,
            max_safety_class,
            reason,
        );
    }

    let required_mode = required_mode_for_safety_class(&record.safety_class);
    let policy = policy_for_required_mode(required_mode)
        .unwrap_or_else(|| DaemonPolicy::suggest(ActionSource::Cli));
    let commands = candidate.manual_commands(plan_path, &policy);

    Some(CandidateSuggestion {
        candidate_name: candidate.candidate_name().to_owned(),
        action_kind: candidate.action_kind().to_owned(),
        descriptor: candidate.descriptor(),
        objective: candidate.objective(),
        evidence: candidate.evidence().to_vec(),
        affected_tasks: record.affected_tasks,
        safety: record.safety_class.clone(),
        reason: reason.into(),
        dry_run_command: commands.dry_run_command,
        manual_apply_command: commands.manual_apply_command,
        required_mode: commands.required_mode,
        required_safety_class: commands.required_safety_class,
        manual_only_reason: commands.manual_only_reason,
    })
}

pub fn suggestion_from_dry_run_record(
    record: &CandidateDryRunRecord,
    tree_pid: u32,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: impl Into<String>,
) -> Option<CandidateSuggestion> {
    cpu_affinity_suggestion_from_dry_run_record(
        record,
        tree_pid,
        profile_path,
        max_safety_class,
        reason,
    )
}

fn cpu_affinity_suggestion_from_dry_run_record(
    record: &CandidateDryRunRecord,
    tree_pid: u32,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: impl Into<String>,
) -> Option<CandidateSuggestion> {
    if !record.eligible {
        return None;
    }

    if record.safety_class > max_safety_class {
        return None;
    }

    let profile_arg = profile_path
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<generated-or-existing-profile>".to_owned());
    let required_mode = required_mode_for_safety_class(&record.safety_class);
    let descriptor = suggestion_action_descriptor(record, required_mode);
    let manual_only_reason = (required_mode == DaemonMode::ApplyHighRisk)
        .then(|| "high-risk apply is not implemented; manual investigation only".to_owned());
    let dry_run_command = Some(apply_profile_command(tree_pid, &profile_arg, true, false));
    let manual_apply_command = if manual_only_reason.is_some() {
        None
    } else {
        manual_apply_command_if_policy_allows(tree_pid, &profile_arg, required_mode, &descriptor)
    };

    Some(CandidateSuggestion {
        candidate_name: record.candidate_name.clone(),
        action_kind: "cpu_affinity_profile".to_owned(),
        descriptor,
        objective: ObjectiveKind::StutterScore,
        evidence: Vec::new(),
        affected_tasks: record.affected_tasks,
        safety: record.safety_class.clone(),
        reason: reason.into(),
        dry_run_command,
        manual_apply_command,
        required_mode,
        required_safety_class: record.safety_class.clone(),
        manual_only_reason,
    })
}

fn required_mode_for_safety_class(safety_class: &SafetyClass) -> DaemonMode {
    match safety_class {
        SafetyClass::ObserveOnly | SafetyClass::ReversibleLowRisk => DaemonMode::ApplyLowRisk,
        SafetyClass::ReversibleMediumRisk => DaemonMode::ApplyMediumRisk,
        SafetyClass::HighRisk => DaemonMode::ApplyHighRisk,
    }
}

fn suggestion_action_descriptor(
    record: &CandidateDryRunRecord,
    required_mode: DaemonMode,
) -> ActionDescriptor {
    ActionDescriptor {
        action_id: crate::actions::ActionId(format!(
            "cpu-affinity-profile:{}",
            record.candidate_name
        )),
        action_kind: "cpu_affinity_profile".to_owned(),
        safety_class: record.safety_class.clone(),
        effect_scope: ActionEffectScope::LocalProcessTree,
        rollback: RollbackRequirement::RequiredBeforeApply,
        persistent_effect: false,
        touches_system_wide_state: false,
        requires_explicit_target: required_mode.supports_apply(),
        confidence: None,
    }
}

fn policy_for_required_mode(required_mode: DaemonMode) -> Option<DaemonPolicy> {
    match required_mode {
        DaemonMode::ApplyLowRisk => Some(DaemonPolicy::apply_low_risk(ActionSource::Cli)),
        DaemonMode::ApplyMediumRisk => Some(DaemonPolicy::apply_medium_risk(ActionSource::Cli)),
        DaemonMode::Observe | DaemonMode::Suggest | DaemonMode::ApplyHighRisk => None,
    }
}

fn manual_apply_command_if_policy_allows(
    tree_pid: u32,
    profile_arg: &str,
    required_mode: DaemonMode,
    descriptor: &ActionDescriptor,
) -> Option<String> {
    let policy = policy_for_required_mode(required_mode)?;
    policy.check_action(PolicyIntent::Apply, descriptor).ok()?;

    Some(apply_profile_command(
        tree_pid,
        profile_arg,
        false,
        required_mode == DaemonMode::ApplyMediumRisk,
    ))
}

fn apply_profile_command(
    tree_pid: u32,
    profile_arg: &str,
    dry_run: bool,
    allow_medium_risk: bool,
) -> String {
    let mut command =
        format!("stutter apply-profile --tree-pid {tree_pid} --profile {profile_arg}");

    if dry_run {
        command.push_str(" --dry-run");
    }

    if allow_medium_risk {
        command.push_str(" --allow-medium-risk");
    }

    command
}

pub fn suggestions_from_candidates_and_dry_run_records(
    candidates: &[CandidateAction],
    records: &[CandidateDryRunRecord],
    plan_dir: &Path,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: &str,
) -> anyhow::Result<Vec<CandidateSuggestion>> {
    let mut suggestions = Vec::new();

    for (candidate, record) in candidates.iter().zip(records.iter()) {
        let plan_path = candidate_plan_path(candidate, plan_dir);

        if !matches!(candidate, CandidateAction::CpuAffinityProfile { .. })
            && record.eligible
            && record.safety_class <= max_safety_class
        {
            write_candidate_plan_file(&plan_path, candidate, Some(record.affected_tasks))?;
        }

        if let Some(suggestion) = suggestion_from_candidate_dry_run_record(
            candidate,
            record,
            &plan_path,
            profile_path,
            max_safety_class.clone(),
            reason.to_owned(),
        ) {
            suggestions.push(suggestion);
        }
    }

    Ok(suggestions)
}

pub fn suggestions_from_dry_run_records(
    records: &[CandidateDryRunRecord],
    tree_pid: u32,
    profile_path: Option<&Path>,
    max_safety_class: SafetyClass,
    reason: &str,
) -> Vec<CandidateSuggestion> {
    records
        .iter()
        .filter_map(|record| {
            suggestion_from_dry_run_record(
                record,
                tree_pid,
                profile_path,
                max_safety_class.clone(),
                reason.to_owned(),
            )
        })
        .collect()
}

pub fn render_candidate_suggestion(suggestion: &CandidateSuggestion) -> String {
    format!(
        "autotune suggestion:\n  candidate={}\n  action={}\n  action_kind={}\n  action_id={}\n  objective={:?}\n  affected_tasks={}\n  safety={:?}\n  reason=\"{}\"\n  note=\"suggest mode did not apply this change\"\n  required_mode={}\n  required_safety_class={:?}\n  rollback=\"stutter restore\"\n  dry_run_command={}\n  manual_apply_command={}\n  manual_only_reason={}",
        shell_safe_value(&suggestion.candidate_name),
        shell_safe_value(&suggestion.action_kind.replace('_', "-")),
        shell_safe_value(&suggestion.action_kind),
        shell_safe_value(&suggestion.descriptor.action_id.0),
        suggestion.objective,
        suggestion.affected_tasks,
        suggestion.safety,
        escape_quoted_value(&suggestion.reason),
        suggestion.required_mode,
        suggestion.required_safety_class,
        render_optional_command(&suggestion.dry_run_command),
        render_optional_command(&suggestion.manual_apply_command),
        render_optional_command(&suggestion.manual_only_reason)
    )
}

pub fn render_candidate_suggestions(suggestions: &[CandidateSuggestion]) -> String {
    suggestions
        .iter()
        .map(render_candidate_suggestion)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn print_candidate_suggestions(suggestions: &[CandidateSuggestion]) {
    for suggestion in suggestions {
        println!("{}", render_candidate_suggestion(suggestion));
    }
}

pub fn apply_candidate_plan_file(path: &Path, dry_run: bool) -> anyhow::Result<CandidatePlanFile> {
    let bytes = std::fs::read(path)?;
    let plan: CandidatePlanFile = serde_json::from_slice(&bytes)?;

    if plan.schema_version != CandidatePlanFile::SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported_candidate_plan_schema: got {} expected {}",
            plan.schema_version,
            CandidatePlanFile::SCHEMA_VERSION
        );
    }

    if dry_run {
        if let Some(executable) = plan.executable.clone() {
            let candidate = executable.into_candidate();
            let record = dry_run_candidate(&candidate);
            if !record.eligible {
                anyhow::bail!(
                    "candidate_plan_dry_run_failed: candidate '{}' action_kind={} reason={}",
                    plan.candidate.candidate_name,
                    plan.candidate.action_kind,
                    record
                        .reason
                        .as_deref()
                        .unwrap_or("dry-run did not produce an eligible candidate")
                );
            }
        }
        return Ok(plan);
    }

    if plan.descriptor.safety_class == SafetyClass::HighRisk
        || plan.descriptor.touches_system_wide_state
    {
        anyhow::bail!(
            "manual_only_high_risk: candidate '{}' action_kind={} cannot be applied by this command",
            plan.candidate.candidate_name,
            plan.candidate.action_kind
        );
    }

    let Some(executable) = plan.executable.clone() else {
        if let Some(reason) = plan.manual_only_reason.as_deref() {
            anyhow::bail!(
                "candidate_plan_manual_only: candidate '{}' action_kind={} reason={} manual_apply_command={}",
                plan.candidate.candidate_name,
                plan.candidate.action_kind,
                reason,
                plan.manual_apply_command.as_deref().unwrap_or("none")
            );
        }
        anyhow::bail!(
            "candidate_plan_payload_not_executable: candidate '{}' action_kind={} requires an executable candidate payload",
            plan.candidate.candidate_name,
            plan.candidate.action_kind
        );
    };

    let candidate = executable.into_candidate();
    if candidate.descriptor().action_id != plan.descriptor.action_id
        || candidate.action_kind() != plan.descriptor.action_kind
    {
        anyhow::bail!(
            "candidate_plan_descriptor_mismatch: candidate '{}' executable payload does not match descriptor",
            plan.candidate.candidate_name
        );
    }

    let policy = match plan.descriptor.safety_class {
        SafetyClass::ObserveOnly | SafetyClass::ReversibleLowRisk => {
            DaemonPolicy::apply_low_risk(ActionSource::Cli)
        }
        SafetyClass::ReversibleMediumRisk => DaemonPolicy::apply_medium_risk(ActionSource::Cli),
        SafetyClass::HighRisk => {
            anyhow::bail!(
                "manual_only_high_risk: candidate '{}' action_kind={} cannot be applied by this command",
                plan.candidate.candidate_name,
                plan.candidate.action_kind
            );
        }
    };
    policy.check_action(PolicyIntent::Apply, &plan.descriptor)?;

    let executor = crate::autotune::apply::executor_for_candidate(candidate)?;
    let result = executor.apply_with_audit(crate::actions::runner::ActionRunPolicy {
        policy,
        context: crate::daemon_policy::DaemonPolicyContext::default(),
        max_affected_tasks: None,
        max_total_duration: None,
        dry_run: false,
    })?;
    if result.rollback.is_none() {
        anyhow::bail!(
            "candidate_plan_apply_missing_rollback: candidate '{}' action_kind={} applied without rollback token",
            plan.candidate.candidate_name,
            plan.candidate.action_kind
        );
    }

    Ok(plan)
}

fn render_optional_command(command: &Option<String>) -> String {
    match command {
        Some(command) => format!("\"{}\"", escape_quoted_value(command)),
        None => "none".to_owned(),
    }
}

fn shell_safe_value(value: &str) -> String {
    if value.is_empty() {
        return "-".to_owned();
    }

    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/' | '@'))
    {
        value.to_owned()
    } else {
        format!("\"{}\"", escape_quoted_value(value))
    }
}

fn escape_quoted_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());

    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }

    escaped
}

pub trait CandidateDryRunner {
    fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord;
}

#[derive(Default)]
pub struct RealCandidateDryRunner;

impl CandidateDryRunner for RealCandidateDryRunner {
    fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord {
        dry_run_candidate(candidate)
    }
}

pub fn dry_run_candidates(candidates: &[CandidateAction]) -> Vec<CandidateDryRunRecord> {
    let mut runner = RealCandidateDryRunner;
    dry_run_candidates_with_runner(candidates, &mut runner)
}

pub fn dry_run_candidates_with_runner<R: CandidateDryRunner>(
    candidates: &[CandidateAction],
    runner: &mut R,
) -> Vec<CandidateDryRunRecord> {
    candidates
        .iter()
        .map(|candidate| runner.dry_run(candidate))
        .collect()
}

pub fn dry_run_record_from_action_state(
    candidate_name: String,
    safety_class: SafetyClass,
    state: ActionState,
) -> CandidateDryRunRecord {
    let affected_tasks = state.affected_tasks;
    CandidateDryRunRecord {
        candidate_name,
        affected_tasks,
        warnings: state.warnings,
        safety_class,
        eligible: affected_tasks > 0,
        reason: if affected_tasks == 0 {
            Some("dry-run matched zero affected tasks".to_owned())
        } else {
            None
        },
    }
}

pub fn dry_run_candidate(candidate: &CandidateAction) -> CandidateDryRunRecord {
    match candidate {
        CandidateAction::CpuAffinityProfile { plan } => {
            let action = CpuAffinityProfileAction {
                tree_pid: plan.tree_pid,
                profile: plan.profile.clone(),
                force_restore_overwrite: false,
            };
            let safety_class = action.safety_class();

            match action.dry_run() {
                Ok(state) => {
                    dry_run_record_from_action_state(plan.profile_name.clone(), safety_class, state)
                }
                Err(err) => CandidateDryRunRecord {
                    candidate_name: plan.profile_name.clone(),
                    affected_tasks: 0,
                    warnings: Vec::new(),
                    safety_class,
                    eligible: false,
                    reason: Some(format!("dry-run failed: {err:#}")),
                },
            }
        }
        CandidateAction::Nice { plan } => {
            dry_run_planned_action(plan.name.clone(), plan.action.safety_class(), &plan.action)
        }
        CandidateAction::IoPrio { plan } => {
            dry_run_planned_action(plan.name.clone(), plan.action.safety_class(), &plan.action)
        }
        CandidateAction::Uclamp { plan } => {
            dry_run_planned_action(plan.name.clone(), plan.action.safety_class(), &plan.action)
        }
        CandidateAction::CgroupPlacement { plan } => {
            dry_run_planned_action(plan.name.clone(), plan.action.safety_class(), &plan.action)
        }
        CandidateAction::IrqAffinity { plan } => {
            dry_run_planned_action(plan.name.clone(), plan.action.safety_class(), &plan.action)
        }
        CandidateAction::CpuPower { plan } => {
            dry_run_planned_action(plan.name.clone(), plan.action.safety_class(), &plan.action)
        }
        CandidateAction::GpuPower { plan } => {
            dry_run_planned_action(plan.name.clone(), plan.action.safety_class(), &plan.action)
        }
        CandidateAction::VmKnob { plan } => {
            dry_run_planned_action(plan.name.clone(), plan.action.safety_class(), &plan.action)
        }
        CandidateAction::Fake { .. } => {
            panic!("dry-run not implemented for Fake candidate");
        }
    }
}

fn dry_run_planned_action<A: TuningAction>(
    candidate_name: String,
    safety_class: SafetyClass,
    action: &A,
) -> CandidateDryRunRecord {
    match action.dry_run() {
        Ok(state) => dry_run_record_from_action_state(candidate_name, safety_class, state),
        Err(err) => CandidateDryRunRecord {
            candidate_name,
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class,
            eligible: false,
            reason: Some(format!("dry-run failed: {err:#}")),
        },
    }
}

pub fn generate_topology_aware_profile_candidates(
    topology: &TopologyModel,
    tree_pid: u32,
) -> Vec<CandidateAction> {
    generate_topology_aware_profile_candidate_plan(
        topology,
        tree_pid,
        &GeneratedCpuSetPolicy::default(),
    )
    .optimization_candidates
}

pub fn generate_topology_aware_profile_candidates_with_policy(
    topology: &TopologyModel,
    tree_pid: u32,
    policy: &GeneratedCpuSetPolicy,
) -> GeneratedProfileCandidatePlan {
    generate_topology_aware_profile_candidate_plan(topology, tree_pid, policy)
}

pub fn generate_topology_aware_profile_candidate_plan(
    topology: &TopologyModel,
    tree_pid: u32,
    policy: &GeneratedCpuSetPolicy,
) -> GeneratedProfileCandidatePlan {
    let generated = generate_topology_aware_profile_plan(topology, policy);
    let mut optimization_candidates = Vec::new();
    let mut recovery_fallback = None;

    for profile in generated.profiles {
        let candidate = CandidateAction::cpu_affinity_profile(profile.clone(), tree_pid);
        if is_baseline_online_profile(&profile.name) {
            recovery_fallback = Some(candidate);
        } else {
            optimization_candidates.push(candidate);
        }
    }

    GeneratedProfileCandidatePlan {
        optimization_candidates,
        recovery_fallback,
        rejected: generated.rejected,
    }
}

pub fn generate_topology_aware_profiles(topology: &TopologyModel) -> Vec<Profile> {
    generate_topology_aware_profile_plan(topology, &GeneratedCpuSetPolicy::default()).profiles
}

pub fn generate_topology_aware_profiles_with_policy(
    topology: &TopologyModel,
    policy: &GeneratedCpuSetPolicy,
) -> GeneratedTopologyProfilePlan {
    generate_topology_aware_profile_plan(topology, policy)
}

pub fn generate_topology_aware_profile_plan(
    topology: &TopologyModel,
    policy: &GeneratedCpuSetPolicy,
) -> GeneratedTopologyProfilePlan {
    let Some(layout) = CandidateCpuLayout::from_topology(topology) else {
        return GeneratedTopologyProfilePlan {
            profiles: Vec::new(),
            rejected: vec![RejectedCandidateProfile {
                profile_name: "<topology>".to_owned(),
                reason: "no online CPUs available for topology-aware generation".to_owned(),
            }],
        };
    };

    let mut profiles = Vec::new();
    let mut rejected = Vec::new();

    for profile in [
        baseline_online_profile(&layout),
        game_isolate_render_profile(&layout),
        game_compositor_separate_profile(&layout),
        helper_spread_profile(&layout),
        wine_server_dedicated_profile(&layout),
        avoid_smt_contention_profile(&layout),
    ] {
        match validate_generated_profile(&profile, topology, policy) {
            Ok(()) => profiles.push(profile),
            Err(reason) => rejected.push(RejectedCandidateProfile {
                profile_name: profile.name,
                reason,
            }),
        }
    }

    GeneratedTopologyProfilePlan { profiles, rejected }
}

#[derive(Clone, Debug)]
struct CandidateCpuLayout {
    online_mask: crate::affinity::CpuMask,
    render_mask: crate::affinity::CpuMask,
    worker_mask: crate::affinity::CpuMask,
    compositor_mask: crate::affinity::CpuMask,
    helper_mask: crate::affinity::CpuMask,
    wine_server_mask: crate::affinity::CpuMask,
    separate_game_mask: crate::affinity::CpuMask,
    separate_compositor_mask: crate::affinity::CpuMask,
    avoid_smt_render_mask: crate::affinity::CpuMask,
    avoid_smt_compositor_mask: crate::affinity::CpuMask,
    avoid_smt_worker_mask: crate::affinity::CpuMask,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CoreChoice {
    package_id: Option<u32>,
    core_id: Option<u32>,
    numa_node: Option<u32>,
    cpus: Vec<u32>,
    primary_cpu: u32,
    max_mhz: Option<u64>,
}

impl CandidateCpuLayout {
    fn from_topology(topology: &TopologyModel) -> Option<Self> {
        let online_cpus = topology_online_cpus(topology);
        if online_cpus.is_empty() {
            return None;
        }

        let online_mask = cpus_to_mask(&online_cpus)?;
        let cores = topology_core_choices(topology, &online_cpus);
        let render_core_count = if cores.len() >= 6 { 2 } else { 1 };

        let render_cores = cores
            .iter()
            .take(render_core_count)
            .cloned()
            .collect::<Vec<_>>();
        let render_primary_cpus = render_cores
            .iter()
            .map(|core| core.primary_cpu)
            .collect::<Vec<_>>();
        let render_all_cpus = flatten_core_cpus(&render_cores);

        let non_render_cores = cores
            .iter()
            .filter(|core| !render_cores.iter().any(|render| same_core(render, core)))
            .cloned()
            .collect::<Vec<_>>();

        let worker_cpus = if non_render_cores.is_empty() {
            online_cpus.clone()
        } else {
            flatten_core_cpus(&non_render_cores)
        };

        let compositor_core = non_render_cores
            .iter()
            .find(|core| core.cpus.iter().all(|cpu| !render_all_cpus.contains(cpu)))
            .cloned()
            .or_else(|| non_render_cores.first().cloned())
            .or_else(|| cores.first().cloned())?;

        let compositor_cpus = vec![compositor_core.primary_cpu];

        let wine_core = non_render_cores
            .iter()
            .find(|core| !same_core(core, &compositor_core))
            .cloned()
            .or_else(|| Some(compositor_core.clone()))?;
        let wine_server_cpus = wine_core.cpus.clone();

        let separate_compositor_core = compositor_core.clone();
        let separate_compositor_cpus = separate_compositor_core.cpus.clone();
        let separate_game_cpus = cores
            .iter()
            .filter(|core| !same_core(core, &separate_compositor_core))
            .flat_map(|core| core.cpus.iter().copied())
            .collect::<Vec<_>>();
        let separate_game_cpus = if separate_game_cpus.is_empty() {
            online_cpus.clone()
        } else {
            separate_game_cpus
        };

        let render_sibling_set = topology
            .smt_siblings
            .get(render_primary_cpus.first().unwrap_or(&online_cpus[0]))
            .cloned()
            .unwrap_or_else(|| render_all_cpus.clone());

        let avoid_smt_compositor_core = non_render_cores
            .iter()
            .find(|core| {
                core.cpus
                    .iter()
                    .all(|cpu| !render_sibling_set.contains(cpu))
            })
            .cloned()
            .or_else(|| non_render_cores.first().cloned())
            .or_else(|| cores.first().cloned())?;

        let avoid_smt_worker_cpus = online_cpus
            .iter()
            .copied()
            .filter(|cpu| {
                !render_sibling_set.contains(cpu) && !avoid_smt_compositor_core.cpus.contains(cpu)
            })
            .collect::<Vec<_>>();
        let avoid_smt_worker_cpus = if avoid_smt_worker_cpus.is_empty() {
            worker_cpus.clone()
        } else {
            avoid_smt_worker_cpus
        };

        Some(Self {
            online_mask,
            render_mask: cpus_to_mask(&render_primary_cpus)?,
            worker_mask: cpus_to_mask(&worker_cpus)?,
            compositor_mask: cpus_to_mask(&compositor_cpus)?,
            helper_mask: cpus_to_mask(&worker_cpus)?,
            wine_server_mask: cpus_to_mask(&wine_server_cpus)?,
            separate_game_mask: cpus_to_mask(&separate_game_cpus)?,
            separate_compositor_mask: cpus_to_mask(&separate_compositor_cpus)?,
            avoid_smt_render_mask: cpus_to_mask(&render_primary_cpus)?,
            avoid_smt_compositor_mask: cpus_to_mask(&[avoid_smt_compositor_core.primary_cpu])?,
            avoid_smt_worker_mask: cpus_to_mask(&avoid_smt_worker_cpus)?,
        })
    }
}

fn topology_online_cpus(topology: &TopologyModel) -> Vec<u32> {
    let online = topology.online_cpu_ids();
    if online.is_empty() {
        cpu_mask_to_vec(&topology.online_cpus)
    } else {
        online
    }
}

fn topology_core_choices(topology: &TopologyModel, online_cpus: &[u32]) -> Vec<CoreChoice> {
    let mut choices = topology
        .cores
        .iter()
        .filter(|core| core.is_online)
        .filter_map(|core| core_choice_from_core(core, online_cpus))
        .collect::<Vec<_>>();

    if choices.is_empty() {
        choices = online_cpus
            .iter()
            .copied()
            .map(|cpu| CoreChoice {
                package_id: None,
                core_id: Some(cpu),
                numa_node: None,
                cpus: vec![cpu],
                primary_cpu: cpu,
                max_mhz: topology.cpu_info(cpu).and_then(|info| info.max_mhz),
            })
            .collect();
    }

    choices.sort_by(|left, right| {
        right
            .max_mhz
            .unwrap_or(0)
            .cmp(&left.max_mhz.unwrap_or(0))
            .then_with(|| left.package_id.cmp(&right.package_id))
            .then_with(|| left.numa_node.cmp(&right.numa_node))
            .then_with(|| left.core_id.cmp(&right.core_id))
            .then_with(|| left.primary_cpu.cmp(&right.primary_cpu))
    });

    choices
}

fn core_choice_from_core(core: &CoreInfo, online_cpus: &[u32]) -> Option<CoreChoice> {
    let cpus = sorted_unique(
        core.cpus
            .iter()
            .copied()
            .filter(|cpu| online_cpus.contains(cpu))
            .collect(),
    );
    let primary_cpu = cpus.first().copied()?;

    Some(CoreChoice {
        package_id: core.package_id,
        core_id: core.core_id,
        numa_node: core.numa_node,
        cpus,
        primary_cpu,
        max_mhz: core.max_mhz,
    })
}

fn same_core(left: &CoreChoice, right: &CoreChoice) -> bool {
    left.package_id == right.package_id
        && left.core_id == right.core_id
        && left.numa_node == right.numa_node
}

fn flatten_core_cpus(cores: &[CoreChoice]) -> Vec<u32> {
    sorted_unique(
        cores
            .iter()
            .flat_map(|core| core.cpus.iter().copied())
            .collect(),
    )
}

fn baseline_online_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "baseline-online".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(layout.online_mask.clone()),
            nice: None,
            ionice: None,
            match_class: Vec::new(),
            match_comm: Vec::new(),
        }],
    }
}

fn game_isolate_render_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "game-isolate-render".to_owned(),
        rules: vec![
            profile_rule(
                &layout.render_mask,
                vec![TaskClass::GameRenderThread],
                Vec::new(),
            ),
            profile_rule(
                &layout.render_mask,
                vec![TaskClass::Game],
                vec!["RenderThread", "Main"],
            ),
            profile_rule(
                &layout.compositor_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
            profile_rule(
                &layout.wine_server_mask,
                vec![TaskClass::WineServer],
                Vec::new(),
            ),
            profile_rule(
                &layout.worker_mask,
                vec![TaskClass::GameWorkerThread, TaskClass::GameHelper],
                Vec::new(),
            ),
            profile_rule(&layout.worker_mask, vec![TaskClass::Game], Vec::new()),
        ],
    }
}

fn game_compositor_separate_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "game-compositor-separate".to_owned(),
        rules: vec![
            profile_rule(
                &layout.separate_compositor_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
            profile_rule(
                &layout.separate_game_mask,
                vec![
                    TaskClass::Game,
                    TaskClass::GameRenderThread,
                    TaskClass::GameWorkerThread,
                    TaskClass::GameHelper,
                    TaskClass::WineServer,
                ],
                Vec::new(),
            ),
        ],
    }
}

fn helper_spread_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "helper-spread".to_owned(),
        rules: vec![profile_rule(
            &layout.helper_mask,
            vec![
                TaskClass::GameHelper,
                TaskClass::GameWorkerThread,
                TaskClass::SteamRuntime,
                TaskClass::Helper,
            ],
            Vec::new(),
        )],
    }
}

fn wine_server_dedicated_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "wine-server-dedicated".to_owned(),
        rules: vec![profile_rule(
            &layout.wine_server_mask,
            vec![TaskClass::WineServer],
            Vec::new(),
        )],
    }
}

fn avoid_smt_contention_profile(layout: &CandidateCpuLayout) -> Profile {
    Profile {
        name: "avoid-smt-contention".to_owned(),
        rules: vec![
            profile_rule(
                &layout.avoid_smt_render_mask,
                vec![TaskClass::GameRenderThread],
                Vec::new(),
            ),
            profile_rule(
                &layout.avoid_smt_render_mask,
                vec![TaskClass::Game],
                vec!["RenderThread", "Main"],
            ),
            profile_rule(
                &layout.avoid_smt_compositor_mask,
                vec![TaskClass::Compositor, TaskClass::GameScope],
                Vec::new(),
            ),
            profile_rule(
                &layout.avoid_smt_worker_mask,
                vec![
                    TaskClass::GameWorkerThread,
                    TaskClass::GameHelper,
                    TaskClass::WineServer,
                ],
                Vec::new(),
            ),
        ],
    }
}

fn profile_rule(
    affinity: &crate::affinity::CpuMask,
    match_class: Vec<TaskClass>,
    match_comm: Vec<&str>,
) -> ProfileRule {
    ProfileRule {
        affinity: Some(affinity.clone()),
        nice: None,
        ionice: None,
        match_class,
        match_comm: match_comm
            .into_iter()
            .map(|pattern| {
                CompiledPattern::new(pattern.to_owned())
                    .expect("generated candidate command pattern must be valid")
            })
            .collect(),
    }
}

fn validate_generated_profile(
    profile: &Profile,
    topology: &TopologyModel,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    if profile.rules.is_empty() {
        return Err("generated profile has no rules".to_owned());
    }

    let online = &topology.online_cpus;

    for (index, rule) in profile.rules.iter().enumerate() {
        validate_generated_rule_mask(profile, index, rule, online, policy)?;
    }

    validate_render_and_game_minimums(profile, policy)?;
    validate_compositor_minimum(profile, policy)?;
    validate_background_capacity(profile, policy)?;

    Ok(())
}

fn validate_generated_rule_mask(
    profile: &Profile,
    index: usize,
    rule: &ProfileRule,
    online: &crate::affinity::CpuMask,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    let Some(affinity) = rule.affinity.as_ref() else {
        return Err(format!("rule {index} is missing CPU mask"));
    };

    if affinity.is_empty() {
        return Err(format!("rule {index} has empty CPU mask"));
    }

    if !affinity.is_subset_of(online) {
        return Err(format!(
            "rule {index} requests offline CPUs: requested={} online={}",
            affinity.to_range_string(),
            online.to_range_string()
        ));
    }

    if let Some(allowed) = &policy.allowed_cpus
        && !affinity.is_subset_of(allowed)
    {
        return Err(format!(
            "rule {index} violates allowed CPU set: requested={} allowed={}",
            affinity.to_range_string(),
            allowed.to_range_string()
        ));
    }

    if let Some(denied) = &policy.denied_cpus {
        let requested = cpu_mask_to_vec(affinity);
        let denied = cpu_mask_to_vec(denied);
        let overlap = requested
            .into_iter()
            .filter(|cpu| denied.contains(cpu))
            .collect::<Vec<_>>();

        if !overlap.is_empty() {
            return Err(format!(
                "rule {index} violates denied CPU set: requested={} denied={} overlap={}",
                affinity.to_range_string(),
                policy
                    .denied_cpus
                    .as_ref()
                    .map(|mask| mask.to_range_string())
                    .unwrap_or_default(),
                crate::topology::cpus_to_range_string(&overlap)
            ));
        }
    }

    if profile.name != "baseline-online" && rule_matches_audio_or_input(rule) {
        return Err(format!(
            "rule {index} targets critical realtime/input classes in generated profile"
        ));
    }

    Ok(())
}

fn validate_render_and_game_minimums(
    profile: &Profile,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    for (index, rule) in profile.rules.iter().enumerate() {
        let Some(affinity) = rule.affinity.as_ref() else {
            return Err(format!("rule {index} is missing CPU mask"));
        };
        let cpu_count = cpu_mask_to_vec(affinity).len();

        if rule_matches_render_or_main_game(rule) && cpu_count < policy.min_render_cpus {
            return Err(format!(
                "rule {index} gives render/main game work fewer than minimum CPUs: cpus={} min={}",
                cpu_count, policy.min_render_cpus
            ));
        }

        if rule_matches_game_work(rule) && cpu_count < policy.min_game_cpus {
            return Err(format!(
                "rule {index} gives game work fewer than minimum CPUs: cpus={} min={}",
                cpu_count, policy.min_game_cpus
            ));
        }
    }

    Ok(())
}

fn validate_compositor_minimum(
    profile: &Profile,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    for (index, rule) in profile.rules.iter().enumerate() {
        if !rule_matches_compositor_or_gamescope(rule) {
            continue;
        }

        let Some(affinity) = rule.affinity.as_ref() else {
            return Err(format!("rule {index} is missing CPU mask"));
        };
        let cpu_count = cpu_mask_to_vec(affinity).len();
        if cpu_count < policy.min_compositor_cpus {
            return Err(format!(
                "rule {index} gives compositor/gamescope fewer than minimum CPUs: cpus={} min={}",
                cpu_count, policy.min_compositor_cpus
            ));
        }
    }

    Ok(())
}

fn validate_background_capacity(
    profile: &Profile,
    policy: &GeneratedCpuSetPolicy,
) -> Result<(), String> {
    if profile.name == "baseline-online" {
        return Ok(());
    }

    for (index, rule) in profile.rules.iter().enumerate() {
        if !rule_matches_background_or_helper_work(rule) {
            continue;
        }

        let Some(affinity) = rule.affinity.as_ref() else {
            return Err(format!("rule {index} is missing CPU mask"));
        };
        let cpu_count = cpu_mask_to_vec(affinity).len();
        if cpu_count < policy.min_background_cpus {
            return Err(format!(
                "rule {index} pushes background/helper work onto too few CPUs: cpus={} min={}",
                cpu_count, policy.min_background_cpus
            ));
        }
    }

    Ok(())
}

fn rule_matches_render_or_main_game(rule: &ProfileRule) -> bool {
    rule.match_class.contains(&TaskClass::GameRenderThread)
        || (rule.match_class.contains(&TaskClass::Game)
            && rule.match_comm.iter().any(|pattern| {
                let raw = pattern.raw().to_ascii_lowercase();
                raw.contains("render") || raw.contains("main")
            }))
}

fn rule_matches_game_work(rule: &ProfileRule) -> bool {
    rule.match_class.iter().any(|class| {
        matches!(
            class,
            TaskClass::Game
                | TaskClass::GameRenderThread
                | TaskClass::GameWorkerThread
                | TaskClass::GameHelper
                | TaskClass::WineServer
        )
    })
}

fn rule_matches_compositor_or_gamescope(rule: &ProfileRule) -> bool {
    rule.match_class
        .iter()
        .any(|class| matches!(class, TaskClass::Compositor | TaskClass::GameScope))
}

fn rule_matches_background_or_helper_work(rule: &ProfileRule) -> bool {
    rule.match_class.iter().any(|class| {
        matches!(
            class,
            TaskClass::GameWorkerThread
                | TaskClass::GameHelper
                | TaskClass::SteamRuntime
                | TaskClass::Helper
                | TaskClass::WineServer
        )
    })
}

fn rule_matches_audio_or_input(rule: &ProfileRule) -> bool {
    rule.match_class
        .iter()
        .any(|class| matches!(class, TaskClass::AudioRealtime | TaskClass::Input))
}

fn is_baseline_online_profile(name: &str) -> bool {
    name == "baseline-online"
}

pub fn generate_profile_candidates(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
) -> Vec<CandidateAction> {
    generate_profile_candidate_plan(profiles, tree_pid, current_profile).optimization_candidates
}

pub fn generate_profile_candidates_for_observation(
    profiles: &[Profile],
    observation: &crate::autotune::observation::AutotuneObservation,
) -> Vec<CandidateAction> {
    generate_profile_candidate_plan_for_observation(profiles, observation).optimization_candidates
}

pub fn generate_profile_candidate_plan_for_observation(
    profiles: &[Profile],
    observation: &crate::autotune::observation::AutotuneObservation,
) -> GeneratedProfileCandidatePlan {
    let Some(tree_pid) = observation.target_root_pid else {
        return GeneratedProfileCandidatePlan {
            optimization_candidates: Vec::new(),
            recovery_fallback: None,
            rejected: Vec::new(),
        };
    };
    let recently_failed_profiles = BTreeSet::new();
    generate_profile_candidate_plan_with_checker(
        profiles,
        tree_pid,
        None,
        &recently_failed_profiles,
        |profile| {
            let matched_tasks = crate::profiles::profile_matched_task_count_from_snapshots(
                &observation.active_tasks,
                profile,
            );
            Ok(CandidateProfileStatus {
                matched_tasks,
                dry_run_tasks: matched_tasks,
            })
        },
    )
}

pub fn generate_profile_candidate_plan(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
) -> GeneratedProfileCandidatePlan {
    let recently_failed_profiles = BTreeSet::new();
    generate_profile_candidate_plan_with_history(
        profiles,
        tree_pid,
        current_profile,
        &recently_failed_profiles,
    )
}

pub fn generate_profile_candidate_plan_with_history(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
    recently_failed_profiles: &BTreeSet<String>,
) -> GeneratedProfileCandidatePlan {
    generate_profile_candidate_plan_with_checker(
        profiles,
        tree_pid,
        current_profile,
        recently_failed_profiles,
        |profile| check_profile_for_candidate(tree_pid, profile),
    )
}

fn generate_profile_candidate_plan_with_checker<F>(
    profiles: &[Profile],
    tree_pid: u32,
    current_profile: Option<&str>,
    recently_failed_profiles: &BTreeSet<String>,
    mut check_profile: F,
) -> GeneratedProfileCandidatePlan
where
    F: FnMut(&Profile) -> anyhow::Result<CandidateProfileStatus>,
{
    let mut preferred_candidates = Vec::new();
    let mut recently_failed_candidates = Vec::new();
    let mut recovery_fallback = None;
    let mut rejected = Vec::new();

    for profile in profiles {
        if Some(profile.name.as_str()) == current_profile {
            rejected.push(RejectedCandidateProfile {
                profile_name: profile.name.clone(),
                reason: "current profile".to_owned(),
            });
            continue;
        }

        let status = match check_profile(profile) {
            Ok(status) => status,
            Err(err) => {
                rejected.push(RejectedCandidateProfile {
                    profile_name: profile.name.clone(),
                    reason: format!("dry-run failed: {err:#}"),
                });
                continue;
            }
        };

        if status.matched_tasks == 0 {
            rejected.push(RejectedCandidateProfile {
                profile_name: profile.name.clone(),
                reason: "zero matched tasks".to_owned(),
            });
            continue;
        }

        let candidate = CandidateAction::cpu_affinity_profile(profile.clone(), tree_pid);

        if is_baseline_online_profile(&profile.name) {
            recovery_fallback = Some(candidate);
            continue;
        }

        if recently_failed_profiles.contains(&profile.name) {
            recently_failed_candidates.push(candidate);
        } else {
            preferred_candidates.push(candidate);
        }
    }

    preferred_candidates.extend(recently_failed_candidates);

    GeneratedProfileCandidatePlan {
        optimization_candidates: preferred_candidates,
        recovery_fallback,
        rejected,
    }
}

fn check_profile_for_candidate(
    tree_pid: u32,
    profile: &Profile,
) -> anyhow::Result<CandidateProfileStatus> {
    let matched_tasks = crate::profiles::profile_matched_task_count_for_tree(tree_pid, profile);
    let dry_run_records = crate::profiles::apply_profile_to_tree(tree_pid, profile, false, true)?;

    Ok(CandidateProfileStatus {
        matched_tasks,
        dry_run_tasks: dry_run_records.len(),
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use super::*;
    use crate::{
        actions::{
            TaskIdentity,
            irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
            nice::{NiceAction, NicePolicy},
        },
        affinity::CpuMask,
        process_tree::TaskClass,
        profiles::ProfileRule,
        topology::{CoreInfo, CpuInfo, TopologyModel},
    };

    fn fake_topology_4c8t() -> TopologyModel {
        let online_cpus = CpuMask::parse("0-7").unwrap();

        TopologyModel {
            online_cpus,
            cpus: vec![
                fake_cpu(0, 0, 0, 0, 5000),
                fake_cpu(1, 1, 0, 0, 4900),
                fake_cpu(2, 2, 0, 0, 4800),
                fake_cpu(3, 3, 0, 0, 4700),
                fake_cpu(4, 0, 0, 0, 5000),
                fake_cpu(5, 1, 0, 0, 4900),
                fake_cpu(6, 2, 0, 0, 4800),
                fake_cpu(7, 3, 0, 0, 4700),
            ],
            cores: vec![
                fake_core(0, 0, 0, vec![0, 4], 5000),
                fake_core(1, 0, 0, vec![1, 5], 4900),
                fake_core(2, 0, 0, vec![2, 6], 4800),
                fake_core(3, 0, 0, vec![3, 7], 4700),
            ],
            smt_siblings: BTreeMap::from([
                (0, vec![0, 4]),
                (4, vec![0, 4]),
                (1, vec![1, 5]),
                (5, vec![1, 5]),
                (2, vec![2, 6]),
                (6, vec![2, 6]),
                (3, vec![3, 7]),
                (7, vec![3, 7]),
            ]),
            numa_nodes: BTreeMap::from([(0, vec![0, 1, 2, 3, 4, 5, 6, 7])]),
            packages: BTreeMap::from([(0, vec![0, 1, 2, 3, 4, 5, 6, 7])]),
        }
    }

    fn fake_cpu(cpu: u32, core_id: u32, package_id: u32, numa_node: u32, max_mhz: u64) -> CpuInfo {
        CpuInfo {
            cpu,
            core_id: Some(core_id),
            package_id: Some(package_id),
            numa_node: Some(numa_node),
            max_mhz: Some(max_mhz),
            is_online: true,
        }
    }

    fn fake_core(
        core_id: u32,
        package_id: u32,
        numa_node: u32,
        cpus: Vec<u32>,
        max_mhz: u64,
    ) -> CoreInfo {
        CoreInfo {
            core_id: Some(core_id),
            package_id: Some(package_id),
            numa_node: Some(numa_node),
            cpus,
            max_mhz: Some(max_mhz),
            is_online: true,
        }
    }

    fn profile_by_name<'a>(profiles: &'a [Profile], name: &str) -> &'a Profile {
        profiles
            .iter()
            .find(|profile| profile.name == name)
            .unwrap_or_else(|| panic!("missing generated profile {name}"))
    }

    fn first_rule_for_class(profile: &Profile, class: TaskClass) -> &ProfileRule {
        profile
            .rules
            .iter()
            .find(|rule| rule.match_class.contains(&class))
            .unwrap_or_else(|| panic!("missing rule for {class:?} in {}", profile.name))
    }

    fn affinity(rule: &ProfileRule) -> &CpuMask {
        rule.affinity.as_ref().unwrap()
    }

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    fn status_for_profile(profile: &Profile) -> anyhow::Result<CandidateProfileStatus> {
        match profile.name.as_str() {
            "dry-run-fails" => anyhow::bail!("intentional dry-run failure"),
            "zero-match" => Ok(CandidateProfileStatus {
                matched_tasks: 0,
                dry_run_tasks: 0,
            }),
            _ => Ok(CandidateProfileStatus {
                matched_tasks: 2,
                dry_run_tasks: 1,
            }),
        }
    }

    #[test]
    fn topology_aware_generation_includes_required_templates() {
        let topology = fake_topology_4c8t();
        let profiles = generate_topology_aware_profiles(&topology);
        let names = profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "baseline-online",
                "game-isolate-render",
                "game-compositor-separate",
                "helper-spread",
                "wine-server-dedicated",
                "avoid-smt-contention"
            ]
        );
    }

    #[test]
    fn baseline_online_profile_matches_all_target_tasks_on_online_cpus() {
        let topology = fake_topology_4c8t();
        let profiles = generate_topology_aware_profiles(&topology);
        let baseline = profile_by_name(&profiles, "baseline-online");

        assert_eq!(baseline.rules.len(), 1);
        assert_eq!(affinity(&baseline.rules[0]).to_range_string(), "0-7");
        assert!(baseline.rules[0].match_class.is_empty());
        assert!(baseline.rules[0].match_comm.is_empty());
    }

    #[test]
    fn game_isolate_render_uses_preferred_and_remaining_physical_cores() {
        let topology = fake_topology_4c8t();
        let profiles = generate_topology_aware_profiles(&topology);
        let profile = profile_by_name(&profiles, "game-isolate-render");

        let render_rule = first_rule_for_class(profile, TaskClass::GameRenderThread);
        let compositor_rule = first_rule_for_class(profile, TaskClass::Compositor);
        let worker_rule = first_rule_for_class(profile, TaskClass::GameWorkerThread);
        let wine_rule = first_rule_for_class(profile, TaskClass::WineServer);

        assert_eq!(affinity(render_rule).to_range_string(), "0");
        assert_eq!(affinity(compositor_rule).to_range_string(), "1");
        assert_eq!(affinity(worker_rule).to_range_string(), "1-3,5-7");
        assert_eq!(affinity(wine_rule).to_range_string(), "2,6");

        let game_main_rule = profile
            .rules
            .iter()
            .find(|rule| {
                rule.match_class.contains(&TaskClass::Game)
                    && rule
                        .match_comm
                        .iter()
                        .any(|pattern| pattern.raw() == "Main")
            })
            .unwrap();
        assert_eq!(affinity(game_main_rule).to_range_string(), "0");
    }

    #[test]
    fn game_compositor_separate_keeps_game_and_compositor_on_separate_physical_cores() {
        let topology = fake_topology_4c8t();
        let profiles = generate_topology_aware_profiles(&topology);
        let profile = profile_by_name(&profiles, "game-compositor-separate");

        let compositor_rule = first_rule_for_class(profile, TaskClass::Compositor);
        let game_rule = first_rule_for_class(profile, TaskClass::Game);

        assert_eq!(affinity(compositor_rule).to_range_string(), "1,5");
        assert_eq!(affinity(game_rule).to_range_string(), "0,2-4,6-7");
    }

    #[test]
    fn helper_spread_uses_non_critical_cores() {
        let topology = fake_topology_4c8t();
        let profiles = generate_topology_aware_profiles(&topology);
        let profile = profile_by_name(&profiles, "helper-spread");
        let helper_rule = first_rule_for_class(profile, TaskClass::GameHelper);

        assert_eq!(affinity(helper_rule).to_range_string(), "1-3,5-7");
        assert!(
            helper_rule
                .match_class
                .contains(&TaskClass::GameWorkerThread)
        );
        assert!(helper_rule.match_class.contains(&TaskClass::SteamRuntime));
        assert!(helper_rule.match_class.contains(&TaskClass::Helper));
    }

    #[test]
    fn wine_server_dedicated_uses_one_non_render_core_pair() {
        let topology = fake_topology_4c8t();
        let profiles = generate_topology_aware_profiles(&topology);
        let profile = profile_by_name(&profiles, "wine-server-dedicated");
        let wine_rule = first_rule_for_class(profile, TaskClass::WineServer);

        assert_eq!(affinity(wine_rule).to_range_string(), "2,6");
    }

    #[test]
    fn avoid_smt_contention_keeps_render_and_compositor_off_smt_siblings() {
        let topology = fake_topology_4c8t();
        let profiles = generate_topology_aware_profiles(&topology);
        let profile = profile_by_name(&profiles, "avoid-smt-contention");

        let render_rule = first_rule_for_class(profile, TaskClass::GameRenderThread);
        let compositor_rule = first_rule_for_class(profile, TaskClass::Compositor);
        let worker_rule = first_rule_for_class(profile, TaskClass::GameWorkerThread);

        assert_eq!(affinity(render_rule).to_range_string(), "0");
        assert_eq!(affinity(compositor_rule).to_range_string(), "1");
        assert_eq!(affinity(worker_rule).to_range_string(), "2-3,6-7");

        let render_siblings = topology.smt_siblings.get(&0).unwrap();
        let compositor_cpus =
            crate::topology::parse_cpu_list(&affinity(compositor_rule).to_range_string()).unwrap();

        assert!(
            compositor_cpus
                .iter()
                .all(|cpu| !render_siblings.contains(cpu))
        );
    }

    #[test]
    fn topology_aware_candidates_wrap_generated_profiles_for_tree_pid() {
        let topology = fake_topology_4c8t();
        let candidates = generate_topology_aware_profile_candidates(&topology, 1234);

        assert_eq!(candidates.len(), 5);
        assert_eq!(candidates[0].profile_name(), "game-isolate-render");
        assert_eq!(candidates[0].tree_pid(), 1234);
        assert_eq!(candidates[1].profile_name(), "game-compositor-separate");
        assert_eq!(candidates[1].action_kind(), "cpu_affinity_profile");
    }

    #[test]
    fn generated_profile_plan_rejects_masks_outside_allowed_cpus() {
        let topology = fake_topology_4c8t();
        let policy = GeneratedCpuSetPolicy {
            allowed_cpus: Some(crate::affinity::CpuMask::parse("0-3").unwrap()),
            denied_cpus: None,
            min_render_cpus: 1,
            min_game_cpus: 1,
            min_compositor_cpus: 1,
            min_background_cpus: 2,
        };

        let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

        assert!(!plan.rejected.is_empty());
        assert!(plan.rejected.iter().any(|rejected| {
            rejected.reason.contains("violates allowed CPU set")
                && rejected.reason.contains("allowed=0-3")
        }));
    }

    #[test]
    fn generated_profile_plan_rejects_masks_overlapping_denied_cpus() {
        let topology = fake_topology_4c8t();
        let policy = GeneratedCpuSetPolicy {
            allowed_cpus: None,
            denied_cpus: Some(crate::affinity::CpuMask::parse("1").unwrap()),
            min_render_cpus: 1,
            min_game_cpus: 1,
            min_compositor_cpus: 1,
            min_background_cpus: 2,
        };

        let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

        assert!(!plan.rejected.is_empty());
        assert!(plan.rejected.iter().any(|rejected| {
            rejected.reason.contains("violates denied CPU set")
                && rejected.reason.contains("overlap=1")
        }));
    }

    #[test]
    fn generated_profile_plan_rejects_render_mask_below_minimum() {
        let topology = fake_topology_4c8t();
        let policy = GeneratedCpuSetPolicy {
            allowed_cpus: None,
            denied_cpus: None,
            min_render_cpus: 2,
            min_game_cpus: 1,
            min_compositor_cpus: 1,
            min_background_cpus: 2,
        };

        let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

        assert!(plan.rejected.iter().any(|rejected| {
            rejected.profile_name == "game-isolate-render"
                && rejected
                    .reason
                    .contains("render/main game work fewer than minimum CPUs")
        }));
    }

    #[test]
    fn generated_profile_plan_rejects_background_helper_single_cpu_overload() {
        let topology = fake_topology_4c8t();
        let policy = GeneratedCpuSetPolicy {
            allowed_cpus: None,
            denied_cpus: None,
            min_render_cpus: 1,
            min_game_cpus: 1,
            min_compositor_cpus: 1,
            min_background_cpus: 7,
        };

        let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

        assert!(plan.rejected.iter().any(|rejected| {
            rejected
                .reason
                .contains("background/helper work onto too few CPUs")
        }));
    }

    #[test]
    fn generated_profile_validation_rejects_offline_cpu_masks() {
        let topology = fake_topology_4c8t();
        let profile = Profile {
            name: "bad-offline".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(crate::affinity::CpuMask::parse("0,99").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };

        let err =
            validate_generated_profile(&profile, &topology, &GeneratedCpuSetPolicy::default())
                .unwrap_err();

        assert!(err.contains("requests offline CPUs"));
        assert!(err.contains("requested=0,99"));
        assert!(err.contains("online=0-7"));
    }

    #[test]
    fn generated_profile_validation_rejects_empty_masks() {
        let topology = fake_topology_4c8t();
        let profile = Profile {
            name: "bad-empty".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(
                    crate::affinity::CpuMask::parse("")
                        .unwrap_or_else(|_| crate::affinity::CpuMask::parse("0").unwrap()),
                ),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };

        if profile.rules[0].affinity.as_ref().unwrap().is_empty() {
            let err =
                validate_generated_profile(&profile, &topology, &GeneratedCpuSetPolicy::default())
                    .unwrap_err();
            assert!(err.contains("empty CPU mask"));
        } else {
            assert!(!profile.rules[0].affinity.as_ref().unwrap().is_empty());
        }
    }

    #[test]
    fn generated_profile_validation_rejects_compositor_zero_cpu_equivalent_empty_mask() {
        let topology = fake_topology_4c8t();
        let policy = GeneratedCpuSetPolicy {
            allowed_cpus: Some(crate::affinity::CpuMask::parse("0").unwrap()),
            denied_cpus: Some(crate::affinity::CpuMask::parse("0").unwrap()),
            min_render_cpus: 1,
            min_game_cpus: 1,
            min_compositor_cpus: 1,
            min_background_cpus: 2,
        };

        let plan = generate_topology_aware_profiles_with_policy(&topology, &policy);

        assert!(plan.rejected.iter().any(|rejected| {
            rejected.reason.contains("violates denied CPU set")
                || rejected.reason.contains("violates allowed CPU set")
        }));
    }

    #[test]
    fn generated_profile_validation_rejects_audio_realtime_and_input_targets() {
        let topology = fake_topology_4c8t();
        let profile = Profile {
            name: "bad-critical".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(crate::affinity::CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::AudioRealtime, TaskClass::Input],
                match_comm: Vec::new(),
            }],
        };

        let err =
            validate_generated_profile(&profile, &topology, &GeneratedCpuSetPolicy::default())
                .unwrap_err();

        assert!(err.contains("critical realtime/input"));
    }

    #[test]
    fn valid_generated_profile_plan_keeps_safe_templates_and_reports_rejections() {
        let topology = fake_topology_4c8t();
        let plan = generate_topology_aware_profiles_with_policy(
            &topology,
            &GeneratedCpuSetPolicy::default(),
        );
        let names = plan
            .profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>();

        assert!(plan.rejected.is_empty());
        assert_eq!(
            names,
            vec![
                "baseline-online",
                "game-isolate-render",
                "game-compositor-separate",
                "helper-spread",
                "wine-server-dedicated",
                "avoid-smt-contention"
            ]
        );
    }

    #[test]
    fn topology_aware_candidate_plan_separates_baseline_recovery_from_optimization() {
        let topology = fake_topology_4c8t();
        let plan = generate_topology_aware_profile_candidate_plan(
            &topology,
            1234,
            &GeneratedCpuSetPolicy::default(),
        );

        assert_eq!(
            plan.recovery_fallback
                .as_ref()
                .map(CandidateAction::profile_name),
            Some("baseline-online")
        );
        assert!(
            plan.optimization_candidates
                .iter()
                .all(|candidate| candidate.profile_name() != "baseline-online")
        );
        assert!(
            plan.optimization_candidates
                .iter()
                .any(|candidate| candidate.profile_name() == "game-isolate-render")
        );
        assert!(plan.rejected.is_empty());
    }

    #[test]
    fn generate_profile_candidates_excludes_current_profile() {
        let profiles = vec![profile("current"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            Some("current"),
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(
            plan.rejected
                .iter()
                .any(|rejected| rejected.profile_name == "current"
                    && rejected.reason == "current profile")
        );
    }

    #[test]
    fn generate_profile_candidates_excludes_profiles_that_fail_dry_run() {
        let profiles = vec![profile("dry-run-fails"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(
            plan.rejected
                .iter()
                .any(|rejected| rejected.profile_name == "dry-run-fails"
                    && rejected.reason.contains("dry-run failed"))
        );
    }

    #[test]
    fn generate_profile_candidates_excludes_zero_matched_tasks() {
        let profiles = vec![profile("zero-match"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(
            plan.rejected
                .iter()
                .any(|rejected| rejected.profile_name == "zero-match"
                    && rejected.reason == "zero matched tasks")
        );
    }

    #[test]
    fn generate_profile_candidates_puts_recently_failed_names_last() {
        let profiles = vec![
            profile("recently-failed"),
            profile("fresh"),
            profile("another-fresh"),
        ];
        let recently_failed_profiles = BTreeSet::from(["recently-failed".to_owned()]);

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &recently_failed_profiles,
            status_for_profile,
        );

        let names = plan
            .optimization_candidates
            .iter()
            .map(CandidateAction::profile_name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["fresh", "another-fresh", "recently-failed"]);
    }

    #[test]
    fn baseline_online_is_recovery_fallback_not_optimization_candidate() {
        let profiles = vec![profile("baseline-online"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert_eq!(
            plan.recovery_fallback
                .as_ref()
                .map(CandidateAction::profile_name),
            Some("baseline-online")
        );
    }

    #[test]
    fn public_generate_profile_candidates_returns_optimization_candidates_only() {
        let profiles = vec![profile("baseline-online"), profile("candidate")];

        let plan = generate_profile_candidate_plan_with_checker(
            &profiles,
            1234,
            None,
            &BTreeSet::new(),
            status_for_profile,
        );

        assert_eq!(plan.optimization_candidates.len(), 1);
        assert_eq!(plan.optimization_candidates[0].profile_name(), "candidate");
        assert!(plan.recovery_fallback.is_some());
    }

    #[test]
    fn suggestion_from_dry_run_record_renders_requested_shape() {
        let record = CandidateDryRunRecord {
            candidate_name: "game-main-suggested".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        )
        .unwrap();

        let rendered = render_candidate_suggestion(&suggestion);

        assert!(rendered.contains("candidate=game-main-suggested"));
        assert!(rendered.contains("action=cpu-affinity-profile"));
        assert!(rendered.contains("affected_tasks=31"));
        assert!(rendered.contains("safety=ReversibleLowRisk"));
        assert!(
            rendered.contains("reason=\"scheduler pressure detected on Game/WineServer classes\"")
        );
        assert!(rendered.contains("note=\"suggest mode did not apply this change\""));
        assert!(rendered.contains("required_mode=apply-low-risk"));
        assert!(rendered.contains("dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile> --dry-run\""));
        assert!(rendered.contains("manual_apply_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile>\""));
    }

    #[test]
    fn generic_candidate_suggestion_writes_plan_file_and_uses_apply_candidate_command() {
        let plan_dir = temp_candidate_plan_dir("generic-nice");
        let candidate = CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "nice-browser-helper".to_owned(),
                action: NiceAction {
                    targets: vec![TaskIdentity {
                        tid: 1234,
                        process_pid: Some(1234),
                        comm: Some("browser".to_owned()),
                        starttime_ticks: Some(77),
                    }],
                    nice: 5,
                    policy: NicePolicy::default(),
                },
                target_root_pid: Some(1234),
                evidence: vec![CandidateEvidence::new("cpu_pressure", "high", 0.9)],
                objective: ObjectiveKind::DesktopInteractivity,
            },
        };
        let records = vec![CandidateDryRunRecord {
            candidate_name: candidate.candidate_name().to_owned(),
            affected_tasks: 1,
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        }];

        let suggestions = suggestions_from_candidates_and_dry_run_records(
            std::slice::from_ref(&candidate),
            &records,
            &plan_dir,
            None,
            SafetyClass::ReversibleMediumRisk,
            "compile CPU pressure",
        )
        .unwrap();

        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        let plan_path = candidate_plan_path(&candidate, &plan_dir);

        assert!(plan_path.exists());
        assert_eq!(suggestion.candidate_name, "nice-browser-helper");
        assert_eq!(suggestion.action_kind, "nice");
        assert_eq!(suggestion.objective, ObjectiveKind::DesktopInteractivity);
        assert_eq!(suggestion.evidence.len(), 1);
        assert_eq!(suggestion.required_mode, DaemonMode::ApplyMediumRisk);
        assert_eq!(
            suggestion.required_safety_class,
            SafetyClass::ReversibleMediumRisk
        );
        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some(format!(
                "stutter autotune apply-candidate --candidate-json {} --dry-run",
                plan_path.display()
            ))
            .as_deref()
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some(format!(
                "stutter autotune apply-candidate --candidate-json {}",
                plan_path.display()
            ))
            .as_deref()
        );

        let decoded: CandidatePlanFile =
            serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        assert_eq!(decoded.candidate.candidate_name, "nice-browser-helper");
        assert_eq!(decoded.candidate.action_kind, "nice");
        assert!(decoded.executable.is_some());

        let rendered = render_candidate_suggestion(suggestion);
        assert!(rendered.contains("action=nice"));
        assert!(rendered.contains("action_kind=nice"));
        assert!(rendered.contains("dry_run_command=\"stutter autotune apply-candidate"));
        assert!(rendered.contains("manual_apply_command=\"stutter autotune apply-candidate"));
    }

    #[test]
    fn high_risk_system_candidate_suggestion_is_dry_run_only() {
        let plan_dir = temp_candidate_plan_dir("high-risk-irq");
        let candidate = CandidateAction::IrqAffinity {
            plan: IrqAffinityActionPlan {
                name: "irq-affinity-44-high-risk".to_owned(),
                action: IrqAffinityAction::new(
                    44,
                    "gpu".to_owned(),
                    "2".to_owned(),
                    IrqAffinityRisk::HighRisk,
                    IrqAffinityEvidence {
                        strong_irq_evidence: true,
                        stable_irq_identity: false,
                        known_device_mapping: true,
                        observed_irq: Some(44),
                        observed_device_hint: Some("gpu".to_owned()),
                        reason: "test IRQ pressure".to_owned(),
                    },
                ),
                evidence: vec![CandidateEvidence::new("irq", "gpu", 0.8)],
                objective: ObjectiveKind::IrqOverlapReduction,
            },
        };
        let records = vec![CandidateDryRunRecord {
            candidate_name: candidate.candidate_name().to_owned(),
            affected_tasks: 1,
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        }];

        let suggestions = suggestions_from_candidates_and_dry_run_records(
            std::slice::from_ref(&candidate),
            &records,
            &plan_dir,
            None,
            SafetyClass::HighRisk,
            "IRQ overlap detected",
        )
        .unwrap();

        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        let plan_path = candidate_plan_path(&candidate, &plan_dir);

        assert!(plan_path.exists());
        assert_eq!(suggestion.action_kind, "irq_affinity");
        assert_eq!(suggestion.required_mode, DaemonMode::ApplyHighRisk);
        assert_eq!(suggestion.required_safety_class, SafetyClass::HighRisk);
        assert!(suggestion.dry_run_command.is_some());
        assert_eq!(suggestion.manual_apply_command, None);
        assert!(
            suggestion
                .manual_only_reason
                .as_deref()
                .unwrap_or_default()
                .contains("manual-only high-risk/system-adjacent")
        );

        let dry_run_plan = apply_candidate_plan_file(&plan_path, true).unwrap();
        assert_eq!(
            dry_run_plan.candidate.candidate_name,
            "irq-affinity-44-high-risk"
        );
        assert!(dry_run_plan.executable.is_none());

        let err = apply_candidate_plan_file(&plan_path, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("manual_only_high_risk"));
    }

    #[test]
    fn cpu_affinity_suggestion_preserves_apply_profile_and_does_not_write_plan_file() {
        let plan_dir = temp_candidate_plan_dir("cpu-affinity-preserve-apply-profile");
        let profile = Profile {
            name: "game".to_owned(),
            rules: Vec::new(),
        };
        let candidate = CandidateAction::CpuAffinityProfile {
            plan: CpuAffinityProfilePlan {
                profile_name: "game".to_owned(),
                profile,
                tree_pid: 1234,
            },
        };
        let records = vec![CandidateDryRunRecord {
            candidate_name: candidate.candidate_name().to_owned(),
            affected_tasks: 1,
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        }];
        let profile_path = Path::new("/tmp/profile.toml");

        let suggestions = suggestions_from_candidates_and_dry_run_records(
            std::slice::from_ref(&candidate),
            &records,
            &plan_dir,
            Some(profile_path),
            SafetyClass::ReversibleMediumRisk,
            "scheduler pressure",
        )
        .unwrap();

        assert_eq!(suggestions.len(), 1);
        let suggestion = &suggestions[0];
        assert_eq!(suggestion.action_kind, "cpu_affinity_profile");
        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profile.toml --dry-run")
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profile.toml")
        );
        assert!(!candidate_plan_path(&candidate, &plan_dir).exists());
    }

    #[test]
    fn candidate_plan_file_can_embed_executable_process_local_payload() {
        let candidate = CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "nice-browser-helper".to_owned(),
                action: NiceAction {
                    targets: vec![TaskIdentity {
                        tid: 1234,
                        process_pid: Some(1234),
                        comm: Some("browser".to_owned()),
                        starttime_ticks: Some(77),
                    }],
                    nice: 5,
                    policy: NicePolicy::default(),
                },
                target_root_pid: Some(1234),
                evidence: vec![CandidateEvidence::new("cpu_pressure", "high", 0.9)],
                objective: ObjectiveKind::DesktopInteractivity,
            },
        };

        let plan = CandidatePlanFile::from_candidate(&candidate, Some(1));
        let json = serde_json::to_string(&plan).unwrap();
        let decoded: CandidatePlanFile = serde_json::from_str(&json).unwrap();

        assert!(decoded.executable.is_some());
        let decoded_candidate = decoded.executable.unwrap().into_candidate();
        assert_eq!(decoded_candidate.action_kind(), "nice");
        assert_eq!(decoded_candidate.candidate_name(), "nice-browser-helper");
    }

    #[test]
    fn cpu_affinity_candidate_plan_file_is_manual_only_with_stable_rejection() {
        let plan_dir = temp_candidate_plan_dir("cpu-affinity-plan-manual-only");
        let candidate = CandidateAction::CpuAffinityProfile {
            plan: CpuAffinityProfilePlan {
                profile_name: "game".to_owned(),
                profile: Profile {
                    name: "game".to_owned(),
                    rules: vec![ProfileRule {
                        affinity: Some(CpuMask::parse("0").unwrap()),
                        nice: None,
                        ionice: None,
                        match_class: vec![TaskClass::Game],
                        match_comm: Vec::new(),
                    }],
                },
                tree_pid: 1234,
            },
        };
        let plan_path = candidate_plan_path(&candidate, &plan_dir);

        let plan = write_candidate_plan_file(&plan_path, &candidate, Some(1)).unwrap();
        assert!(plan.executable.is_none());
        assert_eq!(
            plan.manual_apply_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile>")
        );
        assert_eq!(
            plan.manual_only_reason.as_deref(),
            Some("cpu-affinity profiles use apply-profile, not candidate-plan apply")
        );

        let decoded: CandidatePlanFile =
            serde_json::from_slice(&std::fs::read(&plan_path).unwrap()).unwrap();
        assert!(decoded.executable.is_none());
        assert_eq!(decoded.manual_only_reason, plan.manual_only_reason);

        let err = apply_candidate_plan_file(&plan_path, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("candidate_plan_manual_only"));
        assert!(err.contains("apply-profile"));
    }

    #[test]
    fn suggestion_from_dry_run_record_uses_existing_profile_path_when_available() {
        let record = CandidateDryRunRecord {
            candidate_name: "game-main-suggested".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            Some(Path::new("/tmp/profiles.toml")),
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        )
        .unwrap();

        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profiles.toml --dry-run")
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile /tmp/profiles.toml")
        );
    }

    #[test]
    fn suggestion_from_dry_run_record_skips_ineligible_candidate() {
        let record = CandidateDryRunRecord {
            candidate_name: "bad-candidate".to_owned(),
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: false,
            reason: Some("dry-run matched zero affected tasks".to_owned()),
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert!(suggestion.is_none());
    }

    #[test]
    fn render_candidate_suggestion_escapes_reason_and_commands() {
        let suggestion = CandidateSuggestion {
            candidate_name: "candidate with space".to_owned(),
            action_kind: "cpu_affinity_profile".to_owned(),
            descriptor: ActionDescriptor {
                action_id: crate::actions::ActionId(
                    "cpu-affinity-profile:candidate with space".to_owned(),
                ),
                action_kind: "cpu_affinity_profile".to_owned(),
                safety_class: SafetyClass::ReversibleLowRisk,
                effect_scope: ActionEffectScope::LocalProcessTree,
                rollback: RollbackRequirement::RequiredBeforeApply,
                persistent_effect: false,
                touches_system_wide_state: false,
                requires_explicit_target: true,
                confidence: None,
            },
            objective: ObjectiveKind::StutterScore,
            evidence: Vec::new(),
            affected_tasks: 31,
            safety: SafetyClass::ReversibleLowRisk,
            reason: "scheduler \"pressure\"\nnext".to_owned(),
            dry_run_command: Some(
                "stutter apply-profile --tree-pid 1234 --profile /tmp/profile \"quoted\".toml --dry-run"
                    .to_owned(),
            ),
            manual_apply_command: Some(
                "stutter apply-profile --tree-pid 1234 --profile /tmp/profile \"quoted\".toml"
                    .to_owned(),
            ),
            required_mode: DaemonMode::ApplyLowRisk,
            required_safety_class: SafetyClass::ReversibleLowRisk,
            manual_only_reason: None,
        };

        let rendered = render_candidate_suggestion(&suggestion);

        assert!(rendered.contains("candidate=\"candidate with space\""));
        assert!(rendered.contains("reason=\"scheduler \\\"pressure\\\"\\nnext\""));
        assert!(rendered.contains("dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile /tmp/profile \\\"quoted\\\".toml --dry-run\""));
        assert!(rendered.contains("manual_apply_command=\"stutter apply-profile --tree-pid 1234 --profile /tmp/profile \\\"quoted\\\".toml\""));
    }

    fn temp_candidate_plan_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stutter-candidate-plan-{name}-{}",
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn generate_profile_candidates_for_observation_without_target_pid_returns_no_candidates() {
        let profiles = vec![Profile {
            name: "fixture-game-helper".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }];

        let observation = crate::autotune::observation::AutotuneObservation {
            target_root_pid: None,
            active_tasks: vec![crate::autotune::observation::ActiveTaskSnapshot {
                tid: 1234,
                process_pid: 1234,
                comm: "game-main".to_owned(),
                class: TaskClass::Game,
                process_starttime_ticks: Some(10),
                task_starttime_ticks: Some(1234),
                cgroup_path: Some("/user.slice/fixture.scope".to_owned()),
            }],
            ..crate::autotune::observation::AutotuneObservation::default()
        };

        let plan = generate_profile_candidate_plan_for_observation(&profiles, &observation);

        assert!(plan.optimization_candidates.is_empty());
        assert!(plan.recovery_fallback.is_none());
        assert!(plan.rejected.is_empty());
    }

    fn eligible_record(name: &str, affected_tasks: usize) -> CandidateDryRunRecord {
        CandidateDryRunRecord {
            candidate_name: name.to_owned(),
            affected_tasks,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: true,
            reason: None,
        }
    }

    #[derive(Default)]
    struct FakeDryRunner {
        dry_run_calls: usize,
        apply_calls: usize,
    }

    impl CandidateDryRunner for FakeDryRunner {
        fn dry_run(&mut self, candidate: &CandidateAction) -> CandidateDryRunRecord {
            self.dry_run_calls += 1;
            eligible_record(candidate.profile_name(), 31)
        }
    }

    #[test]
    fn suggest_mode_emits_candidates_but_never_calls_apply() {
        let candidates = vec![CandidateAction::cpu_affinity_profile(
            profile("game-main-suggested"),
            1234,
        )];
        let mut runner = FakeDryRunner::default();

        let records = dry_run_candidates_with_runner(&candidates, &mut runner);
        let suggestions = suggestions_from_dry_run_records(
            &records,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert_eq!(runner.dry_run_calls, 1);
        assert_eq!(runner.apply_calls, 0);
        assert_eq!(suggestions.len(), 1);

        let rendered = render_candidate_suggestion(&suggestions[0]);
        assert!(rendered.contains("candidate=game-main-suggested"));
        assert!(rendered.contains("note=\"suggest mode did not apply this change\""));
        assert!(rendered.contains("dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile <generated-or-existing-profile> --dry-run\""));
    }

    #[test]
    fn profile_with_zero_affected_tasks_is_rejected() {
        let record = CandidateDryRunRecord {
            candidate_name: "zero-task-profile".to_owned(),
            affected_tasks: 0,
            warnings: Vec::new(),
            safety_class: SafetyClass::ReversibleLowRisk,
            eligible: false,
            reason: Some("dry-run matched zero affected tasks".to_owned()),
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert!(suggestion.is_none());
        assert!(!record.eligible);
        assert_eq!(
            record.reason.as_deref(),
            Some("dry-run matched zero affected tasks")
        );
    }

    #[test]
    fn profile_dry_run_warning_is_preserved() {
        let state = ActionState {
            applied: false,
            affected_tasks: 31,
            checked_tasks: 31,
            pending_changes: 31,
            warnings: vec![ActionWarning {
                message: "restore file already exists at /tmp/stutter-restore.json; new affinity records will be merged".to_owned(),
            }],
        };

        let record = dry_run_record_from_action_state(
            "warned-profile".to_owned(),
            SafetyClass::ReversibleLowRisk,
            state,
        );

        assert!(record.eligible);
        assert_eq!(record.affected_tasks, 31);
        assert_eq!(record.warnings.len(), 1);
        assert!(
            record.warnings[0]
                .message
                .contains("restore file already exists")
        );
    }

    #[test]
    fn high_risk_candidates_are_blocked() {
        let record = CandidateDryRunRecord {
            candidate_name: "high-risk-profile".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::HighRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert!(suggestion.is_none());
    }

    #[test]
    fn high_risk_candidates_are_allowed_when_policy_allows_high_risk() {
        let record = CandidateDryRunRecord {
            candidate_name: "high-risk-profile".to_owned(),
            affected_tasks: 31,
            warnings: Vec::new(),
            safety_class: SafetyClass::HighRisk,
            eligible: true,
            reason: None,
        };

        let suggestion = suggestion_from_dry_run_record(
            &record,
            1234,
            None,
            SafetyClass::HighRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );

        assert!(suggestion.is_some());
        assert_eq!(suggestion.unwrap().safety, SafetyClass::HighRisk);
    }

    #[test]
    fn dry_run_candidate_records_failure_as_ineligible() {
        let candidate = CandidateAction::cpu_affinity_profile(profile("bad-tree"), 0);

        let record = dry_run_candidate(&candidate);

        assert_eq!(record.candidate_name, "bad-tree");
        assert_eq!(record.affected_tasks, 0);
        assert_eq!(record.safety_class, SafetyClass::ReversibleLowRisk);
        assert!(!record.eligible);
        assert!(
            record
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("dry-run failed")
        );
        assert!(
            record
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("tree pid must be greater than zero")
        );
    }

    #[test]
    fn dry_run_candidates_preserves_candidate_order() {
        let candidates = vec![
            CandidateAction::cpu_affinity_profile(profile("first"), 0),
            CandidateAction::cpu_affinity_profile(profile("second"), 0),
        ];

        let records = dry_run_candidates(&candidates);

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].candidate_name, "first");
        assert_eq!(records[1].candidate_name, "second");
    }

    #[test]
    fn candidate_helpers_return_stable_metadata() {
        let candidate = CandidateAction::cpu_affinity_profile(profile("game-main"), 1234);

        assert_eq!(candidate.candidate_name(), "game-main");
        assert_eq!(candidate.target_root_pid(), Some(1234));
        assert_eq!(candidate.action_kind(), "cpu_affinity_profile");
        assert_eq!(candidate.safety_class(), SafetyClass::ReversibleLowRisk);
        assert_eq!(
            candidate.descriptor().effect_scope,
            ActionEffectScope::LocalProcessTree
        );
        assert_eq!(
            candidate.conflict_group(),
            ActionConflictGroup::CpuPlacement
        );
    }

    #[test]
    fn generic_candidate_variant_reports_descriptor_scope_and_objective() {
        let candidate = CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "nice-root-1234-to-5".to_owned(),
                action: crate::actions::nice::NiceAction {
                    targets: vec![crate::actions::TaskIdentity {
                        tid: 1234,
                        process_pid: Some(1234),
                        comm: None,
                        starttime_ticks: None,
                    }],
                    nice: 5,
                    policy: crate::actions::nice::NicePolicy::default(),
                },
                target_root_pid: Some(1234),
                evidence: vec![CandidateEvidence::new("situation", "CompileCpuBound", 0.8)],
                objective: ObjectiveKind::DesktopInteractivity,
            },
        };

        assert_eq!(candidate.candidate_name(), "nice-root-1234-to-5");
        assert_eq!(candidate.action_kind(), "nice");
        assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
        assert_eq!(
            candidate.effect_scope(),
            ActionEffectScope::LocalProcessTree
        );
        assert_eq!(candidate.target_root_pid(), Some(1234));
        assert_eq!(candidate.conflict_group(), ActionConflictGroup::CpuPriority);
        assert_eq!(candidate.objective(), ObjectiveKind::DesktopInteractivity);
    }

    #[test]
    fn profile_with_nice_or_ionice_is_medium_risk_candidate() {
        let candidate = CandidateAction::cpu_affinity_profile(
            Profile {
                name: "background-demotion".to_owned(),
                rules: vec![ProfileRule {
                    affinity: None,
                    nice: Some(10),
                    ionice: Some(crate::actions::ioprio::IoPrioValue::idle()),
                    match_class: vec![TaskClass::Indexer],
                    match_comm: Vec::new(),
                }],
            },
            1234,
        );

        assert_eq!(candidate.safety_class(), SafetyClass::ReversibleMediumRisk);
    }

    fn dry_run_record(safety_class: SafetyClass) -> CandidateDryRunRecord {
        CandidateDryRunRecord {
            candidate_name: "game-main".to_owned(),
            affected_tasks: 4,
            warnings: Vec::new(),
            safety_class,
            eligible: true,
            reason: None,
        }
    }

    #[test]
    fn low_risk_suggestion_renders_policy_aware_commands() {
        let suggestion = suggestion_from_dry_run_record(
            &dry_run_record(SafetyClass::ReversibleLowRisk),
            1234,
            Some(Path::new("profiles.toml")),
            SafetyClass::ReversibleLowRisk,
            "scheduler pressure detected",
        )
        .unwrap();

        assert_eq!(suggestion.required_mode, DaemonMode::ApplyLowRisk);
        assert_eq!(
            suggestion.required_safety_class,
            SafetyClass::ReversibleLowRisk
        );
        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml --dry-run")
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml")
        );

        let rendered = render_candidate_suggestion(&suggestion);
        assert!(rendered.contains("suggest mode did not apply this change"));
        assert!(rendered.contains("required_mode=apply-low-risk"));
        assert!(rendered.contains("required_safety_class=ReversibleLowRisk"));
        assert!(rendered.contains("rollback=\"stutter restore\""));
        assert!(rendered.contains(
            "dry_run_command=\"stutter apply-profile --tree-pid 1234 --profile profiles.toml --dry-run\""
        ));
        assert!(rendered.contains(
            "manual_apply_command=\"stutter apply-profile --tree-pid 1234 --profile profiles.toml\""
        ));
    }

    #[test]
    fn medium_risk_suggestion_requires_medium_mode_and_flag() {
        let suggestion = suggestion_from_dry_run_record(
            &dry_run_record(SafetyClass::ReversibleMediumRisk),
            1234,
            Some(Path::new("profiles.toml")),
            SafetyClass::ReversibleMediumRisk,
            "priority profile may help",
        )
        .unwrap();

        assert_eq!(suggestion.required_mode, DaemonMode::ApplyMediumRisk);
        assert_eq!(
            suggestion.required_safety_class,
            SafetyClass::ReversibleMediumRisk
        );
        assert_eq!(
            suggestion.manual_apply_command.as_deref(),
            Some(
                "stutter apply-profile --tree-pid 1234 --profile profiles.toml --allow-medium-risk"
            )
        );

        let rendered = render_candidate_suggestion(&suggestion);
        assert!(rendered.contains("required_mode=apply-medium-risk"));
        assert!(rendered.contains("required_safety_class=ReversibleMediumRisk"));
        assert!(rendered.contains("--allow-medium-risk"));
    }

    #[test]
    fn high_risk_suggestion_suppresses_manual_apply_command() {
        let suggestion = suggestion_from_dry_run_record(
            &dry_run_record(SafetyClass::HighRisk),
            1234,
            Some(Path::new("profiles.toml")),
            SafetyClass::HighRisk,
            "high risk candidate",
        )
        .unwrap();

        assert_eq!(suggestion.required_mode, DaemonMode::ApplyHighRisk);
        assert_eq!(suggestion.required_safety_class, SafetyClass::HighRisk);
        assert_eq!(
            suggestion.dry_run_command.as_deref(),
            Some("stutter apply-profile --tree-pid 1234 --profile profiles.toml --dry-run")
        );
        assert_eq!(suggestion.manual_apply_command, None);

        let rendered = render_candidate_suggestion(&suggestion);
        assert!(rendered.contains("required_mode=apply-high-risk"));
        assert!(rendered.contains("manual_apply_command=none"));
        assert!(!rendered.contains("manual_apply_command=\"stutter apply-profile"));
    }
}
