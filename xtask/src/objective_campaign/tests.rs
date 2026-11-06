use super::stats::*;

const SECOND: u64 = 1_000_000_000;

fn run(block: u32, position: u32, condition: Condition, seconds: f64) -> Run {
    Run {
        block,
        position,
        condition,
        elapsed_ns: (seconds * SECOND as f64) as u64,
        execution_id: format!("e-{block}-{position}"),
    }
}

use Condition::{Clean as C, Interfered as I};

/// One block of each order, then three more of the same shape with the given timings.
fn blocks(clean: f64, interfered: f64, count: u32, jitter: f64) -> Vec<Run> {
    let mut runs = Vec::new();
    for block in 0..count {
        let wobble = |n: u32| 1.0 + jitter * f64::from((block * 7 + n * 3) % 5) / 4.0;
        let order = if block % 2 == 0 {
            [C, C, I, C]
        } else {
            [C, I, C, C]
        };
        for (position, condition) in order.into_iter().enumerate() {
            let base = if condition == C { clean } else { interfered };
            runs.push(run(
                block,
                position as u32,
                condition,
                base * wobble(position as u32),
            ));
        }
    }
    runs
}

const RULE: Rule = Rule {
    noise_margin: 2.0,
    min_beneficial_keep_rate: 0.8,
    max_false_keeps: 0,
};
const GRID: [f64; 5] = [7.5, 10.0, 12.5, 15.0, 20.0];

#[test]
fn each_block_yields_one_pair_of_every_kind_in_either_order_and_never_crosses_blocks() {
    let pairs = pairs(&blocks(80.0, 105.0, 4, 0.0));
    assert_eq!(pairs.len(), 4 * 3);
    for block in 0..4 {
        let mut kinds: Vec<_> = pairs
            .iter()
            .filter(|pair| pair.block == block)
            .map(|pair| pair.kind)
            .collect();
        kinds.sort_by_key(|kind| *kind as u8);
        assert_eq!(
            kinds,
            [
                PairKind::Neutral,
                PairKind::Beneficial,
                PairKind::Regression
            ]
        );
    }
    // Two interfered runs in a row compare nothing that differs, so they form no pair.
    let two = [run(0, 0, I, 100.0), run(0, 1, I, 101.0)];
    assert!(super::stats::pairs(&two).is_empty());
}

#[test]
fn the_controllers_own_rule_judges_every_pair_at_the_threshold() {
    let pair = |kind, baseline: f64, candidate: f64| Pair {
        block: 0,
        kind,
        baseline_ns: (baseline * SECOND as f64) as u64,
        candidate_ns: (candidate * SECOND as f64) as u64,
    };
    let pairs = [
        pair(PairKind::Beneficial, 100.0, 85.0),
        pair(PairKind::Neutral, 100.0, 90.0),
        pair(PairKind::Regression, 100.0, 125.0),
    ];
    // At 10 percent the 15 percent gain is kept, but a neutral 10 percent drop is a false keep.
    let at_ten = tally(&pairs, 10.0);
    assert_eq!((at_ten.beneficial_kept, at_ten.false_keeps), (1, 1));
    // At 12.5 percent only the genuine gain is kept.
    let at_twelve = tally(&pairs, 12.5);
    assert_eq!((at_twelve.beneficial_kept, at_twelve.false_keeps), (1, 0));
    // A regression is never a keep, whatever the threshold.
    assert_eq!(tally(&pairs[2..], 1.0).false_keeps, 0);
}

#[test]
fn the_smallest_threshold_that_clears_noise_and_keeps_the_gain_is_selected() {
    let calibration = pairs(&blocks(80.0, 105.0, 10, 0.01));
    let (threshold, tally) = select_threshold(&calibration, &GRID, &RULE).unwrap();
    assert_eq!(threshold, 7.5);
    assert_eq!(tally.false_keeps, 0);
    assert!(tally.keep_rate() >= 0.8);
}

#[test]
fn noise_larger_than_the_candidates_or_a_gain_smaller_than_them_selects_nothing() {
    // Clean runs wobbling by 5 percent make a 7.5 percent threshold unsafe (needs 2x the noise),
    // and 10 percent only just: the margin pushes selection to the first clearing value.
    let noisy = pairs(&blocks(80.0, 105.0, 10, 0.05));
    let (threshold, _) = select_threshold(&noisy, &GRID, &RULE).unwrap();
    assert!(threshold >= 2.0 * neutral_noise_percent(&noisy));
    // A real effect of only 4 percent is below every candidate: unproven, not forced.
    assert!(select_threshold(&pairs(&blocks(80.0, 83.2, 10, 0.0)), &GRID, &RULE).is_none());
    // Noise above the whole grid also selects nothing.
    assert!(select_threshold(&pairs(&blocks(80.0, 105.0, 10, 0.30)), &GRID, &RULE).is_none());
}

#[test]
fn a_holdout_needs_every_kind_of_pair_no_false_keep_and_the_keep_rate() {
    let good = Tally {
        neutral: 5,
        regression: 5,
        beneficial: 5,
        beneficial_kept: 5,
        false_keeps: 0,
    };
    assert!(holdout_passes(&good, &RULE));
    assert!(!holdout_passes(
        &Tally {
            false_keeps: 1,
            ..good
        },
        &RULE
    ));
    assert!(!holdout_passes(
        &Tally {
            beneficial_kept: 3,
            ..good
        },
        &RULE
    ));
    assert!(!holdout_passes(&Tally { neutral: 0, ..good }, &RULE));
    assert!(!holdout_passes(
        &Tally {
            beneficial: 0,
            beneficial_kept: 0,
            ..good
        },
        &RULE
    ));
}

#[test]
fn the_committed_manifest_is_valid_and_cannot_be_loosened() {
    let text = include_str!("../../../docs/validation/objective-graduation.toml");
    let spec = super::parse_manifest(text).unwrap();
    assert_eq!(spec.rule().max_false_keeps, 0);
    for (old, new) in [
        ("max_false_keeps = 0", "max_false_keeps = 1"),
        (
            "min_beneficial_keep_rate = 0.8",
            "min_beneficial_keep_rate = 0.5",
        ),
        ("calibration_blocks = 10", "calibration_blocks = 3"),
        ("holdout_blocks = 5", "holdout_blocks = 1"),
        ("noise_margin = 2.0", "noise_margin = 0.5"),
    ] {
        assert!(
            super::parse_manifest(&text.replace(old, new)).is_err(),
            "{old}"
        );
    }
}

#[test]
fn block_orders_alternate_and_every_block_has_one_pair_of_each_kind() {
    let a = super::block_order(0);
    let b = super::block_order(1);
    assert_ne!(a, b);
    for order in [a, b] {
        assert_eq!(
            order
                .iter()
                .filter(|c| **c == Condition::Interfered)
                .count(),
            1
        );
        assert_eq!(order.len(), 4);
    }
}
