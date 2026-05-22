#![allow(unused_imports)] // Transitional runtime journal facade while old paths are preserved.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) use crate::autotune::controller_journal::{
    ControllerJournalActionMetadata, ControllerJournalRecord, ControllerJournalState,
    read_controller_journal, write_controller_journal_applied, write_controller_journal_applying,
    write_controller_journal_clean,
};
