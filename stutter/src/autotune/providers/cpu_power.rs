use crate::{
    actions::cpu_power::CpuPowerAction,
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, CpuPowerActionPlan},
        objective::ObjectiveKind,
        providers::{
            CandidateProposal, CandidateProvider, CandidateProviderInput,
            signal_quality_confidence_weight,
        },
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
    pub battery_present: bool,
    pub battery_discharging: Option<bool>,
    pub cpu_power_on_battery_allowed: bool,
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
                        "policy={} related_cpus={:?} governor={:?} epp={:?} thermal_headroom={} ac_power={:?} battery_present={} battery_discharging={:?} battery_override={} limited_cpu={:?}",
                        structured_evidence.policy,
                        structured_evidence.related_cpus,
                        structured_evidence.current_governor,
                        structured_evidence.current_epp,
                        structured_evidence.thermal_headroom,
                        structured_evidence.ac_power,
                        structured_evidence.battery_present,
                        structured_evidence.battery_discharging,
                        structured_evidence.cpu_power_on_battery_allowed,
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
    let power_source = &input.system_context.inventory.power_source;
    if power_source.on_battery_or_discharging() && !input.daemon_policy.allow_cpu_power_on_battery {
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
    if !thermal_headroom {
        return None;
    }

    Some(CpuPowerCandidateEvidence {
        policy: policy.policy.clone(),
        related_cpus,
        current_governor: policy.scaling_governor.clone(),
        current_epp: policy.energy_performance_preference.clone(),
        thermal_headroom,
        ac_power: power_source.ac_power_for_evidence(),
        battery_present: power_source.battery_present,
        battery_discharging: power_source.battery_discharging,
        cpu_power_on_battery_allowed: input.daemon_policy.allow_cpu_power_on_battery,
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
        evidence.power_source_evidence_present(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count() as f32
        / 7.0;

    let signal_weight = signal_quality_confidence_weight(
        input.observation.objective_signals.signal_quality.cpu_power,
    );

    (input.observation.situation.confidence * completeness * signal_weight).clamp(0.0, 1.0)
}

impl CpuPowerCandidateEvidence {
    fn power_source_evidence_present(&self) -> bool {
        self.ac_power.is_some() || self.battery_discharging.is_some() || !self.battery_present
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
        system_inventory::{CpuPolicyInventory, PowerSourceSnapshot, SystemInventory},
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
                power_source: Default::default(),
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
                power_source: PowerSourceSnapshot {
                    ac_online: Some(true),
                    battery_present: false,
                    battery_discharging: None,
                },
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
                power_source: PowerSourceSnapshot {
                    ac_online: Some(true),
                    battery_present: false,
                    battery_discharging: None,
                },
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

    #[test]
    fn cpu_power_provider_includes_ac_power_evidence_when_ac_online() {
        let policy = policy();
        let proposals = proposals_for_power_source(
            PowerSourceSnapshot {
                ac_online: Some(true),
                battery_present: true,
                battery_discharging: Some(false),
            },
            Some(false),
            &policy,
        );

        assert_eq!(proposals.len(), 1);
        let CandidateAction::CpuPower { plan } = &proposals[0].candidate else {
            panic!("expected cpu power candidate");
        };
        assert!(plan.evidence[0].value.contains("ac_power=Some(true)"));
        assert!(
            plan.evidence[0]
                .value
                .contains("battery_discharging=Some(false)")
        );
    }

    #[test]
    fn cpu_power_provider_blocks_battery_discharging_by_default() {
        let policy = policy();
        let proposals = proposals_for_power_source(
            PowerSourceSnapshot {
                ac_online: Some(false),
                battery_present: true,
                battery_discharging: Some(true),
            },
            Some(false),
            &policy,
        );

        assert!(proposals.is_empty());
    }

    #[test]
    fn cpu_power_provider_allows_battery_discharging_with_explicit_policy() {
        let policy = policy_with_cpu_power_on_battery();
        let proposals = proposals_for_power_source(
            PowerSourceSnapshot {
                ac_online: Some(false),
                battery_present: true,
                battery_discharging: Some(true),
            },
            Some(false),
            &policy,
        );

        assert_eq!(proposals.len(), 1);
        let CandidateAction::CpuPower { plan } = &proposals[0].candidate else {
            panic!("expected cpu power candidate");
        };
        assert!(plan.evidence[0].value.contains("battery_override=true"));
    }

    #[test]
    fn cpu_power_provider_allows_no_battery_desktop_without_ac_supply() {
        let policy = policy();
        let proposals = proposals_for_power_source(
            PowerSourceSnapshot {
                ac_online: None,
                battery_present: false,
                battery_discharging: None,
            },
            Some(false),
            &policy,
        );

        assert_eq!(proposals.len(), 1);
        let CandidateAction::CpuPower { plan } = &proposals[0].candidate else {
            panic!("expected cpu power candidate");
        };
        assert!(plan.evidence[0].value.contains("ac_power=Some(true)"));
        assert!(plan.evidence[0].value.contains("battery_present=false"));
    }

    #[test]
    fn cpu_power_provider_blocks_thermal_degraded() {
        let policy = policy();
        let proposals = proposals_for_power_source(
            PowerSourceSnapshot {
                ac_online: Some(true),
                battery_present: false,
                battery_discharging: None,
            },
            Some(true),
            &policy,
        );

        assert!(proposals.is_empty());
    }

    #[test]
    fn cpu_power_provider_rejects_already_performance_policy() {
        let provider = CpuPowerProvider;
        let mut observation = cpu_power_limited_observation(Some(false));
        observation.objective_signals.cpu_power_limited_cpu = Some(9);
        let policy = policy();
        let system_context = system_context_with_cpu_power(
            PowerSourceSnapshot {
                ac_online: Some(true),
                battery_present: false,
                battery_discharging: None,
            },
            Some("performance".to_owned()),
            Some("performance".to_owned()),
        );

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

    fn proposals_for_power_source(
        power_source: PowerSourceSnapshot,
        thermal_degraded: Option<bool>,
        policy: &crate::daemon::DaemonPolicy,
    ) -> Vec<CandidateProposal> {
        let provider = CpuPowerProvider;
        let observation = cpu_power_limited_observation(thermal_degraded);
        let system_context = system_context_with_cpu_power(
            power_source,
            Some("powersave".to_owned()),
            Some("balance_power".to_owned()),
        );

        provider.propose(&CandidateProviderInput {
            observation: &observation,
            daemon_policy: policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            system_context: &system_context,
            controller_state: &ControllerRuntimeState::default(),
            profiles: &[],
        })
    }

    fn cpu_power_limited_observation(thermal_degraded: Option<bool>) -> AutotuneObservation {
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
        observation.objective_signals.thermal_degraded = thermal_degraded;
        observation
    }

    fn system_context_with_cpu_power(
        power_source: PowerSourceSnapshot,
        scaling_governor: Option<String>,
        energy_performance_preference: Option<String>,
    ) -> SystemContextSnapshot {
        SystemContextSnapshot {
            capabilities: DaemonCapabilities::default(),
            health: SystemHealthSnapshot::default(),
            inventory: SystemInventory {
                cpu_policies: vec![CpuPolicyInventory {
                    policy: "policy9".to_owned(),
                    path: PathBuf::from("/fake/sys/devices/system/cpu/cpufreq/policy9"),
                    scaling_governor,
                    available_governors: Some("powersave performance".to_owned()),
                    energy_performance_preference,
                    energy_performance_available_preferences: Some(
                        "balance_power performance".to_owned(),
                    ),
                    related_cpus: Some("9".to_owned()),
                }],
                drm_devices: Vec::new(),
                irq_default_smp_affinity: None,
                irq_lines: Vec::new(),
                power_source,
                sched_ext_available: false,
                vm_knobs: Default::default(),
                inventory_hash: "fake-cpu-power-source-inventory".to_owned(),
            },
            active_config: Default::default(),
            sampled_at_unix_nanos: 10,
        }
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

    fn policy_with_cpu_power_on_battery() -> crate::daemon::DaemonPolicy {
        let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        config.autotune.allow_cpu_power_on_battery = true;
        build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        })
    }
}
