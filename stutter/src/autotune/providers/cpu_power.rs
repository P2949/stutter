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

        let inventory = &input.system_context.inventory;
        let Some(policy) = inventory.cpu_policies.first() else {
            return Vec::new();
        };
        let cpus = parse_related_cpus(policy.related_cpus.as_deref());
        if cpus.is_empty() {
            return Vec::new();
        }
        let structured_evidence = CpuPowerCandidateEvidence {
            policy: policy.policy.clone(),
            related_cpus: cpus.clone(),
            current_governor: policy.scaling_governor.clone(),
            current_epp: policy.energy_performance_preference.clone(),
            thermal_headroom: input.system_health.ok_for_apply,
            ac_power: None,
        };
        let action = CpuPowerAction {
            sysfs_root: std::path::PathBuf::from("/sys"),
            cpus,
            scaling_governor: Some("performance".to_owned()),
            energy_performance_preference: Some("performance".to_owned()),
        };
        let objective = match input.observation.primary_situation {
            SituationKind::CompileCpuBound => {
                ObjectiveKind::CompileThroughputWithForegroundProtection
            }
            _ => ObjectiveKind::GameRunnableLatency,
        };
        let candidate = CandidateAction::CpuPower {
            plan: CpuPowerActionPlan {
                name: format!("cpu-power-policy-{}-performance", policy.policy),
                action,
                evidence: vec![CandidateEvidence::new(
                    "inventory",
                    format!(
                        "policy={} related_cpus={:?} governor={:?} epp={:?}",
                        structured_evidence.policy,
                        structured_evidence.related_cpus,
                        structured_evidence.current_governor,
                        structured_evidence.current_epp
                    ),
                    0.7,
                )],
                objective,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: Vec::new(),
            objective,
            rank_hint: 80,
        }]
    }
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
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "fake-cpu-inventory".to_owned(),
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
