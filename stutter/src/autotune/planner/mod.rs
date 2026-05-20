//! Compatibility facade for the autotune planner API.

pub use crate::autotune::planning::{
    denial::CandidateDenyReason,
    model::{
        CandidateEvaluation, PlanResult, PlannerDenySummary, PlannerEvaluationSummary,
        PlannerNoActionSummary, PlannerSelectedSummary, PlannerSummary,
    },
    planner::{CandidatePlanner, PlannerInput},
};
pub(crate) use crate::autotune::planning::{
    evaluate::evaluate_proposals_with_runner, ranking::sort_candidate_evaluations,
};

#[cfg(test)]
#[path = "../planner_tests/mod.rs"]
mod planner_tests;
