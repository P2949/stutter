use std::{fmt, str::FromStr};

use super::model::TuneIterationOrder;
use crate::profiles;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuneOrderStrategy {
    Alternating,
    Fixed,
    Seed(u64),
}

impl FromStr for TuneOrderStrategy {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "alternating" => Ok(Self::Alternating),
            "fixed" => Ok(Self::Fixed),
            _ => {
                if let Some(seed) = value.strip_prefix("seed:") {
                    if seed.is_empty() {
                        anyhow::bail!(
                            "invalid --order value '{value}': expected alternating, fixed, or seed:<number>"
                        );
                    }

                    let seed = seed.parse::<u64>().map_err(|_| {
                        anyhow::anyhow!(
                            "invalid --order value '{value}': expected alternating, fixed, or seed:<number>"
                        )
                    })?;

                    Ok(Self::Seed(seed))
                } else {
                    anyhow::bail!(
                        "invalid --order value '{value}': expected alternating, fixed, or seed:<number>"
                    );
                }
            }
        }
    }
}

impl fmt::Display for TuneOrderStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Alternating => f.write_str("alternating"),
            Self::Fixed => f.write_str("fixed"),
            Self::Seed(seed) => write!(f, "seed:{seed}"),
        }
    }
}

pub(crate) fn candidate_order_for_iteration(profile_count: usize, iteration: u32) -> Vec<usize> {
    let mut order: Vec<usize> = (0..profile_count).collect();

    if profile_count <= 1 {
        return order;
    }

    if profile_count == 2 {
        if iteration.is_multiple_of(2) {
            order.reverse();
        }
        return order;
    }

    let zero_based = iteration.saturating_sub(1) as usize;
    let rotation = zero_based % profile_count;
    let reverse_block = (zero_based / profile_count) % 2 == 1;

    if reverse_block {
        order.reverse();
    }
    order.rotate_left(rotation);

    order
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    z ^ (z >> 31)
}

fn seeded_order(profile_count: usize, iteration: u32, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..profile_count).collect();
    if profile_count <= 1 {
        return order;
    }

    let mut state = seed.wrapping_add(iteration as u64);
    for i in (1..profile_count).rev() {
        let r = (splitmix64(&mut state) as usize) % (i + 1);
        order.swap(i, r);
    }

    order
}

pub(crate) fn candidate_order_for_iteration_with_strategy(
    profile_count: usize,
    iteration: u32,
    strategy: TuneOrderStrategy,
) -> Vec<usize> {
    match strategy {
        TuneOrderStrategy::Alternating => candidate_order_for_iteration(profile_count, iteration),
        TuneOrderStrategy::Fixed => (0..profile_count).collect(),
        TuneOrderStrategy::Seed(seed) => seeded_order(profile_count, iteration, seed),
    }
}

pub(super) fn tune_candidate_order(
    profiles: &[profiles::Profile],
    runs: u32,
    strategy: TuneOrderStrategy,
) -> Vec<TuneIterationOrder> {
    (1..=runs)
        .map(|iteration| TuneIterationOrder {
            iteration,
            profiles: candidate_order_for_iteration_with_strategy(
                profiles.len(),
                iteration,
                strategy,
            )
            .into_iter()
            .map(|profile_idx| profiles[profile_idx].name.clone())
            .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_order_strategy_accepts_valid_values() {
        assert_eq!(
            "alternating".parse::<TuneOrderStrategy>().unwrap(),
            TuneOrderStrategy::Alternating
        );
        assert_eq!(
            "fixed".parse::<TuneOrderStrategy>().unwrap(),
            TuneOrderStrategy::Fixed
        );
        assert_eq!(
            "seed:0".parse::<TuneOrderStrategy>().unwrap(),
            TuneOrderStrategy::Seed(0)
        );
        assert_eq!(
            "seed:42".parse::<TuneOrderStrategy>().unwrap(),
            TuneOrderStrategy::Seed(42)
        );
    }

    #[test]
    fn parse_order_strategy_rejects_invalid_values() {
        for value in [
            "",
            "seed:",
            "seed:nope",
            "seed:-1",
            "seed:1.2",
            "random",
            "fixed:1",
        ] {
            assert!(
                value.parse::<TuneOrderStrategy>().is_err(),
                "{value} should be rejected"
            );
        }
    }

    #[test]
    fn invalid_seed_no_longer_falls_back_to_alternating() {
        assert!("seed:nope".parse::<TuneOrderStrategy>().is_err());
    }

    #[test]
    fn fixed_strategy_keeps_candidate_order() {
        assert_eq!(
            candidate_order_for_iteration_with_strategy(3, 2, TuneOrderStrategy::Fixed),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn alternating_strategy_preserves_existing_two_profile_counterbalance() {
        assert_eq!(
            candidate_order_for_iteration_with_strategy(2, 1, TuneOrderStrategy::Alternating),
            vec![0, 1]
        );
        assert_eq!(
            candidate_order_for_iteration_with_strategy(2, 2, TuneOrderStrategy::Alternating),
            vec![1, 0]
        );
    }

    #[test]
    fn seeded_strategy_is_deterministic() {
        let first = candidate_order_for_iteration_with_strategy(4, 1, TuneOrderStrategy::Seed(123));
        let second =
            candidate_order_for_iteration_with_strategy(4, 1, TuneOrderStrategy::Seed(123));
        assert_eq!(first, second);
    }
}
