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
