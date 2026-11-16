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

mod research_action_gate {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{
        affinity::CpuMask,
        autotune::research::{
            ResearchAction,
            action_child::ActionChildScope,
            action_gate::{ActionGate, process_identity},
        },
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
    };

    fn profile() -> Profile {
        Profile {
            name: "research-gate".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    fn runtime_for(pid: u32, candidates: Vec<CandidateAction>) -> AutotuneRuntime {
        let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(pid), None)
            .with_simulated_candidates(candidates);
        config.history_log = None;
        let mut runtime = AutotuneRuntime::new(config);
        runtime.research_campaign_active = true;
        runtime
    }

    fn gate(actions: &[ResearchAction], pid: u32) -> Arc<Mutex<ActionGate>> {
        let target = process_identity(pid).expect("a live process");
        let until = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 600;
        Arc::new(Mutex::new(ActionGate::new(
            ActionChildScope::new(actions.iter().copied(), target, until),
            Box::new(|| Some(0)),
        )))
    }

    #[test]
    fn an_installed_gate_lets_an_authorized_game_action_through_and_nothing_else() {
        let pid = std::process::id();
        let cpu = CandidateAction::cpu_affinity_profile(profile(), pid);
        let fake = CandidateAction::fake(
            crate::actions::ActionId::new("not-a-game-action".to_owned()),
            SafetyClass::ReversibleLowRisk,
        );

        // No gate: nothing is ever permitted, the blanket suppression it always was.
        let mut blanket = runtime_for(pid, vec![cpu.clone()]);
        assert!(!blanket.research_gate_permits(&cpu));

        // A gate that authorizes this action on this process permits the candidate, once.
        let mut gated = runtime_for(pid, vec![cpu.clone()]);
        let shared = gate(&[ResearchAction::CpuAffinityProfile], pid);
        gated.set_research_action_gate(Some(shared.clone()));
        assert!(gated.research_gate_permits(&cpu));
        // One at a time: while it is outstanding, the same or another candidate is refused.
        assert!(!gated.research_gate_permits(&cpu));
        // The event ended with no experiment applied: the gate settles with nothing to restore and
        // the refused action is not retried.
        gated.observe_research_gate();
        let evidence = shared.lock().unwrap().evidence();
        assert!(evidence.clean && evidence.settled.is_empty());
        assert!(!gated.research_gate_permits(&cpu));

        // An action the campaign did not authorize, and a non-game candidate, stay suppressed.
        let mut other = runtime_for(pid, vec![cpu.clone()]);
        other.set_research_action_gate(Some(gate(&[ResearchAction::CgroupPlacement], pid)));
        assert!(!other.research_gate_permits(&cpu));
        let mut not_game = runtime_for(pid, vec![fake.clone()]);
        not_game.set_research_action_gate(Some(gate(&[ResearchAction::CpuAffinityProfile], pid)));
        assert!(!not_game.research_gate_permits(&fake));
    }

    #[test]
    fn a_gate_armed_for_another_process_never_permits_this_ones_candidate() {
        let pid = std::process::id();
        let cpu_again = CandidateAction::cpu_affinity_profile(profile(), pid);
        let mut runtime = runtime_for(pid, vec![cpu_again.clone()]);
        // The gate was frozen on a different process (pid 1 is never this test process).
        let stranger = process_identity(1).expect("init exists");
        let until = u64::MAX / 2;
        runtime.set_research_action_gate(Some(Arc::new(Mutex::new(ActionGate::new(
            ActionChildScope::new([ResearchAction::CpuAffinityProfile], stranger, until),
            Box::new(|| Some(0)),
        )))));
        assert!(!runtime.research_gate_permits(&cpu_again));
    }
}
