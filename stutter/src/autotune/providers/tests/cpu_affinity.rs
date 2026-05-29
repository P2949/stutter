use super::*;

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

    assert!(NiceProvider.propose(&input).is_empty());
    assert!(UclampProvider.propose(&input).is_empty());
    assert!(CgroupProvider.propose(&input).is_empty());

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

    assert!(IoPrioProvider.propose(&io_input).is_empty());
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

    let proposals = NiceProvider.propose(&input);

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
