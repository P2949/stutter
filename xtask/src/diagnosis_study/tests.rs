use serde_json::json;

use super::{
    impact::evaluate,
    model::StudyConfig,
    plan::{Action, Slot, next_action, pair_order},
    store::{Decision, Outcome, Role, RunRecord},
};
use crate::validation_campaign::scenarios::model::{ControlType, Family};

const TEXT: &str = include_str!("../../../docs/validation/scenarios/diagnosis-v2.toml");

fn config() -> StudyConfig {
    StudyConfig::parse(TEXT).unwrap()
}

fn run(
    scenario: &str,
    family: Option<Family>,
    role: Role,
    pair: usize,
    ok: bool,
    v: f64,
) -> RunRecord {
    RunRecord {
        seq: 0,
        scenario: scenario.to_owned(),
        family,
        role,
        pair,
        accepted: ok,
        stage: (!ok).then(|| "acceptance".to_owned()),
        reasons: Vec::new(),
        capture_id: ok.then(|| format!("sc-{scenario}-{pair}")),
        fixture: None,
        metrics: json!({"primary": v}),
        unix_ms: 0,
    }
}

fn decision(family: Family, pairs: usize, outcome: Outcome) -> Decision {
    Decision {
        family,
        pairs,
        outcome,
        report_sha256: "0".repeat(64),
        unix_ms: 0,
    }
}

/// Accepted control/positive runs for `pairs` pairs of `family` with the given metric values.
fn pairs_of(family: Family, pairs: usize, control: f64, positive: f64) -> Vec<RunRecord> {
    let mut runs = Vec::new();
    for pair in 0..pairs {
        // A little deterministic spread so the series are not constant.
        let wobble = (pair % 3) as f64 * 0.01 * control.max(1.0);
        runs.push(run(
            &format!("v2-{}-control", family.as_str()),
            Some(family),
            Role::Control,
            pair,
            true,
            control + wobble,
        ));
        runs.push(run(
            &format!("v2-{}-positive", family.as_str()),
            Some(family),
            Role::Positive,
            pair,
            true,
            positive + wobble,
        ));
    }
    runs
}

#[test]
fn the_committed_study_is_valid_and_matched() {
    let config = config();
    for def in &config.families {
        let (control, positive) = (config.control_spec(def), config.positive_spec(def));
        assert_eq!(control.target, positive.target, "{:?}", def.family);
        assert_eq!(control.placement, positive.placement);
        assert_eq!(control.control, ControlType::NegativeControl);
        assert!(control.interventions.is_empty() && !positive.interventions.is_empty());
        assert!(control.informational && positive.informational);
    }
    // Matched pairs tolerate the same background noise, and saturating scenarios declare more.
    for def in &config.families {
        let (control, positive) = (config.control_spec(def), config.positive_spec(def));
        assert_eq!(
            control.accepted_wakeup_coalescing,
            positive.accepted_wakeup_coalescing
        );
        assert_eq!(
            control.accepted_block_collisions,
            positive.accepted_block_collisions
        );
        assert!(
            control.accepted_wakeup_coalescing >= 16 && control.accepted_block_collisions >= 16
        );
    }
    assert_eq!(
        config
            .positive_spec(config.family(Family::Io).unwrap())
            .accepted_block_collisions,
        100_000
    );
    // The GPU control is the heavy target, never the light clean scene.
    let gpu = config.family(Family::Gpu).unwrap();
    assert_eq!(config.control_spec(gpu).target, gpu.target);
    assert_eq!(config.mixed.len(), 2);
}

#[test]
fn pairs_are_counterbalanced_not_blocked() {
    let orders: Vec<_> = (0..4).map(pair_order).collect();
    assert_eq!(orders[0], [Role::Control, Role::Positive]);
    assert_eq!(orders[1], [Role::Positive, Role::Control]);
    assert_eq!(orders[2], orders[0]);
    assert_eq!(orders[3], orders[1]);
    let config = config();
    let mut runs = Vec::new();
    let mut seen = Vec::new();
    // Take the next 4 captures, accepting each, and record their roles in order.
    for _ in 0..4 {
        let Action::Capture(Slot {
            scenario,
            role,
            pair,
            ..
        }) = next_action(&config, &runs, &[])
        else {
            panic!("expected a capture")
        };
        seen.push((role, pair));
        runs.push(run(
            &scenario,
            Some(Family::Scheduler),
            role,
            pair,
            true,
            1.0,
        ));
    }
    assert_eq!(
        seen,
        vec![
            (Role::Control, 0),
            (Role::Positive, 0),
            (Role::Positive, 1),
            (Role::Control, 1)
        ]
    );
}

