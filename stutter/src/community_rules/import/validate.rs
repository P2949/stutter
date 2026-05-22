#![allow(dead_code)] // Transitional validation extraction target for imported rules.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ImportValidationReport {
    pub warnings: Vec<String>,
}
