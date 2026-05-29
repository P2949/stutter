use super::*;

#[test]
fn cgroup_provider_requires_configured_allowlist() {
    let policy = policy(DaemonMode::Suggest);
    let provider = CgroupProvider;
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
    let provider = CgroupProvider;
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        primary_situation: SituationKind::CompileCpuBound,
        focus_kind: Some(FocusGroupKind::Compile),
        focus_confidence: 0.95,
        active_tasks: vec![
            ActiveTaskSnapshot {
                tid: (1234).into(),
                process_pid: (1234).into(),
                comm: "rustc".to_owned(),
                class: TaskClass::Compiler,
                process_starttime_ticks: Some(10),
                task_starttime_ticks: Some(10),
                cgroup_path: Some("/user.slice/app.scope".to_owned()),
            },
            ActiveTaskSnapshot {
                tid: (1235).into(),
                process_pid: (1234).into(),
                comm: "ld.lld".to_owned(),
                class: TaskClass::Linker,
                process_starttime_ticks: Some(10),
                task_starttime_ticks: Some(11),
                cgroup_path: Some("/user.slice/app.scope".to_owned()),
            },
            ActiveTaskSnapshot {
                tid: (77).into(),
                process_pid: (77).into(),
                comm: "pipewire".to_owned(),
                class: TaskClass::AudioRealtime,
                process_starttime_ticks: Some(20),
                task_starttime_ticks: Some(20),
                cgroup_path: Some("/user.slice/session.scope".to_owned()),
            },
        ],
        protected_tasks: vec![ProtectedTask {
            tid: (77).into(),
            process_pid: (77).into(),
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
        .map(|target| target.identity.tid.as_u32())
        .collect::<Vec<_>>();
    assert_eq!(tids, vec![1234, 1235]);
}
