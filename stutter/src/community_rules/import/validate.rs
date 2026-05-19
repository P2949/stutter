#![allow(dead_code)] // Transitional validation extraction target for imported rules.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ImportValidationReport {
    pub warnings: Vec<String>,
}
