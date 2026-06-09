use super::*;

#[test]
fn runtime_starts_with_default_observation() {
    let runtime = runtime();
    let observation = runtime.observation();

    assert!(!observation.target_present);
    assert_eq!(observation.target_root_pid, None);
    assert_eq!(observation.score.total, 0);
    assert!(observation.data_quality.blocks_action());
}

#[test]
fn apply_medium_runtime_can_start_reversible_medium_experiment_in_simulation() {
    let daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::ApplyMediumRisk,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    let mut daemon_config = daemon_config;
    daemon_config.autotune.allow_medium_risk_apply = true;
    let mut config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None)
        .with_simulated_action_effects();
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-medium".to_owned()),
        SafetyClass::ReversibleMediumRisk,
    );

    runtime
        .apply_decision_side_effects(
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate,
                reason: "test medium start".to_owned(),
            },
            "test medium start",
        )
        .unwrap();

    assert!(runtime.has_active_experiment());
    assert_eq!(runtime.controller.state.phase, ControllerPhase::Measuring);
    assert_eq!(
        runtime
            .pending_history_context
            .as_ref()
            .map(|context| context.action_kind.as_str()),
        Some("fake")
    );
}

#[test]
fn controller_session_finish_rolls_back_active_experiment_on_clean_stop() {
    let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
        .with_simulated_action_effects();
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    runtime.last_observation = observation.clone();

    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-low-risk-stop".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );

    runtime
        .apply_decision_side_effects(
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate,
                reason: "test start".to_owned(),
            },
            "test start",
        )
        .unwrap();

    assert!(runtime.has_active_experiment());

    let exit =
        finish_autotune_controller_session(&mut runtime, Ok("stop requested".to_owned())).unwrap();

    assert_eq!(exit.reason, "stop requested");
    assert!(!runtime.has_active_experiment());
    assert_eq!(runtime.controller.state.phase, ControllerPhase::Cooldown);
    assert!(runtime.controller.state.active_experiment.is_none());
}
