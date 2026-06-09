use crate::{
    actions::SafetyClass,
    autotune::{
        conflicts::ActionConflictGroup,
        controller::ControllerRuntimeState,
        objective::{ObjectiveKind, ObjectiveSignalQuality},
        observation::AutotuneObservation,
        planning::candidate::{CandidateAction, CandidateFamily},
        system_context::SystemContextSnapshot,
    },
    daemon::{DaemonPolicy, capabilities::DaemonCapabilities, health::SystemHealthSnapshot},
    daemon_policy::{DaemonMode, RollbackRequirement},
    profiles::Profile,
};

pub mod cgroup;
pub mod cpu_affinity;
pub mod cpu_power;
pub mod gpu_power;
pub mod ioprio;
pub mod irq_affinity;
pub mod nice;
pub mod uclamp;
pub mod vm_knob;

#[derive(Clone, Debug)]
pub struct CandidateProposal {
    pub candidate: CandidateAction,
    pub provider: &'static str,
    pub confidence: f32,
    pub deny_reasons: Vec<String>,
    pub objective: ObjectiveKind,
    pub rank_hint: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProviderConfidenceCalibration {
    pub family: String,
    pub min_required_signals: Vec<String>,
    pub direct_signal_weight: f32,
    pub inferred_signal_weight: f32,
    pub max_without_direct_signal: f32,
    pub max_without_active_config: f32,
}

impl ProviderConfidenceCalibration {
    pub fn for_family(family: &str) -> Self {
        match family {
            "cpu_affinity_profile" => Self::new(family, &["active_tasks"], 1.0, 0.85, 0.70, 0.49),
            "nice" | "ionice" | "uclamp" | "cgroup_placement" => {
                Self::new(family, &["active_tasks"], 1.0, 0.85, 0.70, 0.49)
            }
            "irq_affinity" => Self::new(
                family,
                &["stable_irq_identity", "current_mask"],
                1.0,
                0.80,
                0.49,
                0.60,
            ),
            "gpu_power" => Self::new(family, &["focused_gpu_identity"], 1.0, 0.75, 0.49, 0.60),
            "cpu_power" => Self::new(family, &["power_source"], 1.0, 0.75, 0.60, 0.60),
            "vm_knob" => Self::new(
                family,
                &["memory_or_writeback_signal"],
                1.0,
                0.75,
                0.60,
                0.60,
            ),
            _ => Self::new(family, &[], 1.0, 1.0, 1.0, 1.0),
        }
    }

    fn new(
        family: &str,
        min_required_signals: &[&str],
        direct_signal_weight: f32,
        inferred_signal_weight: f32,
        max_without_direct_signal: f32,
        max_without_active_config: f32,
    ) -> Self {
        Self {
            family: family.to_owned(),
            min_required_signals: min_required_signals
                .iter()
                .map(|signal| signal.to_string())
                .collect(),
            direct_signal_weight,
            inferred_signal_weight,
            max_without_direct_signal,
            max_without_active_config,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CandidateProviderMetadata {
    pub family: CandidateFamily,
    pub description: &'static str,
    pub safety_class: SafetyClass,
    pub required_mode: DaemonMode,
    pub rollback_requirement: RollbackRequirement,
    pub capability_requirements: &'static [&'static str],
    pub conflict_group: ActionConflictGroup,
    pub cooldown_key: &'static str,
    pub objective: ObjectiveKind,
    pub policy_coverage: &'static [&'static str],
}

pub trait CandidateProvider {
    fn family(&self) -> CandidateFamily;
    fn metadata(&self) -> CandidateProviderMetadata {
        provider_metadata_for_family(self.family())
    }
    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal>;
}

pub struct CandidateProviderInput<'a> {
    pub observation: &'a AutotuneObservation,
    pub daemon_policy: &'a DaemonPolicy,
    pub capabilities: &'a DaemonCapabilities,
    pub system_health: &'a SystemHealthSnapshot,
    pub system_context: &'a SystemContextSnapshot,
    pub controller_state: &'a ControllerRuntimeState,
    pub profiles: &'a [Profile],
}

pub(crate) fn signal_quality_confidence_weight(quality: ObjectiveSignalQuality) -> f32 {
    match quality {
        ObjectiveSignalQuality::Direct => 1.0,
        ObjectiveSignalQuality::Derived => 1.0,
        ObjectiveSignalQuality::Approximate => 0.85,
        ObjectiveSignalQuality::Missing => 0.50,
    }
}

#[derive(Default)]
pub struct CandidateProviderRegistry {
    providers: Vec<Box<dyn CandidateProvider>>,
}

impl CandidateProviderRegistry {
    pub fn default_for_policy(policy: &DaemonPolicy) -> Self {
        let mut registry = Self::default();
        registry.register(Box::new(cpu_affinity::CpuAffinityProvider));

        if policy.mode == crate::daemon::policy::DaemonMode::Suggest
            || policy.max_safety_class >= crate::actions::SafetyClass::ReversibleMediumRisk
        {
            registry.register(Box::new(nice::NiceProvider));
            registry.register(Box::new(ioprio::IoPrioProvider));
            registry.register(Box::new(uclamp::UclampProvider));
            registry.register(Box::new(cgroup::CgroupProvider));
        }

        let system_wide_suggestions = policy.mode == crate::daemon::policy::DaemonMode::Suggest
            && policy.allow_system_wide_suggestions;
        let medium_risk_apply = policy.mode == crate::daemon::policy::DaemonMode::ApplyMediumRisk
            && policy.allow_medium_risk_apply;

        if system_wide_suggestions || medium_risk_apply {
            registry.register(Box::new(irq_affinity::IrqAffinityProvider));
            registry.register(Box::new(cpu_power::CpuPowerProvider));
            registry.register(Box::new(gpu_power::GpuPowerProvider));
            registry.register(Box::new(vm_knob::VmKnobProvider));
        }

        registry
    }

