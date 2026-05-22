use super::{super::*, support::*};

#[test]
fn active_window_decision_tracks_washout_and_measurement_windows() {
    let mut manager = LiveExperimentManager::new();
    manager.set_current_for_tests(live_experiment());

    let washout = manager
        .active_window_decision(&observation(1_000, 150))
        .unwrap();
    assert!(matches!(washout, AutotuneDecision::Noop { .. }));
    assert!(decision_reason(&washout).contains("washout window"));

    let measurement = manager
        .active_window_decision(&observation(1_000, 250))
        .unwrap();
    assert!(matches!(measurement, AutotuneDecision::Noop { .. }));
    assert!(decision_reason(&measurement).contains("measurement window"));

    assert!(
        manager
            .active_window_decision(&observation(1_000, 350))
            .is_none()
    );
}

#[test]
fn keep_current_records_kept_state_and_clears_active_experiment() {
    let journal_path = temp_journal_path("keep");
    let mut manager = LiveExperimentManager::new();
    manager.set_current_for_tests(live_experiment());
    let mut controller_state = ControllerRuntimeState {
        active_experiment: Some(crate::autotune::controller::ActiveExperiment {
            experiment_id: ExperimentId::new("experiment-active"),
            candidate: low_risk_candidate(),
            baseline_score: score(1_000),
        }),
        ..ControllerRuntimeState::default()
    };
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor::default();
    let observation = observation(500, 400);

    let outcome = manager
        .apply_decision_side_effects_with_executor(
            input(journal_path),
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::KeepCurrent {
                experiment_id: ExperimentId::new("experiment-active"),
                reason: "candidate improved".to_owned(),
            },
            "candidate improved",
            &mut executor,
        )
        .unwrap();

    assert_eq!(outcome.event, LiveExperimentEvent::Kept);
    assert!(!manager.has_active_experiment());
    assert_eq!(controller_state.phase, ControllerPhase::Cooldown);
    assert!(controller_state.active_experiment.is_none());
    assert_eq!(active_profile_state.kept_action_count(), 1);
    assert_eq!(executor.rollback_calls, 0);
    assert_eq!(
        outcome
            .history_context
            .as_ref()
            .map(|context| context.rollback_performed),
        Some(false)
    );
}

#[test]
fn compare_keep_result_rejects_io_candidate_when_live_io_signal_regresses() {
    let experiment = LiveExperiment {
        experiment_id: ExperimentId::new("io-test"),
        safety_class: SafetyClass::ReversibleMediumRisk,
        mode: DaemonMode::ApplyMediumRisk,
        candidate: CandidateAction::IoPrio {
            plan: crate::autotune::planning::executable_plan::IoPrioActionPlan {
                name: "fake-io".to_owned(),
                action: crate::actions::ioprio::IoPrioAction {
                    targets: vec![crate::actions::TaskIdentity {
                        tid: 99999,
                        process_pid: Some(99999),
                        comm: Some("fake-io".to_owned()),
                        starttime_ticks: None,
                    }],
                    ioprio: crate::actions::ioprio::IoPrioValue::best_effort(0),
                    policy: crate::actions::ioprio::IoPrioPolicy {
                        allow_ioprio_changes: true,
                        strong_block_io_evidence: true,
                        ..Default::default()
                    },
                },
                target_root_pid: Some(99999),
                evidence: Vec::new(),
                objective: ObjectiveKind::IoLatency,
            },
        },
        baseline_score: score(1_000),
        baseline_signals: ObjectiveSignals {
            block_io_overlap_count: Some(1),
            block_io_worst_latency_ns: Some(2_000_000),
            ..ObjectiveSignals::from_window_score(&score(1_000))
        },
        baseline_active_config: None,
        applied_unix_nanos: 10,
        washout_until_unix_nanos: 20,
        measure_until_unix_nanos: 30,
        rollback: fake_rollback(),
    };
    let candidate_score = score(800);
    let observation = AutotuneObservation {
        target_present: true,
        target_root_pid: Some(99999),
        data_quality: OnlineDataQuality::High,
        objective_signals: ObjectiveSignals {
            block_io_overlap_count: Some(2),
            block_io_worst_latency_ns: Some(3_000_000),
            ..ObjectiveSignals::from_window_score(&candidate_score)
        },
        ..AutotuneObservation::default()
    };

    let result =
        LiveExperimentManager::compare_keep_result(&experiment, &candidate_score, &observation);

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
    assert_eq!(experiment.candidate.objective(), ObjectiveKind::IoLatency);
}
