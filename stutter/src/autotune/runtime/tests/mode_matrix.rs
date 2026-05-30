use super::*;
use crate::actions::ActionId;

fn fake_mode_candidate(id: &str, safety_class: SafetyClass) -> CandidateAction {
    CandidateAction::fake(ActionId::new(id.to_owned()), safety_class)
}

#[test]
fn observe_mode_matrix_never_starts_experiment() {
    let mut runtime = AutotuneRuntime::new(AutotuneRuntimeConfig::observe(None, Some(1234), None));
    assert_eq!(runtime.config().mode(), DaemonMode::Observe);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    let candidate = fake_mode_candidate("mode-observe-candidate", SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(
        &runtime.controller.policy,
        &runtime.controller.state,
        &observation,
        Some(candidate),
    );

    let AutotuneDecision::Noop { reason } = &decision else {
        panic!("observe mode should produce Noop, got {decision:?}");
    };
    assert!(reason.contains("observe mode never applies or suggests actions"));

    runtime
        .apply_decision_side_effects(&observation, &decision, "observe mode matrix")
        .unwrap();

    assert!(!runtime.has_active_experiment());
    assert!(runtime.daemon_state_snapshot().active_rollback.is_none());
}

#[test]
fn suggest_mode_matrix_reports_candidate_without_starting_experiment() {
    let mut runtime = AutotuneRuntime::new(AutotuneRuntimeConfig::suggest(None, Some(1234), None));
    assert_eq!(runtime.config().mode(), DaemonMode::Suggest);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    let candidate = fake_mode_candidate("mode-suggest-candidate", SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(
        &runtime.controller.policy,
        &runtime.controller.state,
        &observation,
        Some(candidate),
    );

    let AutotuneDecision::Suggest { candidate, reason } = &decision else {
        panic!("suggest mode should produce Suggest, got {decision:?}");
    };
    assert_eq!(candidate.action_id().as_str(), "mode-suggest-candidate");
    assert!(reason.contains("suggest mode reports candidate without applying"));

    runtime
        .apply_decision_side_effects(&observation, &decision, "suggest mode matrix")
        .unwrap();

    assert!(!runtime.has_active_experiment());
    assert!(runtime.daemon_state_snapshot().active_rollback.is_none());
}

#[test]
fn apply_low_risk_mode_matrix_starts_experiment_and_exposes_rollback_state() {
    let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
        .with_simulated_action_effects();
    config.history_log = None;

    let mut runtime = AutotuneRuntime::new(config);
    assert_eq!(runtime.config().mode(), DaemonMode::ApplyLowRisk);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    let candidate = fake_mode_candidate("mode-apply-low-candidate", SafetyClass::ReversibleLowRisk);

    let decision = decide_autotune_transition(
        &runtime.controller.policy,
        &runtime.controller.state,
        &observation,
        Some(candidate),
    );

    let AutotuneDecision::StartExperiment { candidate, reason } = &decision else {
        panic!("apply-low-risk mode should start an experiment, got {decision:?}");
    };
    assert_eq!(candidate.action_id().as_str(), "mode-apply-low-candidate");
    assert!(reason.contains("candidate passed data-quality and safety gates"));

    runtime
        .apply_decision_side_effects(&observation, &decision, "apply-low-risk mode matrix")
        .unwrap();

    assert!(runtime.has_active_experiment());

    let daemon_state = runtime.daemon_state_snapshot();
    let rollback = daemon_state
        .active_rollback
        .expect("active low-risk experiment should expose rollback state");

    assert_eq!(rollback.action_id, "mode-apply-low-candidate");
    assert_eq!(rollback.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(rollback.safety_class, SafetyClass::ReversibleLowRisk);
    assert!(rollback.rollback_available);
    assert!(rollback.token.is_some());
}

#[test]
fn apply_medium_risk_mode_matrix_rejects_start_without_explicit_unlock() {
    let daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::ApplyMediumRisk,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    let mut config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None)
        .with_simulated_action_effects();
    config.history_log = None;

    let mut runtime = AutotuneRuntime::new(config);
    assert_eq!(runtime.config().mode(), DaemonMode::ApplyMediumRisk);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    let decision = AutotuneDecision::StartExperiment {
        candidate: fake_mode_candidate(
            "mode-apply-medium-candidate",
            SafetyClass::ReversibleMediumRisk,
        ),
        reason: "mode matrix medium-risk start".to_owned(),
    };

    let err = runtime
        .apply_decision_side_effects(
            &observation,
            &decision,
            "apply-medium-risk rejection mode matrix",
        )
        .expect_err("medium-risk apply must require explicit unlock");

    assert!(
        err.to_string()
            .contains("live apply-medium-risk requires explicit medium-risk unlock"),
        "unexpected error: {err:#}"
    );

    assert!(!runtime.has_active_experiment());
    assert!(runtime.daemon_state_snapshot().active_rollback.is_none());
}
