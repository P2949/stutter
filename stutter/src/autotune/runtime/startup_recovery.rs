#![allow(unused_imports)] // Transitional runtime recovery facade while old paths are preserved.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) use crate::autotune::startup_recovery::{
    StartupRecoveryConfig, StartupRecoveryOutcome, recover_controller_journal_on_startup,
    recover_controller_journal_with_executor,
};
