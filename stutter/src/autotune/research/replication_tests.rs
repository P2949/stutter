use serde_json::json;

use super::*;

fn report(arms: &[(&str, &str)]) -> Value {
    json!({ "arms": arms.iter()
        .map(|(source, verdict)| json!({ "source": source, "numeric_verdict": verdict }))
        .collect::<Vec<_>>() })
}

#[test]
fn the_same_verdict_twice_is_reproduced_and_a_different_one_is_a_contradiction() {
    let improved = report(&[("ge", "improved")]);
    assert_eq!(compare(&improved, &improved), Replication::Reproduced);
    // Both valid measurements, opposite conclusions: not two passes.
    assert_eq!(
        compare(&improved, &report(&[("ge", "regressed")])),
        Replication::Contradicted
    );
    assert_eq!(
        compare(&improved, &report(&[("ge", "equivalent")])),
        Replication::Contradicted
    );
    // A null result reproduces as a null result, in either of its two forms.
    let null = report(&[("ge", "equivalent")]);
    assert_eq!(compare(&null, &null), Replication::ReproducedNull);
    let none = report(&[("ge", "no_useful_effect")]);
    assert_eq!(compare(&none, &none), Replication::ReproducedNull);
    assert!(Replication::Reproduced.supports() && Replication::ReproducedNull.supports());
    assert!(!Replication::Contradicted.supports() && !Replication::Undetermined.supports());
}

#[test]
fn an_inconclusive_missing_or_unmatched_repetition_reproduces_nothing() {
    let improved = report(&[("ge", "improved")]);
    let inconclusive = report(&[("ge", "inconclusive")]);
    assert_eq!(
        compare(&inconclusive, &inconclusive),
        Replication::Undetermined
    );
    // A side with no numeric verdict at all.
    assert_eq!(
        compare(&improved, &json!({ "arms": [] })),
        Replication::Undetermined
    );
    assert_eq!(compare(&json!({}), &improved), Replication::Undetermined);
    // A build compared in the confirmation and absent from the holdout.
    assert_eq!(
        compare(&improved, &report(&[("em", "improved")])),
        Replication::Undetermined
    );
    // Builds that each agree with themselves but reached different kinds of conclusion.
    let mixed = report(&[("ge", "improved"), ("em", "equivalent")]);
    assert_eq!(compare(&mixed, &mixed), Replication::Undetermined);
    // One build contradicting hides behind no other agreeing one.
    let both = report(&[("ge", "improved"), ("em", "improved")]);
    let one_flipped = report(&[("ge", "improved"), ("em", "regressed")]);
    assert_eq!(compare(&both, &one_flipped), Replication::Contradicted);
}

#[test]
fn a_holdout_is_judged_against_the_confirmation_it_depends_on_in_the_plan() {
    use crate::autotune::research::{
        ResearchTool,
        child_study::{Profile, Stage, plan},
        tests::intent,
    };
    let mut campaign = intent();
    campaign.tools = [
        crate::autotune::research::ProtonSource::ValveStable,
        crate::autotune::research::ProtonSource::Ge,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, source)| ResearchTool {
        source,
        identity_sha256: format!("{index:x}").repeat(64)[..64].to_owned(),
    })
    .collect();
    let studies = plan(&campaign).unwrap();
    let holdout = studies
        .iter()
        .find(|s| s.stage == Stage::Holdout && s.profile == Profile::Cpu)
        .unwrap();
    let confirm = studies
        .iter()
        .find(|s| s.stage == Stage::Confirm && s.profile == Profile::Cpu)
        .unwrap();
    let (holdout_name, confirm_name) = (&holdout.study_sha256[..16], &confirm.study_sha256[..16]);
    let reports = |confirmation: &'static str, held: &'static str| {
        let (confirm_name, holdout_name) = (confirm_name.to_owned(), holdout_name.to_owned());
        move |name: &str| {
            if name == confirm_name {
                Some(report(&[("ge", confirmation)]))
            } else if name == holdout_name {
                Some(report(&[("ge", held)]))
            } else {
                None
            }
        }
    };
    let judge =
        |c: &'static str, h: &'static str| replication_of(&campaign, holdout_name, &reports(c, h));
    assert_eq!(judge("improved", "improved"), Some(Replication::Reproduced));
    assert_eq!(
        judge("improved", "regressed"),
        Some(Replication::Contradicted)
    );
    // Not a holdout at all: nothing to judge.
    assert_eq!(
        replication_of(&campaign, confirm_name, &reports("improved", "improved")),
        None
    );
    // The confirmation's report is unavailable: nothing is reproduced.
    assert_eq!(
        replication_of(&campaign, holdout_name, &|name: &str| {
            (name == holdout_name).then(|| report(&[("ge", "improved")]))
        }),
        Some(Replication::Undetermined)
    );
}
