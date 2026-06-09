use super::{super::*, support::*};

#[test]
fn low_risk_active_apply_experiment_exposes_daemon_rollback_state_after_start() {
    let journal_path = temp_journal_path("low-risk-rollback-invariant");
    let mut manager = LiveExperimentManager::new();
    let mut controller_state = ControllerRuntimeState::default();
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor::default();
    let observation = observation(1_000, 1_000_000_000);
    let candidate = low_risk_candidate();
    let expected_action_id = candidate.action_id();

    let outcome = manager
        .apply_decision_side_effects_with_executor(
            input(journal_path),
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate,
                reason: "candidate passed rollback invariant gate".to_owned(),
            },
            "candidate passed rollback invariant gate",
            &mut executor,
        )
        .unwrap();

    assert_eq!(outcome.event, LiveExperimentEvent::Started);
    assert!(manager.has_active_experiment());

    let rollback = manager
        .daemon_rollback_state("stutter daemon emergency-restore")
        .expect("active apply experiment must expose daemon rollback state");

    assert_eq!(rollback.action_id, expected_action_id);
    assert_eq!(rollback.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(rollback.safety_class, SafetyClass::ReversibleLowRisk);
    assert!(rollback.rollback_available);
    assert!(rollback.token.is_some());
    assert_eq!(
        rollback.manual_restore_command.as_deref(),
        Some("stutter daemon emergency-restore")
    );
}

#[test]
fn medium_risk_active_apply_experiment_exposes_daemon_rollback_state_after_start() {
    let journal_path = temp_journal_path("medium-risk-rollback-invariant");
    let mut manager = LiveExperimentManager::new();
    let mut controller_state = ControllerRuntimeState::default();
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor::default();
    let observation = observation(1_000, 1_000_000_000);
    let candidate = medium_risk_candidate();
    let expected_action_id = candidate.action_id();

    let outcome = manager
        .apply_decision_side_effects_with_executor(
            medium_input(journal_path, None),
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate,
                reason: "medium candidate passed rollback invariant gate".to_owned(),
            },
            "medium candidate passed rollback invariant gate",
            &mut executor,
        )
        .unwrap();

    assert_eq!(outcome.event, LiveExperimentEvent::Started);
    assert!(manager.has_active_experiment());

    let rollback = manager
        .daemon_rollback_state("stutter daemon emergency-restore")
        .expect("active medium-risk experiment must expose daemon rollback state");

    assert_eq!(rollback.action_id, expected_action_id);
    assert_eq!(rollback.mode, DaemonMode::ApplyMediumRisk);
    assert_eq!(rollback.safety_class, SafetyClass::ReversibleMediumRisk);
    assert!(rollback.rollback_available);
    assert!(rollback.token.is_some());
    assert_eq!(
        rollback.manual_restore_command.as_deref(),
        Some("stutter daemon emergency-restore")
    );
}

#[test]
fn active_window_decision_is_deterministic_for_identical_observation() {
    let mut manager = LiveExperimentManager::new();
    manager.set_current_for_tests(live_experiment());

    let observation = observation(1_000, 150);

    let left = manager
        .active_window_decision(&observation)
        .expect("washout observation should produce a decision");
    let right = manager
        .active_window_decision(&observation)
        .expect("same washout observation should produce a decision");

    assert_eq!(decision_fingerprint(&left), decision_fingerprint(&right));
}

#[test]
fn active_experiment_controller_decision_is_deterministic_for_identical_observation() {
    let daemon_policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    let policy = ControllerPolicy::from_daemon_policy(&daemon_policy);
    let candidate = low_risk_candidate();

    let state = ControllerRuntimeState {
        active_experiment: Some(crate::autotune::controller::ActiveExperiment {
            experiment_id: ExperimentId::new("experiment-active"),
            candidate,
            baseline_score: score(1_000),
        }),
        ..ControllerRuntimeState::default()
    };

    let observation = observation(1_200, 400);

    let left = crate::autotune::controller::decide_autotune_transition(
        &policy,
        &state,
        &observation,
        None,
    );
    let right = crate::autotune::controller::decide_autotune_transition(
        &policy,
        &state,
        &observation,
        None,
    );

    assert_eq!(decision_fingerprint(&left), decision_fingerprint(&right));
}
