use crate::{
    actions::cpu_power::CpuPowerAction,
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, CpuPowerActionPlan},
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuPowerCandidateEvidence {
    pub policy: String,
    pub related_cpus: Vec<u32>,
    pub current_governor: Option<String>,
    pub current_epp: Option<String>,
    pub thermal_headroom: bool,
    pub ac_power: Option<bool>,
}

#[derive(Default)]
pub struct CpuPowerProvider;

impl CandidateProvider for CpuPowerProvider {
    fn family(&self) -> &'static str {
        "cpu_power"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !matches!(
            input.observation.primary_situation,
            SituationKind::GameCpuSchedulerPressure | SituationKind::CompileCpuBound
        ) || !input.system_health.ok_for_apply
        {
            return Vec::new();
        }

        let Some(structured_evidence) = cpu_power_evidence(input) else {
            return Vec::new();
        };

        let epp = input
            .system_context
            .inventory
            .cpu_policies
            .iter()
            .find(|policy| policy.policy == structured_evidence.policy)
            .and_then(|policy| policy.energy_performance_available_preferences.as_deref())
            .filter(|available| supports_token(Some(available), "performance"))
            .map(|_| "performance".to_owned());

        let action = CpuPowerAction {
            sysfs_root: std::path::PathBuf::from("/sys"),
            cpus: structured_evidence.related_cpus.clone(),
            scaling_governor: Some("performance".to_owned()),
            energy_performance_preference: epp,
        };
        let objective = match input.observation.primary_situation {
            SituationKind::CompileCpuBound => {
                ObjectiveKind::CompileThroughputWithForegroundProtection
            }
            _ => ObjectiveKind::GameRunnableLatency,
        };
        let confidence = cpu_power_confidence(input, &structured_evidence);
        let candidate = CandidateAction::CpuPower {
            plan: CpuPowerActionPlan {
                name: format!(
                    "cpu-power-policy-{}-performance",
                    structured_evidence.policy
                ),
                action,
                evidence: vec![CandidateEvidence::new(
                    "cpu_power_structured",
                    format!(
                        "policy={} related_cpus={:?} governor={:?} epp={:?} thermal_headroom={} ac_power={:?} limited_cpu={:?}",
                        structured_evidence.policy,
                        structured_evidence.related_cpus,
                        structured_evidence.current_governor,
                        structured_evidence.current_epp,
                        structured_evidence.thermal_headroom,
                        structured_evidence.ac_power,
                        input.observation.objective_signals.cpu_power_limited_cpu
                    ),
                    confidence,
                )],
                objective,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence,
            deny_reasons: Vec::new(),
            objective,
            rank_hint: 80,
        }]
    }
}

fn cpu_power_evidence(input: &CandidateProviderInput<'_>) -> Option<CpuPowerCandidateEvidence> {
    if input.observation.objective_signals.cpu_power_limited != Some(true) {
        return None;
    }

    let limited_cpu = input.observation.objective_signals.cpu_power_limited_cpu?;
    let policy = input
        .system_context
        .inventory
        .cpu_policies
        .iter()
        .find(|policy| parse_related_cpus(policy.related_cpus.as_deref()).contains(&limited_cpu))?;

    if !supports_token(policy.available_governors.as_deref(), "performance") {
        return None;
    }

    let related_cpus = parse_related_cpus(policy.related_cpus.as_deref());
    if related_cpus.is_empty() {
        return None;
    }

    let already_performance = policy.scaling_governor.as_deref() == Some("performance")
        && policy.energy_performance_preference.as_deref() == Some("performance");
    if already_performance {
        return None;
    }

    let thermal_headroom = input.observation.objective_signals.thermal_degraded != Some(true)
        && input.system_health.ok_for_apply;

    Some(CpuPowerCandidateEvidence {
        policy: policy.policy.clone(),
        related_cpus,
        current_governor: policy.scaling_governor.clone(),
        current_epp: policy.energy_performance_preference.clone(),
        thermal_headroom,
        ac_power: None,
    })
}

fn supports_token(value: Option<&str>, token: &str) -> bool {
    value
        .unwrap_or_default()
        .split_whitespace()
        .any(|part| part == token)
}

