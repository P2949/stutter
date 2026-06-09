use super::*;

#[test]
fn vm_swappiness_provider_produces_medium_risk_non_manual_candidate() {
    let provider = VmKnobProvider;
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 1,
        scored_task_count: 1,
        interval_count: 3,
        scored_samples: 300,
        data_quality: OnlineDataQuality::High,
        primary_situation: SituationKind::IoPressure,
        focus_kind: Some(FocusGroupKind::Desktop),
        focus_confidence: 0.95,
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation.primary_situation = SituationKind::IoPressure;
    observation.situation.confidence = 0.95;
    observation.objective_signals.swap_activity_events = Some(100);

    let mut system_context = SystemContextSnapshot::from_observation(&observation);
    system_context
        .inventory
        .vm_knobs
        .insert("proc/sys/vm/swappiness".to_owned(), "60".to_owned());
    let policy = DaemonPolicy::suggest(ActionSource::Test);
    let controller_state = ControllerRuntimeState::default();

    let proposals = provider.propose(&CandidateProviderInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &observation.capabilities,
        system_health: &observation.system_health,
        system_context: &system_context,
        controller_state: &controller_state,
        profiles: &[],
    });

    let swappiness = proposals
        .iter()
        .find(|proposal| match &proposal.candidate {
            CandidateAction::VmKnob { plan } => plan
                .action
                .changes
                .iter()
                .any(|change| change.path == *Path::new("proc/sys/vm/swappiness")),

            _ => false,
        })
        .expect("vm.swappiness proposal should be produced for swap pressure");

    assert_eq!(
        swappiness.candidate.safety_class(),
        SafetyClass::ReversibleMediumRisk
    );
    assert!(!swappiness.candidate.is_high_risk_system_adjacent());
    assert!(swappiness.candidate.manual_only_reason().is_none());
}

#[test]
fn activity_classifier_suppresses_idle_game_candidates() {
    let candidate = CandidateAction::fake(
        ActionId::new("idle-suppressed-fake".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    let mut registry = CandidateProviderRegistry::default();
    registry.register(Box::new(StaticCandidateProvider { candidate }));
    let planner = CandidatePlanner::new(registry);
    let mut observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(1234),
        active_target_count: 1,
        scored_task_count: 1,
        interval_count: 5,
        scored_samples: 0,
        data_quality: OnlineDataQuality::High,
        activity_level: ActivityLevel::Idle,
        primary_situation: SituationKind::GameCpuSchedulerPressure,
        focus_kind: Some(FocusGroupKind::Game),
        focus_confidence: 0.95,
        focus_roots: vec![1234],
        ..AutotuneObservation::default()
    };
    observation.refresh_situation_classification();
    observation.primary_situation = SituationKind::GameCpuSchedulerPressure;
    let policy = DaemonPolicy::suggest(ActionSource::Test);
    let workload_policy = WorkloadPolicyMatrix::default_rules();
    let controller_state = ControllerRuntimeState::default();
    let capabilities = DaemonCapabilities::default();
    let system_health = SystemHealthSnapshot::default();

    let result = planner.plan(PlannerInput {
        observation: &observation,
        daemon_policy: &policy,
        capabilities: &capabilities,
        system_health: &system_health,
        controller_state: &controller_state,
        active_profile_state: None,
        workload_policy: &workload_policy,
        profiles: &[],
    });

    assert!(result.selected.is_none());
    assert!(result.evaluations.iter().any(|evaluation| {
        evaluation
            .deny_reasons
            .contains(&CandidateDenyReason::WorkloadIdle)
    }));
}
