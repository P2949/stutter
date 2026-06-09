use super::model::TuneIterationOrder;
use crate::profiles;

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

pub(crate) fn candidate_order_for_iteration_with_strategy(
    profile_count: usize,
    iteration: u32,
    strategy: &str,
) -> Vec<usize> {
    if strategy == "fixed" {
        return (0..profile_count).collect();
    }

    if let Some(rest) = strategy.strip_prefix("seed:") {
        match rest.parse::<u64>() {
            Ok(seed) => {
                let mut order: Vec<usize> = (0..profile_count).collect();
                if profile_count <= 1 {
                    return order;
                }
                let mut state = seed.wrapping_add(iteration as u64);
                for i in (1..profile_count).rev() {
                    let r = (splitmix64(&mut state) as usize) % (i + 1);
                    order.swap(i, r);
                }
                return order;
            }
            Err(_) => {
                // fallthrough to default behavior
            }
        }
    }

    // Fallback to default alternating/rotating behaviour.
    candidate_order_for_iteration(profile_count, iteration)
}

pub(super) fn tune_candidate_order(
    profiles: &[profiles::Profile],
    runs: u32,
    strategy: &str,
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