fn cpu_power_confidence(
    input: &CandidateProviderInput<'_>,
    evidence: &CpuPowerCandidateEvidence,
) -> f32 {
    let completeness = [
        true,
        !evidence.related_cpus.is_empty(),
        evidence.current_governor.is_some(),
        evidence.current_epp.is_some(),
        evidence.thermal_headroom,
        input
            .observation
            .objective_signals
            .cpu_power_limited_cpu
            .is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count() as f32
        / 6.0;

    (input.observation.situation.confidence * completeness).clamp(0.0, 1.0)
}

fn parse_related_cpus(related_cpus: Option<&str>) -> Vec<u32> {
    let mut cpus = related_cpus
        .unwrap_or_default()
        .split_whitespace()
        .flat_map(|part| part.split(','))
        .flat_map(parse_cpu_token)
        .collect::<Vec<_>>();
    cpus.sort_unstable();
    cpus.dedup();
    cpus
}

fn parse_cpu_token(token: &str) -> Vec<u32> {
    if let Some((start, end)) = token.split_once('-') {
        let Some(start) = start.parse::<u32>().ok() else {
            return Vec::new();
        };
        let Some(end) = end.parse::<u32>().ok() else {
            return Vec::new();
        };
        return (start..=end).collect();
    }

    token.parse::<u32>().ok().into_iter().collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        autotune::{
            controller::ControllerRuntimeState, observation::AutotuneObservation,
            system_context::SystemContextSnapshot,
        },
        daemon::{ActionSource, DaemonCapabilities, DaemonMode, SystemHealthSnapshot},
        daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
        focus::FocusGroupKind,
        system_inventory::{CpuPolicyInventory, SystemInventory},
    };

    #[test]
    fn cpu_power_provider_rejects_missing_cpu_power_limit_evidence() {
        let provider = CpuPowerProvider;
        let mut observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            primary_situation: SituationKind::CompileCpuBound,
            focus_kind: Some(FocusGroupKind::Compile),
            focus_confidence: 0.95,
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation.primary_situation = SituationKind::CompileCpuBound;

        let policy = policy();
        let system_context = SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: Vec::new(),
                drm_devices: Vec::new(),
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "empty-cpu-inventory".to_owned(),
            },
            active_config: Default::default(),
            sampled_at_unix_nanos: 10,
        };

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert!(proposals.is_empty());
    }

    #[test]
    fn cpu_power_provider_does_not_request_epp_without_available_epp_evidence() {
        let provider = CpuPowerProvider;
        let mut observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            primary_situation: SituationKind::CompileCpuBound,
            focus_kind: Some(FocusGroupKind::Compile),
            focus_confidence: 0.95,
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation.primary_situation = SituationKind::CompileCpuBound;
        observation.objective_signals.cpu_power_limited = Some(true);
        observation.objective_signals.cpu_power_limited_cpu = Some(9);

        let policy = policy();
        let system_context = SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: vec![CpuPolicyInventory {
                    policy: "policy9".to_owned(),
                    path: PathBuf::from("/fake/sys/devices/system/cpu/cpufreq/policy9"),
                    scaling_governor: Some("powersave".to_owned()),
                    available_governors: Some("powersave performance".to_owned()),
                    energy_performance_preference: None,
                    energy_performance_available_preferences: None,
                    related_cpus: Some("9".to_owned()),
                }],
                drm_devices: Vec::new(),
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "fake-cpu-no-epp-inventory".to_owned(),
            },
            active_config: Default::default(),
            sampled_at_unix_nanos: 10,
        };

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert_eq!(proposals.len(), 1);
        let CandidateAction::CpuPower { plan } = &proposals[0].candidate else {
            panic!("expected cpu power candidate");
        };
        assert_eq!(plan.action.cpus, vec![9]);
        assert_eq!(plan.action.scaling_governor.as_deref(), Some("performance"));
        assert_eq!(plan.action.energy_performance_preference, None);
    }

    #[test]
    fn cpu_power_provider_uses_system_context_inventory() {
        let provider = CpuPowerProvider;
        let mut observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(1234),
            primary_situation: SituationKind::CompileCpuBound,
            focus_kind: Some(FocusGroupKind::Compile),
            focus_confidence: 0.95,
            ..AutotuneObservation::default()
        };
        observation.refresh_situation_classification();
        observation.primary_situation = SituationKind::CompileCpuBound;

        let policy = policy();
        let system_context = SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: vec![CpuPolicyInventory {
                    policy: "policy9".to_owned(),
                    path: PathBuf::from("/fake/sys/devices/system/cpu/cpufreq/policy9"),
                    scaling_governor: Some("powersave".to_owned()),
                    available_governors: Some("powersave performance".to_owned()),
                    energy_performance_preference: Some("balance_power".to_owned()),
                    energy_performance_available_preferences: Some(
                        "balance_power performance".to_owned(),
                    ),
                    related_cpus: Some("9".to_owned()),
                }],
                drm_devices: Vec::new(),
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "fake-cpu-inventory".to_owned(),
            },
            active_config: Default::default(),
            sampled_at_unix_nanos: 10,
        };

        observation.objective_signals.cpu_power_limited = Some(true);
        observation.objective_signals.cpu_power_limited_cpu = Some(9);

        let proposals = provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        });

        assert_eq!(proposals.len(), 1);
        let CandidateAction::CpuPower { plan } = &proposals[0].candidate else {
            panic!("expected cpu power candidate");
        };
        assert_eq!(plan.name, "cpu-power-policy-policy9-performance");
        assert_eq!(plan.action.cpus, vec![9]);
    }

    fn policy() -> crate::daemon::DaemonPolicy {
        let config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }
}
