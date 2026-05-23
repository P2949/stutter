use super::*;
use crate::{
    actions::ActionId,
    autotune::{
        controller::ControllerRuntimeState,
        observation::{ActiveTaskSnapshot, AutotuneObservation, ProtectedTask},
        quality::OnlineDataQuality,
        state::SituationKind,
    },
    daemon::{
        health::SystemHealthSnapshot,
        policy::{ActionSource, DaemonMode},
    },
    daemon_policy::{DaemonPolicyBuildInput, build_daemon_policy},
    focus::FocusGroupKind,
    process_tree::TaskClass,
    system_inventory::DrmDeviceInventory,
};

fn policy(mode: DaemonMode) -> DaemonPolicy {
    let config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        mode,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

fn policy_with_system_wide_suggestions(mode: DaemonMode) -> DaemonPolicy {
    let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        mode,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    config.safety.allow_system_wide_suggestions = true;
    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

fn policy_with_compile_cgroup() -> DaemonPolicy {
    let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        DaemonMode::Suggest,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    config.safety.cgroup_targets.compile_cgroup = Some(std::path::PathBuf::from(
        "/user.slice/stutter-compile.slice",
    ));
    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

fn apply_medium_policy_with_compile_cgroup() -> DaemonPolicy {
    let mut config = crate::autotune::runtime::daemon_config_for_runtime_mode(
        DaemonMode::ApplyMediumRisk,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    config.autotune.allow_medium_risk_apply = true;
    config.safety.cgroup_targets.compile_cgroup = Some(std::path::PathBuf::from(
        "/user.slice/stutter-compile.slice",
    ));
    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

fn system_context_for_observation(observation: &AutotuneObservation) -> SystemContextSnapshot {
    SystemContextSnapshot::from_observation(observation)
}

fn calibration_proposal(provider: &'static str, confidence: f32) -> CandidateProposal {
    CandidateProposal {
        candidate: CandidateAction::fake(
            ActionId::new(format!("{provider}:calibration")),
            SafetyClass::HighRisk,
        ),
        provider,
        confidence,
        deny_reasons: Vec::new(),
        objective: ObjectiveKind::DesktopInteractivity,
        rank_hint: 1,
    }
}

fn active_task_snapshot() -> ActiveTaskSnapshot {
    ActiveTaskSnapshot {
        tid: 1234,
        process_pid: 1234,
        comm: "game".to_owned(),
        class: TaskClass::Game,
        process_starttime_ticks: Some(10),
        task_starttime_ticks: Some(10),
        cgroup_path: None,
    }
}

#[test]
fn registry_includes_safe_and_suggest_first_provider_families() {
    let registry = CandidateProviderRegistry::default_for_policy(
        &policy_with_system_wide_suggestions(DaemonMode::Suggest),
    );
    let families = registry.families();

    assert!(families.contains(&"cpu_affinity_profile"));
    assert!(families.contains(&"nice"));
    assert!(families.contains(&"ionice"));
    assert!(families.contains(&"uclamp"));
    assert!(families.contains(&"cgroup_placement"));
    assert!(families.contains(&"irq_affinity"));
    assert!(families.contains(&"cpu_power"));
    assert!(families.contains(&"gpu_power"));
    assert!(families.contains(&"vm_knob"));
}

#[test]
fn registered_providers_expose_complete_policy_metadata() {
    let registry = CandidateProviderRegistry::default_for_policy(
        &policy_with_system_wide_suggestions(DaemonMode::Suggest),
    );

    for metadata in registry.metadata() {
        assert!(!metadata.family.is_empty());
        assert!(!metadata.description.trim().is_empty());
        assert_ne!(
            metadata.rollback_requirement,
            RollbackRequirement::Unavailable
        );
        assert!(
            !metadata.capability_requirements.is_empty(),
            "{} must document capability requirements",
            metadata.family
        );
        assert_ne!(
            metadata.conflict_group,
            ActionConflictGroup::None,
            "{} must declare a conflict group",
            metadata.family
        );
        assert!(!metadata.cooldown_key.is_empty());
        assert!(
            !metadata.policy_coverage.is_empty(),
            "{} must document policy gates",
            metadata.family
        );

        match metadata.safety_class {
            SafetyClass::ReversibleLowRisk
            | SafetyClass::ReversibleMediumRisk
            | SafetyClass::HighRisk => {}
            SafetyClass::ObserveOnly => {
                panic!(
                    "{} is an action provider and must not be observe-only",
                    metadata.family
                )
            }
        }

        match metadata.required_mode {
            DaemonMode::Suggest | DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk => {}
            DaemonMode::Observe | DaemonMode::ApplyHighRisk => {
                panic!(
                    "{} must declare suggest/apply-low/apply-medium as its required mode",
                    metadata.family
                )
            }
        }

        let objective = format!("{:?}", metadata.objective);
        assert!(!objective.is_empty());
    }
}

#[test]
fn confidence_calibration_caps_process_local_without_active_tasks() {
    let observation = AutotuneObservation::default();
    let context = system_context_for_observation(&observation);
    let policy = policy(DaemonMode::Suggest);
    let health = SystemHealthSnapshot::default();
    let controller_state = ControllerRuntimeState::default();
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &health,
        system_context: &context,
        controller_state: &controller_state,
        profiles: &[],
    };

    let proposal = calibrate_provider_proposal(calibration_proposal("nice", 0.95), &input);

    assert!(proposal.confidence <= 0.49);
}

#[test]
fn confidence_calibration_preserves_process_local_with_active_tasks() {
    let observation = AutotuneObservation {
        active_tasks: vec![active_task_snapshot()],
        ..AutotuneObservation::default()
    };
    let context = system_context_for_observation(&observation);
    let policy = policy(DaemonMode::Suggest);
    let health = SystemHealthSnapshot::default();
    let controller_state = ControllerRuntimeState::default();
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &health,
        system_context: &context,
        controller_state: &controller_state,
        profiles: &[],
    };

    let proposal = calibrate_provider_proposal(calibration_proposal("nice", 0.95), &input);

    assert_eq!(proposal.confidence, 0.95);
}

#[test]
fn confidence_calibration_caps_missing_irq_identity() {
    let observation = AutotuneObservation::default();
    let context = system_context_for_observation(&observation);
    let policy = policy_with_system_wide_suggestions(DaemonMode::Suggest);
    let health = SystemHealthSnapshot::default();
    let controller_state = ControllerRuntimeState::default();
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &health,
        system_context: &context,
        controller_state: &controller_state,
        profiles: &[],
    };

    let proposal = calibrate_provider_proposal(calibration_proposal("irq_affinity", 0.95), &input);

    assert!(proposal.confidence <= 0.49);
}

#[test]
fn confidence_calibration_caps_multigpu_without_focused_gpu_identity() {
    let observation = AutotuneObservation::default();
    let mut context = system_context_for_observation(&observation);
    context.inventory.drm_devices = vec![
        DrmDeviceInventory {
            name: "card0".to_owned(),
            path: "/sys/class/drm/card0".into(),
            render_node: Some("/dev/dri/renderD128".to_owned()),
            pci_id: None,
            vendor: None,
            hwmon_paths: Vec::new(),
        },
        DrmDeviceInventory {
            name: "card1".to_owned(),
            path: "/sys/class/drm/card1".into(),
            render_node: Some("/dev/dri/renderD129".to_owned()),
            pci_id: None,
            vendor: None,
            hwmon_paths: Vec::new(),
        },
    ];
    let policy = policy_with_system_wide_suggestions(DaemonMode::Suggest);
    let health = SystemHealthSnapshot::default();
    let controller_state = ControllerRuntimeState::default();
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &health,
        system_context: &context,
        controller_state: &controller_state,
        profiles: &[],
    };

    let proposal = calibrate_provider_proposal(calibration_proposal("gpu_power", 0.95), &input);

    assert!(proposal.confidence <= 0.49);
}

#[test]
fn confidence_calibration_caps_laptop_cpu_power_without_power_source_state() {
    let observation = AutotuneObservation::default();
    let mut context = system_context_for_observation(&observation);
    context.inventory.power_source.battery_present = true;
    context.inventory.power_source.ac_online = None;
    context.inventory.power_source.battery_discharging = None;
    let policy = policy_with_system_wide_suggestions(DaemonMode::Suggest);
    let health = SystemHealthSnapshot::default();
    let controller_state = ControllerRuntimeState::default();
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &health,
        system_context: &context,
        controller_state: &controller_state,
        profiles: &[],
    };

    let proposal = calibrate_provider_proposal(calibration_proposal("cpu_power", 0.95), &input);

    assert!(proposal.confidence <= 0.60);
}