    pub fn register(&mut self, provider: Box<dyn CandidateProvider>) {
        self.providers.push(provider);
    }

    pub fn families(&self) -> Vec<&'static str> {
        self.providers
            .iter()
            .map(|provider| provider.family())
            .collect()
    }

    pub fn metadata(&self) -> Vec<CandidateProviderMetadata> {
        self.providers
            .iter()
            .map(|provider| provider.metadata())
            .collect()
    }

    pub fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        self.providers
            .iter()
            .filter(|provider| family_enabled(input.daemon_policy, provider.family()))
            .flat_map(|provider| provider.propose(input))
            .map(|proposal| calibrate_provider_proposal(proposal, input))
            .collect()
    }
}

pub(crate) fn calibrate_provider_proposal(
    mut proposal: CandidateProposal,
    input: &CandidateProviderInput<'_>,
) -> CandidateProposal {
    let calibration = ProviderConfidenceCalibration::for_family(proposal.provider);
    let mut cap = 1.0_f32;

    match proposal.provider {
        "cpu_affinity_profile" | "nice" | "ionice" | "uclamp" | "cgroup_placement"
            if input.observation.active_tasks.is_empty() =>
        {
            cap = cap.min(calibration.max_without_active_config);
        }
        "irq_affinity"
            if !proposal_has_evidence_token(&proposal, "stable_identity=true")
                || !proposal_has_evidence_token(&proposal, "current_mask=") =>
        {
            cap = cap.min(calibration.max_without_direct_signal);
        }
        "gpu_power" => {
            if input.system_context.inventory.drm_devices.len() > 1
                && !proposal_has_evidence_token(&proposal, "active_for_focus=true")
            {
                cap = cap.min(calibration.max_without_direct_signal);
            }
            if input.observation.active_config_snapshot.is_none() {
                cap = cap.min(calibration.max_without_active_config);
            }
        }
        "cpu_power" => {
            let power_source = &input.system_context.inventory.power_source;
            let has_power_source = power_source.ac_online.is_some()
                || power_source.battery_discharging.is_some()
                || !power_source.battery_present;
            if !has_power_source {
                cap = cap.min(calibration.max_without_direct_signal);
            }
        }
        "vm_knob" => {
            let signals = &input.observation.objective_signals;
            let has_direct_signal = signals.memory_pressure_some_avg10_percent.is_some()
                || signals.swap_activity_events.is_some()
                || signals.dirty_writeback_events.is_some();
            if !has_direct_signal {
                cap = cap.min(calibration.max_without_direct_signal);
            }
        }
        _ => {}
    }

    proposal.confidence = proposal.confidence.min(cap).clamp(0.0, 1.0);
    proposal
}

