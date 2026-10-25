use super::*;

fn summary(p99_ns: u64, over_5ms: u64) -> RunnableSummaryRecord {
    RunnableSummaryRecord {
        p99_ns,
        samples: 2_000,
        over_5ms,
    }
}

fn pair(scenario: Scenario, index: usize, baseline_p99: u64, candidate_p99: u64) -> PairResult {
    PairResult {
        scenario,
        pair: index,
        attempt: 0,
        order: if index.is_multiple_of(2) {
            Order::BaselineFirst
        } else {
            Order::CandidateFirst
        },
        baseline: summary(baseline_p99, 20),
        candidate: summary(candidate_p99, 20),
        placement_observed: true,
        guardrails_ok: true,
        manager_kept: false,
    }
}

fn grid() -> ThresholdGrid {
    ThresholdGrid {
        min_improvement_percent: vec![10.0, 12.5, 15.0, 20.0],
        max_regression_percent: vec![5.0, 7.5, 10.0],
        min_samples: vec![1000],
    }
}

fn dataset() -> Vec<PairResult> {
    let mut pairs = Vec::new();
    for index in 0..10 {
        // Beneficial: a clear ~30 % improvement.
        pairs.push(pair(Scenario::Beneficial, index, 10_000_000, 7_000_000));
        // Neutral: ~3 % noise either way.
        let noise = if index % 2 == 0 {
            10_300_000
        } else {
            9_700_000
        };
        pairs.push(pair(Scenario::Neutral, index, 10_000_000, noise));
        // Regression: ~25 % worse.
        pairs.push(pair(Scenario::Regression, index, 10_000_000, 12_500_000));
    }
    pairs
}

#[test]
fn the_split_is_stable_and_roughly_two_to_one() {
    let mut calibration = 0;
    let mut holdout = 0;
    for scenario in Scenario::ALL {
        for index in 0..300 {
            let part = part_of(scenario, index, 2, 1);
            assert_eq!(part, part_of(scenario, index, 2, 1));
            match part {
                Part::Calibration => calibration += 1,
                Part::Holdout => holdout += 1,
            }
        }
    }
    let share = calibration as f64 / (calibration + holdout) as f64;
    assert!((0.60..0.74).contains(&share), "calibration share {share}");
}

#[test]
fn clear_data_keeps_the_beneficial_pairs_and_reverts_the_rest() {
    let pairs = dataset();
    let tally = tally(pairs.iter(), default_thresholds());
    assert_eq!(tally.beneficial, 10);
    assert_eq!(tally.beneficial_kept, 10);
    assert_eq!(tally.false_keeps, 0);
    assert_eq!(tally.regression_reverted, 10);
}

#[test]
fn a_missing_mechanism_or_failed_guardrail_withholds_a_keep() {
    let mut pairs = dataset();
    pairs[0].placement_observed = false;
    pairs[3].guardrails_ok = false;
    let tally = tally(pairs.iter(), default_thresholds());
    assert_eq!(tally.beneficial_kept, 8);
    assert_eq!(tally.guarded_out, 2);
}

#[test]
fn selection_prefers_zero_false_keeps_then_the_defaults() {
    let pairs = dataset();
    let calibration: Vec<&PairResult> = pairs.iter().collect();
    let (selected, tally) = select_thresholds(&calibration, &grid()).unwrap();
    assert_eq!(tally.false_keeps, 0);
    assert_eq!(tally.beneficial_kept, 10);
    // Every grid point ties on the data, so the one nearest the defaults wins.
    assert_eq!(selected, default_thresholds());
}

#[test]
fn a_noisy_neutral_scenario_forces_a_stricter_threshold() {
    let mut pairs = dataset();
    // A neutral pair that looks like a 15 % win: the defaults would falsely keep it.
    pairs.push(pair(Scenario::Neutral, 10, 10_000_000, 8_500_000));
    let calibration: Vec<&PairResult> = pairs.iter().collect();
    let default_tally = tally(calibration.iter().copied(), default_thresholds());
    assert_eq!(default_tally.false_keeps, 1);
    let (selected, tally) = select_thresholds(&calibration, &grid()).unwrap();
    assert_eq!(tally.false_keeps, 0);
    assert!(selected.min_improvement_percent > 15.0, "{selected:?}");
    assert_eq!(tally.beneficial_kept, 10);
}

#[test]
fn no_grid_point_with_zero_false_keeps_means_no_calibration() {
    let mut pairs = dataset();
    // A neutral pair indistinguishable from the beneficial ones at every grid threshold.
    pairs.push(pair(Scenario::Neutral, 10, 10_000_000, 6_000_000));
    let calibration: Vec<&PairResult> = pairs.iter().collect();
    assert!(select_thresholds(&calibration, &grid()).is_none());
}

#[test]
fn the_holdout_needs_zero_false_keeps_and_the_keep_rate() {
    let pairs = dataset();
    let holdout: Vec<&PairResult> = pairs.iter().collect();
    let verdict = evaluate_holdout(&holdout, default_thresholds(), 0.8);
    assert!(verdict.passed);

    let mut weaker = dataset();
    for pair in weaker
        .iter_mut()
        .filter(|p| p.scenario == Scenario::Beneficial)
        .take(5)
    {
        pair.candidate = summary(9_500_000, 20);
    }
    let holdout: Vec<&PairResult> = weaker.iter().collect();
    let verdict = evaluate_holdout(&holdout, default_thresholds(), 0.8);
    assert!(!verdict.passed);
    assert!(verdict.zero_false_keeps);
    assert!(verdict.beneficial_keep_rate < 0.8);

    let mut leaky = dataset();
    leaky.push(pair(Scenario::Neutral, 99, 10_000_000, 7_000_000));
    let holdout: Vec<&PairResult> = leaky.iter().collect();
    assert!(!evaluate_holdout(&holdout, default_thresholds(), 0.8).passed);
}

#[test]
fn an_empty_beneficial_scenario_cannot_pass() {
    let pairs: Vec<PairResult> = dataset()
        .into_iter()
        .filter(|p| p.scenario != Scenario::Beneficial)
        .collect();
    let holdout: Vec<&PairResult> = pairs.iter().collect();
    assert!(!evaluate_holdout(&holdout, default_thresholds(), 0.8).passed);
}
