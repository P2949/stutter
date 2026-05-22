#![allow(dead_code)] // Transitional service boundary; command call sites migrate incrementally.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

#[derive(Clone, Debug, Default)]
pub(crate) struct ReportService;
