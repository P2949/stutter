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
    let fps = measured(&[
        (ProtonSource::ValveStable, &[60.0]),
        (ProtonSource::Ge, &[70.0]),
        (ProtonSource::Em, &[65.0]),
    ]);
    assert_eq!(
        control_of(&with_valve, &fps),
        Some(ProtonSource::ValveStable)
    );
    let without = campaign(&[ProtonSource::Ge, ProtonSource::Em]);
    assert_eq!(control_of(&without, &fps), Some(ProtonSource::Ge));
}

#[test]
fn control_falls_back_to_a_measured_authorized_build() {
    let intent = campaign(&[
        ProtonSource::ValveStable,
        ProtonSource::Ge,
        ProtonSource::ValveExperimental,
        ProtonSource::Em,
    ]);
    let fps = measured(&[
        (ProtonSource::ValveStable, &[f64::NAN]),
        (ProtonSource::Ge, &[70.0]),
        (ProtonSource::ValveExperimental, &[60.0]),
        (ProtonSource::Em, &[80.0]),
    ]);
    assert_eq!(
        select_pair(&intent, &fps),
        Some([ProtonSource::ValveExperimental, ProtonSource::Em])
    );
    let no_valve = measured(&[(ProtonSource::Ge, &[70.0]), (ProtonSource::Em, &[80.0])]);
    assert_eq!(
        select_pair(&intent, &no_valve),
        Some([ProtonSource::Ge, ProtonSource::Em])
    );
    let invalid = measured(&[
        (ProtonSource::Ge, &[70.0, -1.0]),
        (ProtonSource::Em, &[80.0]),
    ]);
    assert_eq!(select_pair(&intent, &invalid), None);
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
            (ProtonSource::ValveStable, &[60.0, 61.0, 60.0]),
            (ProtonSource::Ge, &[70.0, 71.0, 70.0]),
            (ProtonSource::Em, &[65.0, 66.0, 65.0]),
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
        assert_eq!(
            pair.arms,
            [
                ProtonSource::ValveStable,
                ProtonSource::Ge,
                ProtonSource::Em
            ]
        );
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

    #[test]
    fn screen_values_do_not_choose_the_confirmation_challenger() {
        let intent = campaign(&[
            ProtonSource::ValveStable,
            ProtonSource::Ge,
            ProtonSource::Em,
        ]);
        let pilot = study_of(&intent, |s| matches!(s, Stage::Pilot));
        let screen = measured(&[
            (ProtonSource::ValveStable, &[60.0]),
            (ProtonSource::Ge, &[100.0]),
            (ProtonSource::Em, &[80.0]),
        ]);
        let inputs = inputs_for_study(&intent, &pilot, &screen, None).unwrap();
        assert_eq!(
            inputs.arms,
            [
                ProtonSource::ValveStable,
                ProtonSource::Ge,
                ProtonSource::Em
            ]
        );
        let confirm = study_of(&intent, |s| matches!(s, Stage::Confirm));
        assert!(
            inputs_for_study(
                &intent,
                &confirm,
                &screen,
                Some(freeze(&[70.0, 73.0, 67.0]).unwrap())
            )
            .is_err()
        );
        let repeated = measured(&[
            (ProtonSource::ValveStable, &[60.0, 60.0, 60.0]),
            (ProtonSource::Ge, &[65.0, 65.0, 65.0]),
            (ProtonSource::Em, &[80.0, 80.0, 80.0]),
        ]);
        let chosen = inputs_for_study(
            &intent,
            &confirm,
            &repeated,
            Some(freeze(&[70.0, 73.0, 67.0]).unwrap()),
        )
        .unwrap();
        assert_eq!(chosen.arms, [ProtonSource::ValveStable, ProtonSource::Em]);
    }

    #[test]
    fn matched_cachyos_comparison_uses_its_own_two_piloted_builds_and_rule() {
        let intent = campaign(&[
            ProtonSource::ValveStable,
            ProtonSource::CachyOs,
            ProtonSource::CachyOsV3,
            ProtonSource::Ge,
        ]);
        let matched = study_of(&intent, |stage| *stage == Stage::MatchedCachyOs);
        let fps = measured(&[
            (ProtonSource::ValveStable, &[60.0, 60.0, 60.0]),
            (ProtonSource::CachyOs, &[70.0, 71.0, 69.0]),
            (ProtonSource::CachyOsV3, &[72.0, 73.0, 71.0]),
            (ProtonSource::Ge, &[100.0, 100.0, 100.0]),
        ]);
        let inputs = inputs_for_study(&intent, &matched, &fps, None).unwrap();
        assert_eq!(
            inputs.arms,
            [ProtonSource::CachyOs, ProtonSource::CachyOsV3]
        );
        let expected = [
            (ProtonSource::CachyOs, 70.0),
            (ProtonSource::CachyOs, 71.0),
            (ProtonSource::CachyOs, 69.0),
            (ProtonSource::CachyOsV3, 72.0),
            (ProtonSource::CachyOsV3, 73.0),
            (ProtonSource::CachyOsV3, 71.0),
        ];
        assert_eq!(
            inputs.rule,
            Some(crate::autotune::research::pilot_rule::rule_from_runs(&expected).unwrap())
        );
        // Under a pilot that froze the tail metric, the matched rule carries it too.
        let tail_pilot = freeze(&[70.0, 73.0, 67.0]).unwrap().with_tail_metric(true);
        let tail = inputs_for_study(&intent, &matched, &fps, Some(tail_pilot)).unwrap();
        assert_eq!(
            tail.rule,
            Some(
                crate::autotune::research::pilot_rule::rule_from_runs(&expected)
                    .unwrap()
                    .with_tail_metric(true)
            )
        );
        assert!(tail.rule.unwrap().is_consistent());
        let mut missing = fps;
        missing.remove(&ProtonSource::CachyOsV3);
        assert!(inputs_for_study(&intent, &matched, &missing, None).is_err());
    }

    #[test]
    fn an_unavailable_screen_blocks_a_pair_study_before_any_driver_starts() {
        use std::os::unix::fs::PermissionsExt;
        let intent = campaign(&[ProtonSource::ValveStable, ProtonSource::Ge]);
        let pilot = study_of(&intent, |s| matches!(s, Stage::Pilot));
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let blocker = crate::autotune::research::funnel_driver::selection_blocker(
            std::fs::File::open(root.path()).unwrap(),
            intent,
            Default::default(),
        );
        assert_eq!(blocker(&pilot), Some("BlockedScreenUnavailable"));
        assert!(!root.path().join("studies").exists());
    }
}

