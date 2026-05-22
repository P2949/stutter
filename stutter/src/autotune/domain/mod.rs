#![allow(unused_imports)] // Transitional domain façade while pure types move from flat modules.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) mod decision;
pub(crate) mod measurement;
pub(crate) mod mode;
pub(crate) mod objective;
pub(crate) mod safety;
pub(crate) mod state;
pub(crate) mod workload;
