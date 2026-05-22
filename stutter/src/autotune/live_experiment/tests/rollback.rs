#![allow(unused_imports)]
use super::{
    super::{journal::controller_journal_record_for_live_experiment, *},
    support::*,
};

#[test]
fn start_candidate_applies_action_writes_journal_registers_rollback_and_clears_window() {
    let journal_path = temp_journal_path("start");
    let registry = crate::autotune::shutdown::ActiveAutotuneActionRegistry::new();
    let daemon_policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let input = LiveExperimentManagerInput {
        mode: DaemonMode::ApplyLowRisk,
        controller_policy: ControllerPolicy::from_daemon_policy(&daemon_policy),
        daemon_policy,
        simulate_action_effects: false,
        washout: WashoutWindowConfig::default(),
        candidate_window_seconds: 30,
        manual_restore_command: "stutter daemon emergency-restore",
        controller_journal_path: Some(journal_path),
        exit_rollback_registry: Some(&registry),
        privileged_action_service: None,
    };
    let mut manager = LiveExperimentManager::new();
    let mut controller_state = ControllerRuntimeState::default();
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor::default();
    let observation = observation(1_000, 1_000_000_000);

    let outcome = manager
        .apply_decision_side_effects_with_executor(
            input,
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate: low_risk_candidate(),
                reason: "candidate passed gate".to_owned(),
            },
            "candidate passed gate",
            &mut executor,
        )
        .unwrap();

    assert_eq!(outcome.event, LiveExperimentEvent::Started);
    assert!(outcome.clear_measurement_window);
    assert!(manager.has_active_experiment());
    assert_eq!(executor.apply_calls, 1);
    assert_eq!(registry.len(), 1);
    assert_eq!(controller_state.phase, ControllerPhase::Measuring);
    assert!(controller_state.active_experiment.is_some());
    assert_eq!(
        outcome
            .history_context
            .as_ref()
            .map(|context| context.action_id.as_str()),
        Some("fake-low-risk")
    );
}

#[test]
fn revert_rolls_back_candidate_and_enters_cooldown() {
    let journal_path = temp_journal_path("revert");
    let mut manager = LiveExperimentManager::new();
    manager.set_current_for_tests(live_experiment());
    let mut controller_state = ControllerRuntimeState {
        active_experiment: Some(ControllerActiveExperiment {
            experiment_id: ExperimentId::new("experiment-active"),
            candidate: low_risk_candidate(),
            baseline_score_total: 1_000,
        }),
        ..ControllerRuntimeState::default()
    };
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor::default();
    let observation = observation(1_200, 400);

    let outcome = manager
        .apply_decision_side_effects_with_executor(
            input(journal_path),
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::Revert {
                experiment_id: ExperimentId::new("experiment-active"),
                reason: "candidate regressed".to_owned(),
            },
            "candidate regressed",
            &mut executor,
        )
        .unwrap();

    assert_eq!(outcome.event, LiveExperimentEvent::Reverted);
    assert!(!manager.has_active_experiment());
    assert_eq!(executor.rollback_calls, 1);
    assert_eq!(controller_state.phase, ControllerPhase::Cooldown);
    assert!(controller_state.active_experiment.is_none());
    assert_eq!(
        outcome
            .history_context
            .as_ref()
            .map(|context| context.rollback_performed),
        Some(true)
    );
}

#[test]
fn rollback_verification_success_clears_active_experiment() {
    let journal_path = temp_journal_path("rollback-verify-success");
    let mut manager = LiveExperimentManager::new();
    manager.set_current_for_tests(nice_live_experiment_with_baseline_config());
    let mut controller_state = ControllerRuntimeState {
        active_experiment: Some(ControllerActiveExperiment {
            experiment_id: ExperimentId::new("experiment-nice"),
            candidate: medium_risk_candidate(),
            baseline_score_total: 1_000,
        }),
        ..ControllerRuntimeState::default()
    };
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor {
        post_rollback_active_config: Some(active_nice_config(42, 0)),
        ..FakeLiveExecutor::default()
    };
    let mut observation = observation(1_200, 400);
    observation.active_config_snapshot = Some(active_nice_config(42, 5));

    let outcome = manager
        .apply_decision_side_effects_with_executor(
            medium_input(journal_path, None),
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::Revert {
                experiment_id: ExperimentId::new("experiment-nice"),
                reason: "candidate regressed".to_owned(),
            },
            "candidate regressed",
            &mut executor,
        )
        .unwrap();

    assert_eq!(outcome.event, LiveExperimentEvent::Reverted);
    assert!(!manager.has_active_experiment());
    assert_eq!(controller_state.phase, ControllerPhase::Cooldown);
    assert_eq!(executor.rollback_calls, 1);
}