#[test]
fn confidence_calibration_caps_vm_without_memory_or_writeback_signal() {
    let observation = AutotuneObservation::default();
    let context = system_context_for_observation(&observation);
    let policy = policy_with_system_wide_suggestions(DaemonMode::Suggest);
    let health = SystemHealthSnapshot::default();
    let controller_state = ControllerRuntimeState::default();
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &health,
        system_context: &context,
        controller_state: &controller_state,
        profiles: &[],
    };

    let proposal = calibrate_provider_proposal(calibration_proposal("vm_knob", 0.95), &input);

    assert!(proposal.confidence <= 0.60);
}

#[test]
fn vm_knob_provider_stays_silent_without_memory_or_swap_evidence() {
    let policy = policy(DaemonMode::Suggest);
    let provider = vm_knob::VmKnobProvider;
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 1,
        data_quality: OnlineDataQuality::High,
        primary_situation: SituationKind::IoPressure,
        focus_kind: Some(FocusGroupKind::Browser),
        focus_confidence: 0.95,
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation.primary_situation = SituationKind::IoPressure;

    let system_context = system_context_for_observation(&observation);
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
fn cgroup_provider_requires_configured_allowlist() {
    let policy = policy(DaemonMode::Suggest);
    let provider = cgroup::CgroupProvider;
    let observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        primary_situation: SituationKind::CompileCpuBound,
        focus_kind: Some(FocusGroupKind::Compile),
        focus_confidence: 0.95,
        ..AutotuneObservation::default()
    };

    let system_context = system_context_for_observation(&observation);
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
fn cgroup_provider_moves_only_allowed_non_protected_target_tasks() {
    let policy = policy_with_compile_cgroup();
    let provider = cgroup::CgroupProvider;
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        primary_situation: SituationKind::CompileCpuBound,
        focus_kind: Some(FocusGroupKind::Compile),
        focus_confidence: 0.95,
        active_tasks: vec![
            ActiveTaskSnapshot {
                tid: 1234,
                process_pid: 1234,
                comm: "rustc".to_owned(),
                class: TaskClass::Compiler,
                process_starttime_ticks: Some(10),
                task_starttime_ticks: Some(10),
                cgroup_path: Some("/user.slice/app.scope".to_owned()),
            },
            ActiveTaskSnapshot {
                tid: 1235,
                process_pid: 1234,
                comm: "ld.lld".to_owned(),
                class: TaskClass::Linker,
                process_starttime_ticks: Some(10),
                task_starttime_ticks: Some(11),
                cgroup_path: Some("/user.slice/app.scope".to_owned()),
            },
            ActiveTaskSnapshot {
                tid: 77,
                process_pid: 77,
                comm: "pipewire".to_owned(),
                class: TaskClass::AudioRealtime,
                process_starttime_ticks: Some(20),
                task_starttime_ticks: Some(20),
                cgroup_path: Some("/user.slice/session.scope".to_owned()),
            },
        ],
        protected_tasks: vec![ProtectedTask {
            tid: 77,
            process_pid: 77,
            comm: "pipewire".to_owned(),
            class: TaskClass::AudioRealtime,
            reason: "audio realtime task".to_owned(),
        }],
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();

    let system_context = system_context_for_observation(&observation);
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
    let CandidateAction::CgroupPlacement { plan } = &proposals[0].candidate else {
        panic!("expected cgroup candidate");
    };
    assert_eq!(
        plan.action.target_cgroup,
        std::path::PathBuf::from("/user.slice/stutter-compile.slice")
    );
    let tids = plan
        .action
        .targets
        .iter()
        .map(|target| target.identity.tid)
        .collect::<Vec<_>>();
    assert_eq!(tids, vec![1234, 1235]);
}

#[test]
fn process_local_providers_do_not_fallback_without_active_snapshots_in_apply_modes() {
    let policy = apply_medium_policy_with_compile_cgroup();
    let mut observation =
        provider_observation(SituationKind::CompileCpuBound, FocusGroupKind::Compile);
    observation.capabilities.ionice_available = true;
    observation.capabilities.uclamp_available = true;
    let system_context = system_context_for_observation(&observation);
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    };

    assert!(nice::NiceProvider.propose(&input).is_empty());
    assert!(uclamp::UclampProvider.propose(&input).is_empty());
    assert!(cgroup::CgroupProvider.propose(&input).is_empty());

    let mut io_observation =
        provider_observation(SituationKind::IoPressure, FocusGroupKind::Desktop);
    io_observation.capabilities.ionice_available = true;
    let io_system_context = system_context_for_observation(&io_observation);
    let io_input = CandidateProviderInput {
        observation: &io_observation,
        daemon_policy: &policy,
        capabilities: &io_observation.capabilities,
        system_health: &io_observation.system_health,
        system_context: &io_system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    };

    assert!(ioprio::IoPrioProvider.propose(&io_input).is_empty());
}