#[test]
fn a_demonstrated_effect_passes_and_unlocks_the_next_family() {
    let config = config();
    let def = config.family(Family::Scheduler).unwrap();
    let runs = pairs_of(Family::Scheduler, 15, 1_000_000.0, 3_000_000.0);
    assert_eq!(
        next_action(&config, &runs, &[]),
        Action::Evaluate {
            family: Family::Scheduler,
            pairs: 15
        }
    );
    let evaluation = evaluate(&config, def, &runs, 15);
    assert_eq!(evaluation.outcome, Outcome::Pass);
    assert_eq!(evaluation.report["worsening_established"], true);
    assert_eq!(evaluation.report_sha256.len(), 64);
    let decisions = [decision(Family::Scheduler, 15, Outcome::Pass)];
    let Action::Capture(slot) = next_action(&config, &runs, &decisions) else {
        panic!("the io family follows")
    };
    assert_eq!(slot.scenario, "v2-io-control");
}

#[test]
fn a_missing_effect_extends_to_the_cap_then_closes_the_family_as_invalid() {
    let config = config();
    let def = config.family(Family::Gpu).unwrap();
    // No difference at all between control and positive.
    let runs = pairs_of(Family::Gpu, 15, 10.0, 10.0);
    let first = evaluate(&config, def, &runs, 15);
    assert_eq!(first.outcome, Outcome::Extend);
    let more = pairs_of(Family::Gpu, 30, 10.0, 10.0);
    assert_eq!(evaluate(&config, def, &more, 25).outcome, Outcome::Extend);
    let last = evaluate(&config, def, &more, 30);
    assert_eq!(last.outcome, Outcome::Invalid);
    assert_eq!(last.report["worsening_established"], false);
}

#[test]
fn an_improving_intervention_is_never_a_demonstrated_worsening() {
    let config = config();
    let def = config.family(Family::Irq).unwrap();
    let runs = pairs_of(Family::Irq, 30, 20.0, 10.0);
    assert_eq!(evaluate(&config, def, &runs, 30).outcome, Outcome::Invalid);
}

#[test]
fn extension_continues_the_same_family_with_more_pairs() {
    let config = config();
    let runs = pairs_of(Family::Scheduler, 15, 1.0, 1.0);
    let decisions = [decision(Family::Scheduler, 15, Outcome::Extend)];
    let Action::Capture(slot) = next_action(&config, &runs, &decisions) else {
        panic!("capture pair 15")
    };
    assert_eq!((slot.family, slot.pair), (Some(Family::Scheduler), 15));
}

#[test]
fn a_slot_that_spends_its_failure_budget_blocks_the_study() {
    let config = config();
    let runs: Vec<_> = (0..3)
        .map(|_| {
            run(
                "v2-scheduler-control",
                Some(Family::Scheduler),
                Role::Control,
                0,
                false,
                0.0,
            )
        })
        .collect();
    assert!(matches!(
        next_action(&config, &runs, &[]),
        Action::Blocked(_)
    ));
    let two: Vec<_> = runs.into_iter().take(2).collect();
    assert!(matches!(
        next_action(&config, &two, &[]),
        Action::Capture(_)
    ));
}

#[test]
fn mixed_captures_wait_for_valid_families_and_skip_invalid_ones() {
    let config = config();
    let mut decisions: Vec<_> = Family::ALL
        .iter()
        .map(|f| decision(*f, 15, Outcome::Pass))
        .collect();
    let Action::Capture(slot) = next_action(&config, &[], &decisions) else {
        panic!("mixed captures start once every family is closed")
    };
    assert_eq!(slot.role, Role::Mixed);
    assert_eq!(slot.scenario, "mixed-scheduler-io");
    // An invalid GPU family removes the IRQ+GPU mixed captures only.
    decisions[3] = decision(Family::Gpu, 30, Outcome::Invalid);
    let done: Vec<_> = (0..6)
        .map(|i| run("mixed-scheduler-io", None, Role::Mixed, i, true, 0.0))
        .collect();
    assert_eq!(next_action(&config, &done, &decisions), Action::Done);
}

