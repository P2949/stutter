use super::*;

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
