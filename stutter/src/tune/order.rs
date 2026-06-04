use super::model::TuneIterationOrder;
use crate::profiles;

pub(crate) fn candidate_order_for_iteration(profile_count: usize, iteration: u32) -> Vec<usize> {
    let mut order: Vec<usize> = (0..profile_count).collect();

    if profile_count <= 1 {
        return order;
    }

    let rotation = ((iteration - 1) as usize) % profile_count;
    order.rotate_left(rotation);

    if iteration.is_multiple_of(2) {
        order.reverse();
    }

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
