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

pub(super) fn tune_candidate_order(
    profiles: &[profiles::Profile],
    runs: u32,
) -> Vec<TuneIterationOrder> {
    (1..=runs)
        .map(|iteration| TuneIterationOrder {
            iteration,
            profiles: candidate_order_for_iteration(profiles.len(), iteration)
                .into_iter()
                .map(|profile_idx| profiles[profile_idx].name.clone())
                .collect(),
        })
        .collect()
}
