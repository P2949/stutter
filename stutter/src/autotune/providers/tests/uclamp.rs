use super::*;

#[test]
fn uclamp_provider_targets_game_render_and_worker_active_tasks_not_only_root_pid() {
    let policy = apply_medium_policy_with_compile_cgroup();
    let provider = UclampProvider;
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
            .map(|target| target.tid.as_u32())
            .collect::<Vec<_>>(),
        vec![1234, 1235, 1236]
    );
    assert!(proposals[0].deny_reasons.is_empty());
}
