use crate::tune::comparability::{
    ScoredIdentityCount, scored_identity_counts_to_map, scored_identity_overlap,
};

pub(super) fn scored_identity_overlap_ratio(
    baseline: &[ScoredIdentityCount],
    candidate: &[ScoredIdentityCount],
) -> Option<f64> {
    if baseline.is_empty() && candidate.is_empty() {
        return None;
    }

    let baseline_map = scored_identity_counts_to_map(baseline);
    let candidate_map = scored_identity_counts_to_map(candidate);
    let common = scored_identity_overlap(&baseline_map, &candidate_map, usize::min);
    let total = scored_identity_overlap(&baseline_map, &candidate_map, usize::max);

    if total == 0 {
        None
    } else {
        Some(common as f64 / total as f64)
    }
}
