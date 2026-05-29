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
