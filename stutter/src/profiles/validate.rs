#![allow(dead_code)] // Transitional validation extraction target.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileValidationReport {
    pub warnings: Vec<String>,
}
