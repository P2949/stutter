#![allow(dead_code)] // Transitional validation extraction target.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileValidationReport {
    pub warnings: Vec<String>,
}
