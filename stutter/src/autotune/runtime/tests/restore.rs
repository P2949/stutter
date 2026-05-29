use super::*;

#[test]
fn active_experiment_unknown_config_faults_instead_of_nooping() {
    let mut runtime = runtime();

    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-unknown-active-config".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );

    runtime
        .live_experiments
        .set_current_for_tests(fake_live_experiment(candidate));

    let mut observation = high_quality_game_observation_with_focus_confidence(0.95);
    observation.active_config_snapshot = Some(ActiveConfigSnapshot::default());

    let decision = runtime
        .active_experiment_external_mutation_decision(&observation)
        .expect("unknown active config should produce a recovery decision");

    match decision {
        AutotuneDecision::Fault { reason } => {
            assert!(reason.contains("active_config_unknown"));
            assert!(reason.contains("live state could not be verified"));
            assert!(reason.contains("recovery_decision=fault_require_manual_restore"));
        }
        other => panic!("expected Fault, got {other:?}"),
    }
}

#[test]
fn simulated_fake_candidate_unknown_active_config_noops() {
    let mut config =
        AutotuneRuntimeConfig::observe(None, Some(1234), None).with_simulated_action_effects();
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);

    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-simulated".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );

    runtime
        .live_experiments
        .set_current_for_tests(fake_live_experiment(candidate));

    let mut observation = high_quality_game_observation_with_focus_confidence(0.95);
    observation.active_config_snapshot = Some(ActiveConfigSnapshot::default());

    let decision = runtime.active_experiment_external_mutation_decision(&observation);

    assert!(decision.is_none());
}

#[test]
fn active_experiment_unknown_config_restore_policy_reverts() {
    let mut daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::ApplyLowRisk,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    daemon_config.autotune.external_mutation_policy = ExternalMutationPolicy::Restore;

    let mut config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None);
    config.history_log = None;

    let mut runtime = AutotuneRuntime::new(config);

    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-unknown-active-config".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );

    runtime
        .live_experiments
        .set_current_for_tests(fake_live_experiment(candidate));

    let mut observation = high_quality_game_observation_with_focus_confidence(0.95);
    observation.active_config_snapshot = Some(ActiveConfigSnapshot::default());

    let decision = runtime
        .active_experiment_external_mutation_decision(&observation)
        .expect("unknown active config should produce a recovery decision");

    match decision {
        AutotuneDecision::Revert {
            experiment_id,
            reason,
        } => {
            assert_eq!(experiment_id.as_str(), "experiment-unknown-active-config");
            assert!(reason.contains("active_config_unknown"));
            assert!(reason.contains("recovery_decision=restore_expected_state"));
        }
        other => panic!("expected Revert, got {other:?}"),
    }
}

#[test]
fn active_experiment_unknown_config_resync_policy_abandons_experiment() {
    let mut daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::ApplyLowRisk,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    daemon_config.autotune.external_mutation_policy = ExternalMutationPolicy::Resync;

    let mut config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None);
    config.history_log = None;

    let mut runtime = AutotuneRuntime::new(config);

    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-unknown-active-config".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    let controller_candidate = candidate.clone();

    runtime
        .live_experiments
        .set_current_for_tests(fake_live_experiment(candidate));

    runtime.controller.state.active_experiment =
        Some(crate::autotune::controller::ActiveExperiment {
            experiment_id: ExperimentId::new("experiment-unknown-active-config"),
            candidate: controller_candidate,
            baseline_score: WindowScore {
                started_unix_nanos: 100,
                finished_unix_nanos: 200,
                interval_count: 1,
                scored_samples: 100,
                scored_task_count: 1,
                score: StutterScore {
                    total: 500,
                    ..StutterScore::default()
                },
            },
        });
    runtime.controller.state.phase = ControllerPhase::Measuring;

    let mut observation = high_quality_game_observation_with_focus_confidence(0.95);
    observation.active_config_snapshot = Some(ActiveConfigSnapshot::default());

    let decision = runtime
        .active_experiment_external_mutation_decision(&observation)
        .expect("unknown active config should produce a recovery decision");

    match decision {
        AutotuneDecision::Noop { reason } => {
            assert!(reason.contains("active_config_unknown"));
            assert!(reason.contains("abandoned_active_experiment=true"));
        }
        other => panic!("expected Noop, got {other:?}"),
    }

    assert!(!runtime.has_active_experiment());
    assert!(runtime.controller.state.active_experiment.is_none());
    assert_eq!(runtime.controller.state.phase, ControllerPhase::Observing);
}

#[test]
fn runtime_rollback_on_stop_noops_without_active_experiment() {
    let mut runtime = runtime();

    let snapshot = runtime.rollback_on_stop("daemon stop").unwrap();

    assert!(snapshot.is_none());
    assert!(!runtime.has_active_experiment());
}