#[test]
fn malformed_studies_are_rejected() {
    let bad = TEXT.replace("initial_pairs = 15", "initial_pairs = 3");
    assert!(StudyConfig::parse(&bad).is_err());
    let missing = TEXT.replace("family = \"gpu\"", "family = \"scheduler\"");
    assert!(StudyConfig::parse(&missing).is_err());
    let mismatch = TEXT.replace(
        "intervention = \"gpu_stressor\"\nmetric",
        "intervention = \"blocking_io\"\nmetric",
    );
    assert!(StudyConfig::parse(&mismatch).is_err());
}

mod promotion {
    use std::fs;

    use super::super::{
        promote::{annotate, promote},
        store::{Role, RunRecord, StudyDir},
    };
    use crate::{
        diagnosis_calibration::load_corpus, validation_campaign::scenarios::model::Family,
    };

    const FIXTURE: &str = include_str!(
        "../../../stutter/tests/fixtures/runs/real_clean_vkmark_0af7e274/fixture.toml"
    );

    #[test]
    fn annotation_keeps_the_fixture_eligible_and_adds_the_study_links() {
        let extra = [
            ("study", "diagnosis-v2".to_owned()),
            ("matched_group", "scheduler-pair-3".to_owned()),
            ("target_impact_report_sha256", "a".repeat(64)),
        ];
        let text = annotate(FIXTURE, &extra).unwrap();
        let dir = std::env::temp_dir().join(format!("stutter-annotate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let fixture = dir.join("real_x");
        fs::create_dir_all(&fixture).unwrap();
        fs::write(fixture.join("fixture.toml"), &text).unwrap();
        let (fixtures, _) = load_corpus(&dir).unwrap();
        let meta = fixtures[0]
            .meta
            .as_ref()
            .expect("still calibration-eligible");
        assert_eq!(meta.control_type, "negative_control");
        assert!(text.contains("matched_group = \"scheduler-pair-3\""));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn promotion_links_files_rewrites_only_the_copy_and_is_idempotent() {
        let root = std::env::temp_dir().join(format!("stutter-promote-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let study = StudyDir(root.clone());
        let original = study.fixtures().join("real_v2_x");
        fs::create_dir_all(&original).unwrap();
        fs::write(original.join("fixture.toml"), FIXTURE).unwrap();
        fs::write(original.join("interval.json"), "{}\n").unwrap();
        let run = RunRecord {
            seq: 1,
            scenario: "v2-scheduler-control".into(),
            family: Some(Family::Scheduler),
            role: Role::Control,
            pair: 0,
            accepted: true,
            stage: None,
            reasons: vec![],
            capture_id: Some("sc-x".into()),
            fixture: Some("real_v2_x".into()),
            metrics: serde_json::json!({"primary": 1.0, "supporting": {"placement": {"cpu": 1}}}),
            unix_ms: 0,
        };
        promote(&study, "v2-test", &run, "scheduler-pair-0", &"b".repeat(64)).unwrap();
        promote(&study, "v2-test", &run, "scheduler-pair-0", &"b".repeat(64)).unwrap();
        let promoted = study.calibration().join("fixtures/real_v2_x");
        assert!(
            fs::read_to_string(promoted.join("fixture.toml"))
                .unwrap()
                .contains("study_id")
        );
        assert!(
            !fs::read_to_string(original.join("fixture.toml"))
                .unwrap()
                .contains("study_id"),
            "the acceptance-time fixture is never modified"
        );
        assert_eq!(
            fs::read_to_string(promoted.join("interval.json")).unwrap(),
            "{}\n"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

mod calibration {
    use serde_json::json;

    use super::super::calibrate::{Decision, Part, Row, build_split, decide};
    use crate::diagnosis_calibration::parse_fixture;

    fn row(t: f64, tp: u64, fp: u64, fn_: u64, fh: u64) -> Row {
        let p = (tp + fp > 0).then(|| tp as f64 / (tp + fp) as f64);
        let r = (tp + fn_ > 0).then(|| tp as f64 / (tp + fn_) as f64);
        Row {
            threshold: t,
            tp,
            fp,
            fn_,
            precision: p,
            recall: r,
            false_high_observation: fh,
            false_high_causal: 0,
        }
    }

    fn fixture(id: &str, scenario: &str) -> crate::diagnosis_calibration::Fixture {
        let keys = crate::fixture_coverage::REQUIRED_PROVENANCE
            .iter()
            .filter(|k| !matches!(**k, "control_type" | "capture_id" | "scenario"))
            .map(|k| format!("{k} = \"x\"\n"))
            .collect::<String>();
        let text = format!(
            "name = \"f_{id}\"\nevidence_kind = \"fixture_real\"\n[expected]\nprimary_cause = \"GameThreadSchedulerDelay\"\n[provenance]\n{keys}control_type = \"positive\"\ncapture_id = \"{id}\"\nscenario = \"{scenario}\"\n[coverage]\ndiagnoses = [\"GameThreadSchedulerDelay\"]\n"
        );
        parse_fixture(std::path::Path::new("d"), &text)
    }

    #[test]
    fn the_split_is_deterministic_stratified_and_balanced() {
        let fixtures: Vec<_> = (0..15)
            .map(|i| fixture(&format!("a-{i}"), "v2-scheduler-positive"))
            .chain((0..14).map(|i| fixture(&format!("b-{i}"), "v2-scheduler-control")))
            .collect();
        let first = build_split(&fixtures);
        assert_eq!(first, build_split(&fixtures));
        let reversed: Vec<_> = fixtures.iter().rev().cloned().collect();
        let mut again = build_split(&reversed);
        let mut original = first.clone();
        again.sort_by(|a, b| a.capture_id.cmp(&b.capture_id));
        original.sort_by(|a, b| a.capture_id.cmp(&b.capture_id));
        assert_eq!(again, original, "the split does not depend on input order");
        for scenario in ["v2-scheduler-positive", "v2-scheduler-control"] {
            let of = |part| {
                first
                    .iter()
                    .filter(|e| e.scenario == scenario && e.part == part)
                    .count()
            };
            let (cal, hold) = (of(Part::Calibration), of(Part::Holdout));
            assert!(
                cal.abs_diff(hold) <= 1 && cal + hold > 0,
                "{scenario}: {cal}/{hold}"
            );
        }
    }

    #[test]
    fn a_threshold_is_replaced_only_when_the_holdout_agrees() {
        let none = json!({});
        // Calibration prefers 1.0 (better recall, same precision); the holdout confirms.
        let calibration = [row(2.0, 3, 0, 7, 0), row(1.0, 6, 0, 4, 0)];
        let holdout = [row(2.0, 2, 0, 8, 0), row(1.0, 5, 0, 5, 0)];
        let decision = decide(2.0, &calibration, &holdout, &none, &none);
        assert!(decision.changed && decision.selected == 1.0, "{decision:?}");
        // The holdout shows more false positives: retain.
        let worse = [row(2.0, 2, 0, 8, 0), row(1.0, 5, 3, 5, 0)];
        let decision = decide(2.0, &calibration, &worse, &none, &none);
        assert!(
            !decision.changed && decision.selected == 2.0,
            "{decision:?}"
        );
        assert!(
            decision
                .reasons
                .iter()
                .any(|r| r.contains("does not generalize"))
        );
        // A candidate that adds a false-high claim is not even considered.
        let risky = [row(2.0, 3, 0, 7, 0), row(1.0, 9, 0, 1, 2)];
        let decision = decide(2.0, &risky, &holdout, &none, &none);
        assert!(!decision.changed, "{decision:?}");
    }

    #[test]
    fn mixed_behavior_can_veto_a_change_and_an_empty_sweep_is_a_logic_gap() {
        let calibration = [row(2.0, 3, 0, 7, 0), row(1.0, 6, 0, 4, 0)];
        let holdout = [row(2.0, 2, 0, 8, 0), row(1.0, 5, 0, 5, 0)];
        let mixed = json!({"mixed_sweep": [
            {"threshold": 2.0, "mixed_false_high_causal_claims": 0},
            {"threshold": 1.0, "mixed_false_high_causal_claims": 1}]});
        let decision = decide(2.0, &calibration, &holdout, &json!({}), &mixed);
        assert!(!decision.changed, "{decision:?}");
        let blind = [row(2.0, 0, 0, 10, 0), row(1.0, 0, 1, 10, 0)];
        let Decision {
            logic_gap, changed, ..
        } = decide(2.0, &blind, &blind, &json!({}), &json!({}));
        assert!(logic_gap && !changed);
    }
}

mod continuation {
    use std::fs;

    use super::{
        super::{
            facts::choose_traffic_cpus,
            import::{import_families, same_definition},
            model::StudyConfig,
            store::{Decision, Outcome, Role, RunRecord, StudyDir, StudyHeader},
        },
        TEXT,
    };
    use crate::validation_campaign::scenarios::model::Family;

    #[test]
    fn traffic_senders_avoid_cpu_zero_the_target_the_interrupt_and_their_cores() {
        let allowed: Vec<u32> = (0..12).collect();
        // Study CPU 1 and IRQ CPU 7 are SMT siblings of 7 and 1; 6 and 0 pair as well.
        let chosen = choose_traffic_cpus(&allowed, &[1, 7], 3);
        assert_eq!(chosen, vec![2, 3, 4]);
        assert!(choose_traffic_cpus(&[0, 1], &[1], 3).is_empty());
    }

    #[test]
    fn only_unchanged_families_have_the_same_definition() {
        let old = StudyConfig::parse(TEXT).unwrap();
        let changed =
            StudyConfig::parse(&TEXT.replace("gpu_stressors = 2", "gpu_stressors = 3")).unwrap();
        assert!(same_definition(&old, &changed, Family::Scheduler));
        assert!(same_definition(&old, &changed, Family::Io));
        assert!(!same_definition(&old, &changed, Family::Gpu));
        let traffic =
            StudyConfig::parse(&TEXT.replace("pps_per_thread = 25000", "pps_per_thread = 1000"))
                .unwrap();
        assert!(!same_definition(&old, &traffic, Family::Irq));
        assert!(same_definition(&old, &traffic, Family::Io));
    }

    fn run(family: Family, role: Role, pair: usize) -> RunRecord {
        RunRecord {
            seq: pair as u64 + 1,
            scenario: format!(
                "v2-{}-{}",
                family.as_str(),
                if role == Role::Control {
                    "control"
                } else {
                    "positive"
                }
            ),
            family: Some(family),
            role,
            pair,
            accepted: true,
            stage: None,
            reasons: vec![],
            capture_id: Some(format!("sc-{pair}")),
            fixture: Some(format!("real_fx_{}_{pair}", family.as_str())),
            metrics: serde_json::json!({"primary": 1.0}),
            unix_ms: 0,
        }
    }

    #[test]
    fn a_passed_unchanged_family_is_imported_with_its_evidence_and_provenance() {
        let root = std::env::temp_dir().join(format!("stutter-import-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (from, into) = (StudyDir(root.join("old")), StudyDir(root.join("new")));
        for dir in [&from, &into] {
            fs::create_dir_all(dir.fixtures()).unwrap();
            fs::create_dir_all(dir.impact()).unwrap();
        }
        let config = StudyConfig::parse(TEXT).unwrap();
        for family in [Family::Scheduler, Family::Irq] {
            let r = run(family, Role::Control, 0);
            let name = r.fixture.clone().unwrap();
            fs::create_dir_all(from.fixtures().join(&name)).unwrap();
            fs::write(from.fixtures().join(&name).join("fixture.toml"), "x").unwrap();
            from.append_run(&r).unwrap();
        }
        fs::write(from.impact().join("scheduler-15.json"), "{}").unwrap();
        from.append_decision(&Decision {
            family: Family::Scheduler,
            pairs: 15,
            outcome: Outcome::Pass,
            report_sha256: "0".repeat(64),
            unix_ms: 0,
        })
        .unwrap();
        from.append_decision(&Decision {
            family: Family::Irq,
            pairs: 30,
            outcome: Outcome::Invalid,
            report_sha256: "0".repeat(64),
            unix_ms: 0,
        })
        .unwrap();
        let header = StudyHeader {
            schema_version: 1,
            study_id: "v2-old".into(),
            base_commit: "a".repeat(40),
            config_sha256: "0".repeat(64),
            companion_campaign: "c".into(),
            frozen: Default::default(),
            imports: vec![],
            created_unix_ms: 0,
        };
        // An invalid family is never imported.
        assert!(import_families(&from, &header, &config, &into, &config, &[Family::Irq]).is_err());
        let import = import_families(
            &from,
            &header,
            &config,
            &into,
            &config,
            &[Family::Scheduler],
        )
        .unwrap();
        assert_eq!((import.runs, import.from_study.as_str()), (1, "v2-old"));
        assert_eq!(into.runs().unwrap().len(), 1);
        assert_eq!(into.decisions().unwrap().len(), 1);
        assert!(into.impact().join("scheduler-15.json").is_file());
        assert!(
            into.fixtures()
                .join("real_fx_scheduler_0/fixture.toml")
                .is_file()
        );
        assert!(!into.fixtures().join("real_fx_irq_0").exists());
        let _ = fs::remove_dir_all(&root);
    }
}
