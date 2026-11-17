use super::*;

#[test]
fn a_feature_is_qualified_only_on_the_games_own_report_and_the_reasons_say_which_gap_remains() {
    use crate::autotune::research::{ResearchFactor, functional_probe::Functional};
    let mut intent = base_intent();
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.factors = vec![ResearchFactor::FrameGeneration];
    let plan = plan(&intent).unwrap();
    let feature = plan
        .iter()
        .find(|s| matches!(s.stage, Stage::Feature(_)))
        .unwrap();
    let run = |functional: Option<Functional>, stage_evidence: Option<bool>| {
        let mut outcome = good(feature);
        outcome.activation = Some(true);
        outcome.functional = functional;
        outcome.stage_evidence = stage_evidence;
        let (mut exec, _) = executor(Ok(true), Ok(()), Ok(outcome));
        run_to_end(&mut exec, feature).unwrap()
    };
    // The control reached the game and the game reported the feature working.
    assert_eq!(
        run(Some(Functional::Observed), Some(true)).result,
        StudyResult::Qualified
    );
    // Reached the game, but the game reported the feature off: not functional, said so.
    assert_eq!(
        run(Some(Functional::NotObserved), Some(false)).reason,
        "FeatureActivatedButNotFunctional"
    );
    // The game has no report for this factor.
    assert_eq!(
        run(Some(Functional::NotObservable), Some(false)).reason,
        "FeatureNotObservable"
    );
    assert_eq!(
        run(Some(Functional::Unproven), Some(false)).reason,
        "FeatureFunctionalUnproven"
    );
}
