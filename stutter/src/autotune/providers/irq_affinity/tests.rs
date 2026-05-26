use std::collections::BTreeMap;

use super::model::*;
use crate::{
    autotune::{
        controller::ControllerRuntimeState,
        observation::{
            ActiveAffinitySnapshot, ActiveConfigSnapshot, ActiveIrqSnapshot, ActiveTaskSnapshot,
            AutotuneObservation,
        },
        planning::candidate::CandidateAction,
        providers::{CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
        system_context::SystemContextSnapshot,
    },
    daemon::{
        capabilities::DaemonCapabilities,
        health::SystemHealthSnapshot,
        policy::{ActionSource, DaemonMode},
    },
    daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
    focus::FocusGroupKind,
    irq_inspect::IrqLine,
    process_tree::TaskClass,
    system_inventory::SystemInventory,
};

#[test]
fn irq_provider_selects_hot_irq_from_structured_telemetry() {
    let provider = IrqAffinityProvider;
    let mut observation = observation();
    observation.objective_signals.irq_overlap_count = Some(2);
    observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
    observation.objective_signals.irq_hot_irq = Some(146);
    observation.objective_signals.irq_hot_cpu = Some(2);
    observation.active_config_snapshot = Some(ActiveConfigSnapshot {
        irq: ActiveIrqSnapshot {
            per_irq: BTreeMap::from([(146, "4".to_owned())]),
        },
        ..ActiveConfigSnapshot::default()
    });

    let mut system_context = system_context();
    system_context.inventory.irq_default_smp_affinity = Some("f".to_owned());
    system_context.inventory.irq_lines = vec![IrqLine {
        irq: "146".to_owned(),
        counts_by_cpu: vec![10_000, 2, 20_000, 30_000],
        total: 60_002,
        kind: "PCI-MSI".to_owned(),
        name: "amdgpu".to_owned(),
        raw: "146: 10000 2 20000 30000 PCI-MSI amdgpu".to_owned(),
    }];

    let proposals = provider.propose(&CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy(),
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    });

    assert_eq!(proposals.len(), 1);
    let CandidateAction::IrqAffinity { plan } = &proposals[0].candidate else {
        panic!("expected irq affinity candidate");
    };
    assert_eq!(plan.action.irq, 146);
    assert_eq!(plan.action.device_hint, "amdgpu");
    assert_eq!(plan.action.smp_affinity, "2");
    assert!(proposals[0].confidence > 0.0);
}

#[test]
fn irq_provider_emits_no_proposal_when_structured_irq_evidence_is_missing() {
    let provider = IrqAffinityProvider;
    let observation = observation();
    let system_context = system_context();

    let proposals = provider.propose(&CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy(),
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    });

    assert!(proposals.is_empty());
}

#[test]
fn irq_provider_emits_no_proposal_when_suggested_cpu_mask_is_unrepresentable() {
    let provider = IrqAffinityProvider;
    let mut observation = observation();
    observation.objective_signals.irq_overlap_count = Some(1);
    observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
    observation.objective_signals.irq_hot_irq = Some(146);
    observation.objective_signals.irq_hot_cpu = Some(130);
    observation.active_config_snapshot = Some(ActiveConfigSnapshot {
        irq: ActiveIrqSnapshot {
            per_irq: BTreeMap::from([(146, "4".to_owned())]),
        },
        ..ActiveConfigSnapshot::default()
    });

    let mut system_context = system_context();
    system_context.inventory.irq_lines = vec![IrqLine {
        irq: "146".to_owned(),
        counts_by_cpu: Vec::new(),
        total: 60_002,
        kind: "PCI-MSI".to_owned(),
        name: "amdgpu".to_owned(),
        raw: "146: PCI-MSI amdgpu".to_owned(),
    }];

    let proposals = provider.propose(&CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy(),
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    });

    assert!(proposals.is_empty());
}

#[test]
fn irq_provider_avoids_audio_compositor_and_reserved_cpus() {
    let provider = IrqAffinityProvider;
    let mut observation = observation_with_irq_pressure();
    observation.active_tasks = vec![
        task(10, TaskClass::AudioRealtime),
        task(11, TaskClass::Compositor),
        task(12, TaskClass::KernelThread),
    ];
    observation.active_config_snapshot = Some(ActiveConfigSnapshot {
        affinity: ActiveAffinitySnapshot {
            per_tid: BTreeMap::from([
                (10, "1".to_owned()),
                (11, "2".to_owned()),
                (12, "3".to_owned()),
            ]),
        },
        irq: ActiveIrqSnapshot {
            per_irq: BTreeMap::from([(146, "4".to_owned())]),
        },
        ..ActiveConfigSnapshot::default()
    });

    let mut system_context = system_context_with_irq_line(vec![50, 1, 2, 3]);
    system_context.inventory.irq_default_smp_affinity = Some("f".to_owned());

    let proposals = provider.propose(&CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy(),
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    });

    assert_eq!(proposals.len(), 1);
    let CandidateAction::IrqAffinity { plan } = &proposals[0].candidate else {
        panic!("expected irq affinity candidate");
    };
    assert_eq!(plan.action.smp_affinity, "1");
    assert!(plan.evidence[0].value.contains("audio_realtime_cpus={1}"));
    assert!(plan.evidence[0].value.contains("compositor_cpus={2}"));
    assert!(plan.evidence[0].value.contains("reserved_cpus={3}"));
}

