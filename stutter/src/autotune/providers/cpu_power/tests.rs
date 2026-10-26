use std::path::PathBuf;

use super::*;
use crate::{
    autotune::{
        controller::ControllerRuntimeState, observation::AutotuneObservation,
        system_context::SystemContextSnapshot,
    },
    daemon::{
        capabilities::DaemonCapabilities,
        health::SystemHealthSnapshot,
        policy::{ActionSource, DaemonMode},
    },
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

    assert!(proposals.is_empty());
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
    assert_eq!(plan.action.scaling_governor, None);
    assert_eq!(
        plan.action.energy_performance_preference.as_deref(),
        Some("performance")
    );
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

fn propose_epp(
    available_governors: &str,
    governor: &str,
    epp: &str,
    available_epp: &str,
) -> Vec<CandidateProposal> {
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        primary_situation: SituationKind::GameCpuSchedulerPressure,
        focus_kind: Some(FocusGroupKind::Game),
        focus_confidence: 0.95,
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation.primary_situation = SituationKind::GameCpuSchedulerPressure;
    observation.objective_signals.cpu_power_limited = Some(true);
    observation.objective_signals.cpu_power_limited_cpu = Some(3);
    let policy = policy();
    let system_context = SystemContextSnapshot {
        capabilities: DaemonCapabilities::default(),
        health: SystemHealthSnapshot::default(),
        inventory: SystemInventory {
            cpu_policies: vec![CpuPolicyInventory {
                policy: "policy3".to_owned(),
                path: PathBuf::from("/fake/sys/devices/system/cpu/cpufreq/policy3"),
                scaling_governor: Some(governor.to_owned()),
                available_governors: Some(available_governors.to_owned()),
                energy_performance_preference: Some(epp.to_owned()),
                energy_performance_available_preferences: Some(available_epp.to_owned()),
                related_cpus: Some("3".to_owned()),
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
            inventory_hash: "epp".to_owned(),
        },
        active_config: Default::default(),
        sampled_at_unix_nanos: 10,
    };
    CpuPowerProvider.propose(&CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    })
}

#[test]
fn epp_is_proposed_without_a_performance_governor_and_never_touches_the_governor() {
    // intel_pstate active mode offers only powersave; EPP-only autonomy must not need more.
    let proposals = propose_epp(
        "powersave",
        "powersave",
        "balance_performance",
        "default performance balance_performance balance_power power",
    );
    assert_eq!(proposals.len(), 1);
    let CandidateAction::CpuPower { plan } = &proposals[0].candidate else {
        panic!("expected cpu power candidate");
    };
    assert_eq!(
        plan.action.energy_performance_preference.as_deref(),
        Some("performance")
    );
    assert!(plan.action.scaling_governor.is_none());
}

#[test]
fn nothing_is_proposed_when_the_preference_already_has_the_desired_value() {
    // Regardless of the governor.
    for governor in ["powersave", "performance"] {
        let proposals = propose_epp(
            "powersave performance",
            governor,
            "performance",
            "default performance balance_performance",
        );
        assert!(proposals.is_empty(), "governor {governor}");
    }
}

#[test]
fn the_desired_token_must_be_available_for_the_policy() {
    assert!(
        propose_epp(
            "powersave",
            "powersave",
            "balance_performance",
            "default balance_performance balance_power power"
        )
        .is_empty()
    );
}
