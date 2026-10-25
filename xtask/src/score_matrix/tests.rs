use super::model::{MatrixConfig, Profile, level_order, slots};

const TEXT: &str = include_str!("../../../docs/validation/scenarios/score-matrix.toml");

#[test]
fn the_committed_matrix_is_valid_and_covers_every_cell() {
    let config = MatrixConfig::parse(TEXT).unwrap();
    let all = slots(&config);
    assert_eq!(all.len(), 4 * 6 * 5);
    for profile in Profile::ALL {
        for level in &config.levels_ms {
            let n = all
                .iter()
                .filter(|s| s.profile == profile && s.level_ms == *level)
                .count();
            assert_eq!(n, config.repetitions, "{profile:?} {level}");
        }
    }
}

#[test]
fn level_order_is_counterbalanced_not_ascending() {
    let n = 6;
    let orders: Vec<Vec<usize>> = (0..6).map(|k| level_order(n, k)).collect();
    assert_ne!(orders[0], vec![0, 1, 2, 3, 4, 5]);
    for order in &orders {
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..6).collect::<Vec<_>>());
    }
    // Over six repetitions every level occupies every position exactly once.
    for position in 0..n {
        let mut seen: Vec<usize> = orders.iter().map(|o| o[position]).collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..n).collect::<Vec<_>>(), "position {position}");
    }
    // ... and every ordered pair of neighbours occurs exactly once (a Williams design).
    let mut pairs = std::collections::BTreeSet::new();
    for order in &orders {
        for w in order.windows(2) {
            assert!(pairs.insert((w[0], w[1])), "{w:?} repeated");
        }
    }
    assert_eq!(pairs.len(), n * (n - 1));
}

#[test]
fn malformed_matrices_are_rejected() {
    assert!(
        MatrixConfig::parse(&TEXT.replace("[0, 1, 2, 4, 8, 16]", "[1, 2, 4, 8, 16, 32]")).is_err()
    );
    assert!(MatrixConfig::parse(&TEXT.replace("repetitions = 5", "repetitions = 2")).is_err());
    assert!(MatrixConfig::parse(&TEXT.replace("period_ms = 32", "period_ms = 8")).is_err());
}
