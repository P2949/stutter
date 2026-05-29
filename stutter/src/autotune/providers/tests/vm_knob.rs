use super::*;

#[test]
fn vm_knob_provider_stays_silent_without_memory_or_swap_evidence() {
    let policy = policy(DaemonMode::Suggest);
    let provider = VmKnobProvider;
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
