#![allow(unused_imports)] // Transitional shutdown facade while old paths are preserved.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) use crate::autotune::shutdown::{
    ActiveAutotuneAction, ActiveAutotuneActionRegistry, ShutdownReason,
    rollback_active_low_risk_actions_on_exit,
};