#[test]
fn suggest_mode_marks_fallback_root_target_selection() {
    let policy = policy(DaemonMode::Suggest);
    let observation = provider_observation(SituationKind::CompileCpuBound, FocusGroupKind::Compile);
    let system_context = system_context_for_observation(&observation);
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    };

    let proposals = nice::NiceProvider.propose(&input);

    assert_eq!(proposals.len(), 1);
    assert!(
        proposals[0]
            .deny_reasons
            .iter()
            .any(|reason| reason.contains("target_selection_fallback_root"))
    );
    let CandidateAction::Nice { plan } = &proposals[0].candidate else {
        panic!("expected nice candidate");
    };
    assert!(
        plan.evidence
            .iter()
            .any(|evidence| evidence.signal == "target_selection_fallback_root")
    );
}

#[test]
fn nice_provider_targets_compile_active_tasks_not_only_root_pid() {
    let policy = apply_medium_policy_with_compile_cgroup();
    let provider = nice::NiceProvider;
    let mut observation =
        provider_observation(SituationKind::CompileCpuBound, FocusGroupKind::Compile);
    observation.active_tasks = vec![
        provider_task(1234, 1234, "cargo", TaskClass::BuildJob),
        provider_task(1235, 1234, "rustc", TaskClass::Compiler),
        provider_task(1236, 1234, "ld.lld", TaskClass::Linker),
        provider_task(1237, 1234, "pipewire", TaskClass::AudioRealtime),
        provider_task(1238, 1234, "unknown-helper", TaskClass::Unknown),
    ];
    let system_context = system_context_for_observation(&observation);
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    };

    let proposals = provider.propose(&input);

    assert_eq!(proposals.len(), 1);
    let CandidateAction::Nice { plan } = &proposals[0].candidate else {
        panic!("expected nice candidate");
    };
    assert_eq!(
        plan.action
            .targets
            .iter()
            .map(|target| target.tid)
            .collect::<Vec<_>>(),
        vec![1234, 1235, 1236]
    );
    assert_eq!(
        plan.action
            .targets
            .iter()
            .map(|target| target.process_pid)
            .collect::<Vec<_>>(),
        vec![Some(1234), Some(1234), Some(1234)]
    );
    assert!(proposals[0].deny_reasons.is_empty());
}

