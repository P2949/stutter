use super::*;

#[test]
fn nice_provider_targets_compile_active_tasks_not_only_root_pid() {
    let policy = apply_medium_policy_with_compile_cgroup();
    let provider = NiceProvider;
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
            .map(|target| target.tid.as_u32())
            .collect::<Vec<_>>(),
        vec![1234, 1235, 1236]
    );
    assert_eq!(
        plan.action
            .targets
            .iter()
            .map(|target| target.process_pid.map(|pid| pid.as_u32()))
            .collect::<Vec<_>>(),
        vec![Some(1234), Some(1234), Some(1234)]
    );
    assert!(proposals[0].deny_reasons.is_empty());
}
