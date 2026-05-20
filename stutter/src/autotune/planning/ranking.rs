//! Candidate ranking and no-action explanation helpers; this module owns ordering, not proposal evaluation.

use crate::autotune::planning::{denial::grouped_denials, model::CandidateEvaluation};

pub(crate) fn no_action_reason_for_evaluations(evaluations: &[CandidateEvaluation]) -> String {
    if evaluations.is_empty() {
        return "no candidate providers produced an eligible candidate".to_owned();
    }

    let grouped = grouped_denials(evaluations);
    if grouped.is_empty() {
        return "candidate providers produced no eligible candidate".to_owned();
    }

    let reason_counts = grouped
        .iter()
        .take(4)
        .map(|summary| format!("{}={}", summary.reason_code, summary.count))
        .collect::<Vec<_>>()
        .join(",");
    let top_message = evaluations
        .iter()
        .find(|evaluation| !evaluation.eligible)
        .and_then(|evaluation| evaluation.deny_messages.first())
        .cloned()
        .unwrap_or_else(|| "all candidates were denied".to_owned());

    format!("{top_message}; deny_reason_counts={reason_counts}")
}

pub(crate) fn sort_candidate_evaluations(evaluations: &mut [CandidateEvaluation]) {
    evaluations.sort_by(|left, right| {
        (!left.eligible)
            .cmp(&(!right.eligible))
            .then_with(|| {
                left.rank
                    .unwrap_or(u32::MAX)
                    .cmp(&right.rank.unwrap_or(u32::MAX))
            })
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| left.candidate_name.cmp(&right.candidate_name))
    });
}

pub(crate) fn rank_with_workload_memory(rank_hint: u32, memory_adjustment: Option<i32>) -> u32 {
    let adjusted = rank_hint as i64 + i64::from(memory_adjustment.unwrap_or(0));
    adjusted.clamp(0, u32::MAX as i64) as u32
}
