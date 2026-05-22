#![allow(unused_imports)] // Transitional emergency-restore facade while old paths are preserved.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) use crate::autotune::emergency_restore::{
    AutotuneRestoreCommandInput, AutotuneRestoreOutcome, restore_known_autotune_actions,
};
