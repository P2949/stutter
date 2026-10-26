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

fn offender(pid: u32, nice: i32) -> crate::autotune::offenders::OffenderProcess {
    crate::autotune::offenders::OffenderProcess {
        pid: pid.into(),
        starttime_ticks: 77,
        comm: "hog".to_owned(),
        uid: 1000,
        nice,
        cpu_percent: 96.0,
        sustained_samples: 4,
        tasks: vec![
            crate::actions::TaskIdentity {
                tid: pid.into(),
                process_pid: Some(pid.into()),
                comm: Some("hog".to_owned()),
                starttime_ticks: Some(77),
            },
            crate::actions::TaskIdentity {
                tid: (pid + 1).into(),
                process_pid: Some(pid.into()),
                comm: Some("hog-worker".to_owned()),
                starttime_ticks: Some(78),
            },
        ],
    }
}

fn propose_for(
    observation: &AutotuneObservation,
) -> Vec<crate::autotune::providers::CandidateProposal> {
    let policy = apply_medium_policy_with_compile_cgroup();
    let system_context = system_context_for_observation(observation);
    NiceProvider.propose(&CandidateProviderInput {
        observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &ControllerRuntimeState::default(),
        profiles: &[],
    })
}

#[test]
fn under_game_scheduler_pressure_the_provider_demotes_exactly_the_measured_offenders() {
    let mut observation = provider_observation(
        SituationKind::GameCpuSchedulerPressure,
        FocusGroupKind::Game,
    );
    observation.background_offenders = vec![offender(900, 0), offender(910, 0)];
    let proposals = propose_for(&observation);
    assert_eq!(proposals.len(), 1);
    let CandidateAction::Nice { plan } = &proposals[0].candidate else {
        panic!("expected nice candidate");
    };
    assert_eq!(plan.action.nice, 10);
    assert_eq!(plan.objective, ObjectiveKind::GameRunnableLatency);
    assert_eq!(
        plan.action
            .targets
            .iter()
            .map(|target| target.tid.as_u32())
            .collect::<Vec<_>>(),
        vec![900, 901, 910, 911],
        "every task of every offender, nothing else"
    );
    assert!(
        plan.evidence
            .iter()
            .any(|entry| entry.signal == "background_cpu_offender")
    );
}

#[test]
fn without_offenders_or_when_already_demoted_nothing_is_proposed_for_a_game() {
    let mut observation = provider_observation(
        SituationKind::GameCpuSchedulerPressure,
        FocusGroupKind::Game,
    );
    assert!(propose_for(&observation).is_empty());
    observation.background_offenders = vec![offender(900, 10), offender(910, 19)];
    assert!(propose_for(&observation).is_empty());
}
