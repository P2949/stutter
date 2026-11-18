use super::*;

#[test]
fn a_qualified_pilot_freezes_the_rule_a_confirmation_will_be_bound_to() {
    let study = pilot();
    assert_eq!(study.min_attempts, 6, "three runs for each of two arms");
    let (mut exec, _) = executor(Ok(true), Ok(()), Ok(good(&study)));
    let outcome = run_to_end(&mut exec, &study).unwrap();
    assert_eq!(outcome.result, StudyResult::Qualified);
    let rule = outcome.rule.expect("a qualified pilot freezes a rule");
    assert!(rule.is_consistent());
    assert_eq!(
        rule.pilot_runs, 3,
        "the rule comes from one arm's three runs"
    );
    // Studies that are not pilots never carry a rule.
    let screen = cpu_screen();
    let (mut other, _) = executor(Ok(true), Ok(()), Ok(good(&screen)));
    assert_eq!(run_to_end(&mut other, &screen).unwrap().rule, None);
}

#[test]
fn a_three_arm_pilot_uses_all_nine_runs_then_freezes_the_selected_pair() {
    let study = pilot();
    let mut outcome = good(&study);
    outcome.source_order = vec![
        ProtonSource::ValveStable,
        ProtonSource::Ge,
        ProtonSource::Em,
    ];
    outcome.planned_attempts = 9;
    outcome.attempts_used = 9;
    outcome.completed = 9;
    outcome.conformance = vec![ProfileConformance::Conforms; 9];
    outcome.run_fps = vec![
        (ProtonSource::ValveStable, 60.0),
        (ProtonSource::ValveStable, 61.0),
        (ProtonSource::ValveStable, 59.0),
        (ProtonSource::Ge, 70.0),
        (ProtonSource::Ge, 71.0),
        (ProtonSource::Ge, 69.0),
        (ProtonSource::Em, 80.0),
        (ProtonSource::Em, 81.0),
        (ProtonSource::Em, 79.0),
    ];
    let expected: Vec<_> = outcome
        .run_fps
        .iter()
        .copied()
        .filter(|(source, _)| *source != ProtonSource::Ge)
        .collect();
    let expected_rule = crate::autotune::research::pilot_rule::rule_from_runs(&expected).unwrap();
    let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome.clone()));
    let result = run_to_end(&mut exec, &study).unwrap();
    assert_eq!(result.result, StudyResult::Qualified);
    assert_eq!(result.attempts_used, 9);
    assert_eq!(result.rule, Some(expected_rule));

    // If the preferred control exits without a result in all three pilot slots, the two fully
    // measured builds can still form a pair. The failures remain explicit and consume budget.
    outcome
        .run_fps
        .retain(|(source, _)| *source != ProtonSource::ValveStable);
    outcome.completed = 6;
    outcome.conformance.truncate(6);
    outcome.failures = vec!["ExitedWithoutNativeResult".into(); 3];
    let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome.clone()));
    assert_eq!(
        run_to_end(&mut exec, &study).unwrap().result,
        StudyResult::Qualified
    );
    outcome.failures[0] = "IdentityMismatch".into();
    let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
    assert_eq!(
        run_to_end(&mut exec, &study).unwrap().result,
        StudyResult::Failed
    );
}

#[test]
fn a_pilot_that_cannot_support_a_confirmation_is_unqualified_with_its_reason() {
    let study = pilot();
    let case = |mutate: &dyn Fn(&mut AttemptsOutcome)| {
        let mut outcome = good(&study);
        mutate(&mut outcome);
        let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
        let result = run_to_end(&mut exec, &study).unwrap();
        (result.result, result.reason, result.rule)
    };
    // One arm has only two runs.
    let thin = case(&|o| {
        let index = o
            .run_fps
            .iter()
            .position(|(arm, _)| *arm == ProtonSource::Ge)
            .unwrap();
        o.run_fps.remove(index);
    });
    assert_eq!(
        (thin.0, thin.1),
        (StudyResult::Failed, "IncompleteAttempts")
    );
    assert!(thin.2.is_none());
    // The runs are wildly noisy.
    let noisy = case(&|o| {
        for (index, (_, fps)) in o.run_fps.iter_mut().enumerate() {
            *fps = [40.0, 100.0, 70.0][index % 3];
        }
    });
    assert_eq!(
        (noisy.0, noisy.1),
        (StudyResult::Unqualified, "PilotTooNoisy")
    );
    // Only one arm produced runs.
    let single = case(&|o| {
        for (arm, _) in o.run_fps.iter_mut() {
            *arm = ProtonSource::Ge;
        }
    });
    assert_eq!(
        (single.0, single.1),
        (StudyResult::Unqualified, "PilotTooThin")
    );
    // A nonsense measurement.
    let invalid = case(&|o| o.run_fps[0].1 = f64::NAN);
    assert_eq!(
        (invalid.0, invalid.1),
        (StudyResult::Unqualified, "PilotInvalid")
    );
}

#[test]
fn an_observed_activation_is_not_a_functional_proof_and_the_reasons_differ() {
    use crate::autotune::research::ResearchFactor;
    let mut intent = base_intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.factors = vec![ResearchFactor::NtSync];
    let plan = plan(&intent).unwrap();
    let feature = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Feature(_)))
        .unwrap();
    let run_with = |activation: Option<bool>| {
        let mut outcome = good(feature);
        outcome.activation = activation;
        let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
        run_to_end(&mut exec, feature).unwrap()
    };
    assert_eq!(run_with(None).reason, "FeatureActivationUnproven");
    assert_eq!(run_with(Some(false)).reason, "FeatureActivationUnproven");
    // The treatment reached the game, but no receipt shows the feature works: still unqualified.
    let reached = run_with(Some(true));
    assert_eq!(
        (reached.result, reached.reason),
        (StudyResult::Unqualified, "FeatureFunctionalUnproven")
    );
}
