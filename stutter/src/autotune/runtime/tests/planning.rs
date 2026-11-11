use super::*;

#[test]
fn top_denied_reason_for_plan_prefers_deny_reason_enum() {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-noop".to_owned()),
        SafetyClass::ObserveOnly,
    );
    let descriptor = candidate.descriptor();
    let evaluation = crate::autotune::planner::CandidateEvaluation {
        candidate_name: "fake-noop".to_owned(),
        action_kind: "fake".to_owned(),
        descriptor,
        provider: "test".to_owned(),
        confidence: 1.0,
        eligible: false,
        deny_reasons: vec![crate::autotune::planner::CandidateDenyReason::NoEffectiveChange],
        deny_messages: vec!["candidate would not change active configuration".to_owned()],
        evidence: Vec::new(),
        objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
        rank: Some(1),
        dry_run: None,
        candidate,
    };
    let plan = PlanResult {
        selected: None,
        evaluations: vec![evaluation],
        no_action_reason: None,
    };

    assert_eq!(
        top_denied_reason_for_plan(&plan).as_deref(),
        Some("NoEffectiveChange")
    );
}

#[test]
fn admitted_research_reserves_the_action_slot_but_retains_candidate_diagnostics() {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("research-slot-candidate".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
        .with_simulated_candidates(vec![candidate]);
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    assert!(
        runtime
            .select_candidate_for_normal_control(&observation)
            .unwrap()
            .is_some()
    );
    runtime.research_campaign_active = true;
    let selected = runtime
        .select_candidate_for_normal_control(&observation)
        .unwrap();
    assert!(selected.is_none());
    assert!(
        runtime
            .last_plan_result
            .as_ref()
            .unwrap()
            .selected
            .is_some()
    );
    let decision = decide_autotune_transition(
        &runtime.controller.policy,
        &runtime.controller.state,
        &observation,
        selected,
    );
    assert!(!matches!(
        decision,
        AutotuneDecision::StartExperiment { .. }
    ));
    runtime
        .apply_decision_side_effects(&observation, &decision, "research owns action slot")
        .unwrap();
    assert!(!runtime.has_active_experiment());
    runtime.research_campaign_active = false;
    assert!(
        runtime
            .select_candidate_for_normal_control(&observation)
            .unwrap()
            .is_some()
    );
}