#[test]
fn rollback_verification_failure_faults_and_keeps_manual_restore_state() {
    let journal_path = temp_journal_path("rollback-verify-failure");
    let mut manager = LiveExperimentManager::new();
    manager.set_current_for_tests(nice_live_experiment_with_baseline_config());
    let mut controller_state = ControllerRuntimeState {
        active_experiment: Some(ControllerActiveExperiment {
            experiment_id: ExperimentId::new("experiment-nice"),
            candidate: medium_risk_candidate(),
            baseline_score_total: 1_000,
        }),
        ..ControllerRuntimeState::default()
    };
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor {
        post_rollback_active_config: Some(active_nice_config(42, 5)),
        ..FakeLiveExecutor::default()
    };
    let mut observation = observation(1_200, 400);
    observation.active_config_snapshot = Some(active_nice_config(42, 5));

    let outcome = manager
        .apply_decision_side_effects_with_executor(
            medium_input(journal_path.clone(), None),
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::Revert {
                experiment_id: ExperimentId::new("experiment-nice"),
                reason: "candidate regressed".to_owned(),
            },
            "candidate regressed",
            &mut executor,
        )
        .unwrap();

    assert_eq!(outcome.event, LiveExperimentEvent::Faulted);
    assert!(manager.has_active_experiment());
    assert_eq!(controller_state.phase, ControllerPhase::Faulted);
    assert_eq!(executor.rollback_calls, 1);
    assert!(
        manager
            .daemon_rollback_state("stutter daemon emergency-restore")
            .is_some()
    );
    assert!(
        outcome
            .history_context
            .as_ref()
            .unwrap()
            .rollback_policy
            .contains("rollback-verification-failed:rollback_state_mismatch")
    );

    let journal =
        crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
    assert_eq!(journal.state(), ControllerJournalState::Faulted);
    assert!(
        journal
            .verify_result
            .as_deref()
            .unwrap()
            .contains("rollback_state_mismatch")
    );
}

#[test]
fn rollback_verification_is_skipped_for_simulated_action_effects() {
    let journal_path = temp_journal_path("rollback-verify-simulated");
    let mut manager = LiveExperimentManager::new();
    manager.set_current_for_tests(nice_live_experiment_with_baseline_config());
    let mut controller_state = ControllerRuntimeState {
        active_experiment: Some(ControllerActiveExperiment {
            experiment_id: ExperimentId::new("experiment-nice"),
            candidate: medium_risk_candidate(),
            baseline_score_total: 1_000,
        }),
        ..ControllerRuntimeState::default()
    };
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor::default();
    let mut observation = observation(1_200, 400);
    observation.active_config_snapshot = Some(active_nice_config(42, 5));
    let mut input = medium_input(journal_path.clone(), None);
    input.simulate_action_effects = true;

    let outcome = manager
        .apply_decision_side_effects_with_executor(
            input,
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::Revert {
                experiment_id: ExperimentId::new("experiment-nice"),
                reason: "candidate regressed".to_owned(),
            },
            "candidate regressed",
            &mut executor,
        )
        .unwrap();

    assert_eq!(outcome.event, LiveExperimentEvent::Reverted);
    assert!(!manager.has_active_experiment());
    assert_eq!(controller_state.phase, ControllerPhase::Cooldown);
    assert_eq!(executor.rollback_calls, 0);

    let journal =
        crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
    assert_eq!(journal.state(), ControllerJournalState::Reverted);
    assert_eq!(journal.verify_result.as_deref(), Some("rollback_simulated"));
}

#[test]
fn rollback_failure_keeps_active_experiment_and_returns_error() {
    let journal_path = temp_journal_path("rollback-failure");
    let mut manager = LiveExperimentManager::new();
    manager.set_current_for_tests(live_experiment());
    let mut controller_state = ControllerRuntimeState::default();
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor {
        fail_rollback: true,
        ..FakeLiveExecutor::default()
    };
    let observation = observation(1_200, 400);

    let err = manager
        .apply_decision_side_effects_with_executor(
            input(journal_path),
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::Revert {
                experiment_id: ExperimentId::new("experiment-active"),
                reason: "candidate regressed".to_owned(),
            },
            "candidate regressed",
            &mut executor,
        )
        .unwrap_err();

    assert!(err.to_string().contains("intentional rollback failure"));
    assert!(manager.has_active_experiment());
    assert_eq!(executor.rollback_calls, 1);
}

#[test]
fn live_experiment_journal_record_carries_phase_metadata_and_rollback() {
    let _manager = LiveExperimentManager::new();
    let journal_path = temp_journal_path("journal-record");
    let input = input(journal_path);
    let observation = observation(1_000, 1_000_000_000);
    let experiment = live_experiment();

    let record = controller_journal_record_for_live_experiment(
        &input,
        &experiment,
        &observation,
        ControllerJournalState::Reverting,
        "rollback_in_progress",
    );

    assert_eq!(record.state(), ControllerJournalState::Reverting);
    assert_eq!(
        record.experiment_action(),
        Some(("experiment-active", "fake-low-risk"))
    );
    assert_eq!(record.candidate.as_deref(), Some("fake-profile"));
    assert_eq!(
        record.target_identity.as_deref(),
        Some("pid:99999:starttime:unknown:active_tasks:1")
    );
    assert_eq!(
        record.verify_result.as_deref(),
        Some("rollback_in_progress")
    );
    assert_eq!(record.mode, Some(DaemonMode::ApplyLowRisk));
    assert_eq!(record.safety_class, Some(SafetyClass::ReversibleLowRisk));
    assert!(record.rollback_token().is_some());
    assert!(record.may_have_mutated_system());
}
