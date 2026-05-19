#![allow(unused_imports)] // Transitional shutdown facade while old paths are preserved.

pub(crate) use crate::autotune::shutdown::{
    ActiveAutotuneAction, ActiveAutotuneActionRegistry, ShutdownReason,
    rollback_active_low_risk_actions_on_exit,
};
