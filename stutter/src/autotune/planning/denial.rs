#![allow(dead_code)] // Transitional denial extraction target.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PlanningDenial {
    pub reason_code: String,
    pub message: String,
}