fn proposal_has_evidence_token(proposal: &CandidateProposal, token: &str) -> bool {
    proposal
        .candidate
        .evidence()
        .iter()
        .any(|evidence| evidence.value.contains(token) || evidence.signal.contains(token))
}

fn provider_metadata_for_family(family: CandidateFamily) -> CandidateProviderMetadata {
    match family {
        "cpu_affinity_profile" => CandidateProviderMetadata {
            family,
            description: "Topology-aware CPU affinity profile suggestions for a focused workload.",
            safety_class: SafetyClass::ReversibleLowRisk,
            required_mode: DaemonMode::Suggest,
            rollback_requirement: RollbackRequirement::RequiredBeforeApply,
            capability_requirements: &["sched_setaffinity", "procfs"],
            conflict_group: ActionConflictGroup::CpuPlacement,
            cooldown_key: "cpu_affinity_profile",
            objective: ObjectiveKind::StutterScore,
            policy_coverage: &[
                "daemon_mode",
                "safety_class",
                "rollback_required",
                "target_scope",
                "protected_tasks",
                "cooldown",
            ],
        },
        "nice" => CandidateProviderMetadata {
            family,
            description: "Per-task nice adjustments for foreground or background CPU pressure.",
            safety_class: SafetyClass::ReversibleMediumRisk,
            required_mode: DaemonMode::ApplyMediumRisk,
            rollback_requirement: RollbackRequirement::RequiredBeforeApply,
            capability_requirements: &["setpriority", "procfs"],
            conflict_group: ActionConflictGroup::CpuPriority,
            cooldown_key: "nice",
            objective: ObjectiveKind::DesktopInteractivity,
            policy_coverage: &[
                "daemon_mode",
                "safety_class",
                "capability",
                "target_scope",
                "protected_tasks",
                "cooldown",
            ],
        },
        "ionice" => CandidateProviderMetadata {
            family,
            description: "Per-task I/O priority adjustments for block I/O pressure.",
            safety_class: SafetyClass::ReversibleMediumRisk,
            required_mode: DaemonMode::ApplyMediumRisk,
            rollback_requirement: RollbackRequirement::RequiredBeforeApply,
            capability_requirements: &["ioprio_set", "procfs"],
            conflict_group: ActionConflictGroup::IoPriority,
            cooldown_key: "ionice",
            objective: ObjectiveKind::DesktopInteractivity,
            policy_coverage: &[
                "daemon_mode",
                "safety_class",
                "capability",
                "evidence_quality",
                "target_scope",
                "protected_tasks",
                "cooldown",
            ],
        },
        "uclamp" => CandidateProviderMetadata {
            family,
            description: "Per-task utilization clamp adjustments for scheduler latency.",
            safety_class: SafetyClass::ReversibleMediumRisk,
            required_mode: DaemonMode::ApplyMediumRisk,
            rollback_requirement: RollbackRequirement::RequiredBeforeApply,
            capability_requirements: &["sched_setattr", "procfs"],
            conflict_group: ActionConflictGroup::CpuPriority,
            cooldown_key: "uclamp",
            objective: ObjectiveKind::GameRunnableLatency,
            policy_coverage: &[
                "daemon_mode",
                "safety_class",
                "capability",
                "system_health",
                "target_scope",
                "protected_tasks",
                "cooldown",
            ],
        },
        "cgroup_placement" => CandidateProviderMetadata {
            family,
            description: "Move eligible workload tasks into configured cgroup targets.",
            safety_class: SafetyClass::ReversibleMediumRisk,
            required_mode: DaemonMode::ApplyMediumRisk,
            rollback_requirement: RollbackRequirement::RequiredBeforeApply,
            capability_requirements: &["cgroup_v2", "cgroup.procs"],
            conflict_group: ActionConflictGroup::CgroupPlacement,
            cooldown_key: "cgroup_placement",
            objective: ObjectiveKind::DesktopInteractivity,
            policy_coverage: &[
                "daemon_mode",
                "safety_class",
                "configured_targets",
                "target_scope",
                "protected_tasks",
                "cooldown",
            ],
        },
        "irq_affinity" => CandidateProviderMetadata {
            family,
            description: "Suggest IRQ affinity placement when stable IRQ pressure evidence exists.",
            safety_class: SafetyClass::HighRisk,
            required_mode: DaemonMode::Suggest,
            rollback_requirement: RollbackRequirement::RequiredBeforeApply,
            capability_requirements: &["proc_irq", "irq_device_identity"],
            conflict_group: ActionConflictGroup::IrqPlacement,
            cooldown_key: "irq_affinity",
            objective: ObjectiveKind::GameRunnableLatency,
            policy_coverage: &[
                "suggest_only",
                "manual_only_high_risk",
                "system_wide_suggestions",
                "evidence_quality",
                "cooldown",
            ],
        },
        "cpu_power" => CandidateProviderMetadata {
            family,
            description: "Suggest CPU power policy changes under workload pressure and safe health.",
            safety_class: SafetyClass::HighRisk,
            required_mode: DaemonMode::Suggest,
            rollback_requirement: RollbackRequirement::RequiredBeforeApply,
            capability_requirements: &["sysfs_cpu_cpufreq", "power_supply"],
            conflict_group: ActionConflictGroup::CpuPower,
            cooldown_key: "cpu_power",
            objective: ObjectiveKind::GameRunnableLatency,
            policy_coverage: &[
                "suggest_only",
                "manual_only_high_risk",
                "system_wide_suggestions",
                "system_health",
                "battery_policy",
                "cooldown",
            ],
        },
        "gpu_power" => CandidateProviderMetadata {
            family,
            description: "Suggest focused GPU power policy changes for GPU-bound workloads.",
            safety_class: SafetyClass::HighRisk,
            required_mode: DaemonMode::Suggest,
            rollback_requirement: RollbackRequirement::RequiredBeforeApply,
            capability_requirements: &["sysfs_drm", "gpu_focus_identity"],
            conflict_group: ActionConflictGroup::GpuPower,
            cooldown_key: "gpu_power",
            objective: ObjectiveKind::GameFramePacing,
            policy_coverage: &[
                "suggest_only",
                "manual_only_high_risk",
                "system_wide_suggestions",
                "system_health",
                "focus_identity",
                "cooldown",
            ],
        },
        "vm_knob" => CandidateProviderMetadata {
            family,
            description: "Suggest VM/sysfs knob changes for memory or writeback pressure.",
            safety_class: SafetyClass::HighRisk,
            required_mode: DaemonMode::Suggest,
            rollback_requirement: RollbackRequirement::RequiredBeforeApply,
            capability_requirements: &["proc_sys_vm", "memory_pressure_evidence"],
            conflict_group: ActionConflictGroup::VmMemory,
            cooldown_key: "vm_knob",
            objective: ObjectiveKind::DesktopInteractivity,
            policy_coverage: &[
                "suggest_only",
                "manual_only_high_risk",
                "system_wide_suggestions",
                "evidence_quality",
                "cooldown",
            ],
        },
        _ => CandidateProviderMetadata {
            family,
            description: "Candidate provider with explicit registration metadata.",
            safety_class: SafetyClass::ObserveOnly,
            required_mode: DaemonMode::Observe,
            rollback_requirement: RollbackRequirement::NotRequiredForDryRun,
            capability_requirements: &["unknown"],
            conflict_group: ActionConflictGroup::None,
            cooldown_key: "unknown",
            objective: ObjectiveKind::StutterScore,
            policy_coverage: &["provider_registered"],
        },
    }
}

fn family_enabled(policy: &DaemonPolicy, family: &str) -> bool {
    let enabled = policy.enabled_action_families.is_empty()
        || policy
            .enabled_action_families
            .iter()
            .any(|enabled| family_matches(family, enabled));
    let denied = policy
        .denied_action_families
        .iter()
        .any(|denied| family_matches(family, denied));

    enabled && !denied
}

fn family_matches(family: &str, configured: &str) -> bool {
    family == configured
        || family.strip_prefix(configured).is_some_and(|suffix| {
            matches!(
                suffix.as_bytes().first(),
                Some(b':') | Some(b'-') | Some(b'_')
            )
        })
}

#[cfg(test)]
mod tests;