#[test]
fn ionice_provider_targets_active_tree_and_excludes_protected_or_unknown_tasks() {
    let policy = apply_medium_policy_with_compile_cgroup();
    let provider = ioprio::IoPrioProvider;
    let mut observation = provider_observation(SituationKind::IoPressure, FocusGroupKind::Desktop);
    observation.capabilities.ionice_available = true;
    observation.active_tasks = vec![
        provider_task(1234, 1234, "game", TaskClass::Game),
        provider_task(1235, 1234, "game-worker", TaskClass::GameWorkerThread),
        provider_task(1236, 1234, "helper", TaskClass::Helper),
        provider_task(1237, 1234, "pipewire", TaskClass::AudioRealtime),
        provider_task(1238, 1234, "compositor", TaskClass::Compositor),
        provider_task(1239, 1234, "unknown", TaskClass::Unknown),
    ];
    let system_context = system_context_for_observation(&observation);
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    };

    let proposals = provider.propose(&input);

    assert_eq!(proposals.len(), 1);
    let CandidateAction::IoPrio { plan } = &proposals[0].candidate else {
        panic!("expected ionice candidate");
    };
    assert_eq!(
        plan.action
            .targets
            .iter()
            .map(|target| target.tid)
            .collect::<Vec<_>>(),
        vec![1234, 1235, 1236]
    );
    assert!(proposals[0].deny_reasons.is_empty());
}

