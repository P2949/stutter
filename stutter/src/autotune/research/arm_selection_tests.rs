use super::*;
use crate::autotune::research::{ResearchAction, ResearchTool, child_study::plan, tests::intent};

fn campaign(tools: &[ProtonSource]) -> ResearchIntentV1 {
    let mut intent = intent();
    intent.tools = tools
        .iter()
        .enumerate()
        .map(|(index, source)| ResearchTool {
            source: *source,
            identity_sha256: format!("{index:x}").repeat(64)[..64].to_owned(),
        })
        .collect();
    intent.actions = vec![ResearchAction::CpuAffinityProfile];
    intent
}

fn measured(rows: &[(ProtonSource, &[f64])]) -> BTreeMap<ProtonSource, Vec<f64>> {
    rows.iter()
        .map(|(arm, runs)| (*arm, runs.to_vec()))
        .collect()
}

#[test]
fn the_control_is_valves_stable_build_or_else_the_first_tool() {
    let with_valve = campaign(&[
        ProtonSource::Ge,
        ProtonSource::ValveStable,
        ProtonSource::Em,
    ]);
    assert_eq!(control_of(&with_valve), Some(ProtonSource::ValveStable));
    let without = campaign(&[ProtonSource::Ge, ProtonSource::Em]);
    assert_eq!(control_of(&without), Some(ProtonSource::Ge));
}

#[test]
fn the_candidate_is_the_best_other_arm_by_mean_fps_and_ties_follow_the_campaigns_tool_order() {
    let intent = campaign(&[
        ProtonSource::ValveStable,
        ProtonSource::Ge,
        ProtonSource::Em,
        ProtonSource::CachyOs,
    ]);
    let fps = measured(&[
        (ProtonSource::ValveStable, &[60.0, 61.0]),
        (ProtonSource::Ge, &[70.0, 71.0]),
        (ProtonSource::Em, &[75.0, 72.0]),
        (ProtonSource::CachyOs, &[70.0, 71.0]),
    ]);
    assert_eq!(
        select_pair(&intent, &fps),
        Some([ProtonSource::ValveStable, ProtonSource::Em])
    );
    // A tie goes to the arm that comes first in the campaign's own tool order.
    let tied = measured(&[
        (ProtonSource::ValveStable, &[60.0]),
        (ProtonSource::Ge, &[70.0]),
        (ProtonSource::Em, &[70.0]),
    ]);
    assert_eq!(
        select_pair(&intent, &tied),
        Some([ProtonSource::ValveStable, ProtonSource::Ge])
    );
}

#[test]
fn no_pair_exists_without_a_measured_control_and_a_measured_candidate() {
    let intent = campaign(&[
        ProtonSource::ValveStable,
        ProtonSource::Ge,
        ProtonSource::Em,
    ]);
    // No control measurement.
    assert_eq!(
        select_pair(&intent, &measured(&[(ProtonSource::Ge, &[70.0])])),
        None
    );
    // No candidate measured, or only a nonsense one.
    assert_eq!(
        select_pair(&intent, &measured(&[(ProtonSource::ValveStable, &[60.0])])),
        None
    );
    assert_eq!(
        select_pair(
            &intent,
            &measured(&[
                (ProtonSource::ValveStable, &[60.0]),
                (ProtonSource::Ge, &[f64::NAN]),
                (ProtonSource::Em, &[]),
            ])
        ),
        None
    );
}

#[test]
fn every_later_study_descends_from_the_screen_of_its_own_profile_and_actions_use_the_cpu_screen() {
    let intent = campaign(&[ProtonSource::ValveStable, ProtonSource::Ge]);
    let plan = plan(&intent).unwrap();
    for study in &plan {
        let screen = screen_of(&intent, study).unwrap();
        assert_eq!(screen.stage, Stage::Screen);
        let expected = match study.stage {
            Stage::Action(_) | Stage::Feature(_) => Profile::Cpu,
            _ => study.profile,
        };
        assert_eq!(screen.profile, expected);
    }
}

mod inputs {
    use super::*;
    use crate::autotune::research::{
        child_study::Stage, funnel_driver::inputs_for_study, pilot_rule::freeze,
    };

    fn study_of(intent: &ResearchIntentV1, stage: fn(&Stage) -> bool) -> ChildStudy {
        plan(intent)
            .unwrap()
            .into_iter()
            .find(|s| stage(&s.stage))
            .unwrap()
    }

    fn screen_fps() -> BTreeMap<ProtonSource, Vec<f64>> {
        measured(&[
            (ProtonSource::ValveStable, &[60.0, 61.0]),
            (ProtonSource::Ge, &[70.0, 71.0]),
            (ProtonSource::Em, &[65.0, 66.0]),
        ])
    }

    #[test]
    fn a_screen_runs_across_every_tool_and_the_others_across_the_chosen_pair() {
        let intent = campaign(&[
            ProtonSource::ValveStable,
            ProtonSource::Ge,
            ProtonSource::Em,
        ]);
        let screen = study_of(&intent, |s| matches!(s, Stage::Screen));
        let all = inputs_for_study(&intent, &screen, &BTreeMap::new(), None).unwrap();
        assert_eq!(all.arms.len(), 3);
        assert!(all.rule.is_none());
        let pilot = study_of(&intent, |s| matches!(s, Stage::Pilot));
        let pair = inputs_for_study(&intent, &pilot, &screen_fps(), None).unwrap();
        assert_eq!(pair.arms, [ProtonSource::ValveStable, ProtonSource::Ge]);
        assert!(pair.rule.is_none(), "only a confirmation carries a rule");
    }

    #[test]
    fn a_confirmation_needs_its_pilots_rule_and_a_measured_pair() {
        let intent = campaign(&[
            ProtonSource::ValveStable,
            ProtonSource::Ge,
            ProtonSource::Em,
        ]);
        let confirm = study_of(&intent, |s| matches!(s, Stage::Confirm));
        let rule = freeze(&[70.0, 73.0, 67.0]).unwrap();
        let inputs =
            inputs_for_study(&intent, &confirm, &screen_fps(), Some(rule.clone())).unwrap();
        assert_eq!(inputs.rule, Some(rule.clone()));
        assert!(inputs_for_study(&intent, &confirm, &screen_fps(), None).is_err());
        assert!(inputs_for_study(&intent, &confirm, &BTreeMap::new(), Some(rule)).is_err());
    }
}
