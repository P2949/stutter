#![allow(dead_code)] // Transitional gate extraction target.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GateDecision {
    pub passed: bool,
    pub reason_code: Option<String>,
    pub message: Option<String>,
}
