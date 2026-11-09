use super::*;

#[test]
fn ionice_provider_never_lowers_the_important_tree_without_a_measured_offender() {
    let policy = apply_medium_policy_with_compile_cgroup();
    let provider = IoPrioProvider;
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

    // Generic I/O pressure has no measured competing offender, and the game tree is the
    // important workload: nothing is proposed, whatever tasks it contains.
    assert!(provider.propose(&input).is_empty());
}