mod treatment_source {
    use super::*;
    use crate::autotune::research::{
        ResearchFactor,
        child_study::Stage,
        funnel_driver::{NoEligibleSource, inputs_for_study},
    };

    const SIX: [ProtonSource; 6] = [
        ProtonSource::ValveStable,
        ProtonSource::ValveExperimental,
        ProtonSource::CachyOs,
        ProtonSource::CachyOsV3,
        ProtonSource::Ge,
        ProtonSource::Em,
    ];

    fn six() -> ResearchIntentV1 {
        let mut intent = campaign(&SIX);
        intent.factors = vec![
            ResearchFactor::NtSync,
            ResearchFactor::Fsr4,
            ResearchFactor::Gamescope,
        ];
        intent.max_attempts = 120;
        intent
    }

    fn feature(intent: &ResearchIntentV1, factor: ResearchFactor) -> ChildStudy {
        plan(intent)
            .unwrap()
            .into_iter()
            .find(|s| s.stage == Stage::Feature(factor))
            .unwrap()
    }

    /// Valve Stable is the fastest and the control, GE next.
    fn fps() -> BTreeMap<ProtonSource, Vec<f64>> {
        measured(&[
            (ProtonSource::ValveStable, &[80.0, 81.0, 80.0]),
            (ProtonSource::ValveExperimental, &[70.0, 71.0, 70.0]),
            (ProtonSource::CachyOs, &[72.0, 73.0, 72.0]),
            (ProtonSource::CachyOsV3, &[74.0, 75.0, 74.0]),
            (ProtonSource::Ge, &[76.0, 77.0, 76.0]),
            (ProtonSource::Em, &[60.0, 61.0, 60.0]),
        ])
    }

    #[test]
    fn eligibility_is_the_factors_own_environment_control_not_the_performance_control() {
        assert!(!eligible_for(
            ProtonSource::ValveStable,
            ResearchFactor::NtSync
        ));
        assert!(eligible_for(ProtonSource::Ge, ResearchFactor::NtSync));
        assert!(eligible_for(ProtonSource::Ge, ResearchFactor::Fsr4));
        assert!(!eligible_for(ProtonSource::CachyOs, ResearchFactor::Fsr4));
        assert!(!eligible_for(ProtonSource::Ge, ResearchFactor::Gamescope));
    }

    #[test]
    fn a_fork_feature_is_measured_on_the_best_eligible_build_never_on_valve_stable() {
        let intent = six();
        // NtSync: every custom build can; the fastest of them (GE), although Valve Stable is faster.
        let nt =
            select_treatment_source(&intent, &feature(&intent, ResearchFactor::NtSync), &fps());
        assert_eq!(nt, Some(ProtonSource::Ge));
        // FSR4 only exists on GE.
        let fsr = select_treatment_source(&intent, &feature(&intent, ResearchFactor::Fsr4), &fps());
        assert_eq!(fsr, Some(ProtonSource::Ge));
        // An unmeasured eligible build is skipped; ties follow the campaign's tool order.
        let mut partial = fps();
        partial.remove(&ProtonSource::Ge);
        let nt =
            select_treatment_source(&intent, &feature(&intent, ResearchFactor::NtSync), &partial);
        assert_eq!(nt, Some(ProtonSource::CachyOsV3));
    }

    #[test]
    fn an_action_study_uses_the_measured_control_and_a_factor_nothing_controls_has_no_source() {
        let intent = six();
        let action = plan(&intent)
            .unwrap()
            .into_iter()
            .find(|s| matches!(s.stage, Stage::Action(_)))
            .unwrap();
        assert_eq!(
            select_treatment_source(&intent, &action, &fps()),
            Some(ProtonSource::ValveStable)
        );
        assert_eq!(
            select_treatment_source(&intent, &action, &BTreeMap::new()),
            None
        );
        let mut fallback = fps();
        fallback.remove(&ProtonSource::ValveStable);
        assert_eq!(
            select_treatment_source(&intent, &action, &fallback),
            Some(ProtonSource::ValveExperimental)
        );
        // Gamescope is not a Proton environment factor: no build can run it.
        let gamescope = feature(&intent, ResearchFactor::Gamescope);
        assert_eq!(select_treatment_source(&intent, &gamescope, &fps()), None);
        let error = inputs_for_study(&intent, &gamescope, &fps(), None)
            .err()
            .expect("no source");
        assert!(error.downcast_ref::<NoEligibleSource>().is_some());
    }
}
