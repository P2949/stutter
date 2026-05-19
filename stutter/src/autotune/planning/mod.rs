#![allow(unused_imports)] // Transitional planning façade while planner/candidate split lands.

pub(crate) mod candidate;
pub(crate) mod collect;
pub(crate) mod denial;
pub(crate) mod dry_run;
pub(crate) mod executable_plan;
pub(crate) mod gates;
pub(crate) mod plan_io;
pub(crate) mod profile_candidates;
pub(crate) mod ranking;
pub(crate) mod suggestion;

pub(crate) use crate::autotune::candidate::{
    ApplyCandidate, ApplyEligibility, CandidateAction, CandidatePlanFile, SuggestionCandidate,
};