#[test]
fn uclamp_provider_targets_game_render_and_worker_active_tasks_not_only_root_pid() {
    let policy = apply_medium_policy_with_compile_cgroup();
    let provider = uclamp::UclampProvider;
    let mut observation = provider_observation(
        SituationKind::GameCpuSchedulerPressure,
        FocusGroupKind::Game,
    );
    observation.capabilities.uclamp_available = true;
    observation.active_tasks = vec![
        provider_task(1234, 1234, "game", TaskClass::Game),
        provider_task(1235, 1234, "render", TaskClass::GameRenderThread),
        provider_task(1236, 1234, "worker", TaskClass::GameWorkerThread),
        provider_task(1237, 1234, "input", TaskClass::Input),
        provider_task(1238, 1234, "irq", TaskClass::IrqThread),
        provider_task(1239, 1234, "service", TaskClass::Service),
        provider_task(1240, 1234, "unknown", TaskClass::Unknown),
    ];
    let system_context = system_context_for_observation(&observation);
    let input = CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    };

    let proposals = provider.propose(&input);

    assert_eq!(proposals.len(), 1);
    let CandidateAction::Uclamp { plan } = &proposals[0].candidate else {
        panic!("expected uclamp candidate");
    };
    assert_eq!(
        plan.action
            .targets
            .iter()
            .map(|target| target.tid)
            .collect::<Vec<_>>(),
        vec![1234, 1235, 1236]
    );
    assert!(proposals[0].deny_reasons.is_empty());
}

fn provider_task(tid: u32, process_pid: u32, comm: &str, class: TaskClass) -> ActiveTaskSnapshot {
    let process_starttime_ticks = Some(10);
    let task_starttime_ticks = if tid == process_pid {
        process_starttime_ticks
    } else {
        Some(u64::from(tid))
    };

    ActiveTaskSnapshot {
        tid,
        process_pid,
        comm: comm.to_owned(),
        class,
        process_starttime_ticks,
        task_starttime_ticks,
        cgroup_path: Some("/user.slice/provider-test.scope".to_owned()),
    }
}

fn provider_observation(
    situation: SituationKind,
    focus_kind: FocusGroupKind,
) -> AutotuneObservation {
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 1,
        primary_situation: situation,
        focus_kind: Some(focus_kind),
        focus_confidence: 0.95,
        system_health: SystemHealthSnapshot {
            ok_for_apply: true,
            ..SystemHealthSnapshot::default()
        },
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation.primary_situation = situation;
    observation
}