#[test]
fn irq_provider_prefers_housekeeping_over_focused_workload_cpu() {
    let provider = IrqAffinityProvider;
    let mut observation = observation_with_irq_pressure();
    observation.active_tasks = vec![task(10, TaskClass::GameRenderThread)];
    observation.active_config_snapshot = Some(ActiveConfigSnapshot {
        affinity: ActiveAffinitySnapshot {
            per_tid: BTreeMap::from([(10, "1".to_owned())]),
        },
        irq: ActiveIrqSnapshot {
            per_irq: BTreeMap::from([(146, "1".to_owned())]),
        },
        ..ActiveConfigSnapshot::default()
    });

    let mut system_context = system_context_with_irq_line(vec![20, 1, 50, 10]);
    system_context.inventory.irq_default_smp_affinity = Some("f".to_owned());

    let proposals = provider.propose(&CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy(),
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    });

    assert_eq!(proposals.len(), 1);
    let CandidateAction::IrqAffinity { plan } = &proposals[0].candidate else {
        panic!("expected irq affinity candidate");
    };
    assert_eq!(plan.action.smp_affinity, "8");
    assert!(plan.evidence[0].value.contains("selection=housekeeping"));
    assert!(plan.evidence[0].value.contains("focused_workload_cpus={1}"));
}

#[test]
fn irq_provider_rejects_when_only_candidate_cpu_is_protected() {
    let provider = IrqAffinityProvider;
    let mut observation = observation_with_irq_pressure();
    observation.objective_signals.irq_hot_cpu = Some(1);
    observation.active_tasks = vec![task(10, TaskClass::AudioRealtime)];
    observation.active_config_snapshot = Some(ActiveConfigSnapshot {
        affinity: ActiveAffinitySnapshot {
            per_tid: BTreeMap::from([(10, "0".to_owned())]),
        },
        irq: ActiveIrqSnapshot {
            per_irq: BTreeMap::from([(146, "2".to_owned())]),
        },
        ..ActiveConfigSnapshot::default()
    });

    let mut system_context = system_context_with_irq_line(vec![1, 2]);
    system_context.inventory.irq_default_smp_affinity = Some("1".to_owned());

    let proposals = provider.propose(&CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy(),
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    });

    assert!(proposals.is_empty());
}

fn observation_with_irq_pressure() -> AutotuneObservation {
    let mut observation = observation();
    observation.objective_signals.irq_overlap_count = Some(2);
    observation.objective_signals.irq_worst_overlap_ns = Some(4_000_000);
    observation.objective_signals.irq_hot_irq = Some(146);
    observation.objective_signals.irq_hot_cpu = Some(2);
    observation.active_config_snapshot = Some(ActiveConfigSnapshot {
        irq: ActiveIrqSnapshot {
            per_irq: BTreeMap::from([(146, "4".to_owned())]),
        },
        ..ActiveConfigSnapshot::default()
    });
    observation
}

fn system_context_with_irq_line(counts_by_cpu: Vec<u64>) -> SystemContextSnapshot {
    let mut system_context = system_context();
    system_context.inventory.irq_lines = vec![IrqLine {
        irq: "146".to_owned(),
        total: counts_by_cpu.iter().sum(),
        counts_by_cpu,
        kind: "PCI-MSI".to_owned(),
        name: "amdgpu".to_owned(),
        raw: "146: PCI-MSI amdgpu".to_owned(),
    }];
    system_context
}

fn task(tid: u32, class: TaskClass) -> ActiveTaskSnapshot {
    ActiveTaskSnapshot {
        tid: tid.into(),
        process_pid: (tid).into(),
        comm: format!("task-{tid}"),
        class,
        process_starttime_ticks: Some(1000 + tid as u64),
        task_starttime_ticks: Some(2000 + tid as u64),
        cgroup_path: None,
    }
}

fn observation() -> AutotuneObservation {
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        primary_situation: SituationKind::IrqPressure,
        focus_kind: Some(FocusGroupKind::Game),
        focus_confidence: 0.95,
        capabilities: DaemonCapabilities {
            irq_affinity_available: true,
            ..DaemonCapabilities::default()
        },
        system_health: SystemHealthSnapshot {
            ok_for_apply: true,
            ..SystemHealthSnapshot::default()
        },
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation.primary_situation = SituationKind::IrqPressure;
    observation
}

fn system_context() -> SystemContextSnapshot {
    SystemContextSnapshot {
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
            inventory_hash: "irq-test".to_owned(),
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
