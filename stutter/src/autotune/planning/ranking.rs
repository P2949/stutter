#![allow(dead_code)] // Transitional ranking extraction target.

use crate::autotune::candidate::CandidateAction;

pub(crate) fn preserve_existing_order(candidates: Vec<CandidateAction>) -> Vec<CandidateAction> {
    candidates
}
