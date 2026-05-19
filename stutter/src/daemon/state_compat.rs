#![allow(dead_code)] // Transitional daemon state compatibility namespace.

pub(crate) const CURRENT_DAEMON_STATE_SCHEMA_VERSION: u32 =
    crate::daemon::state::DAEMON_STATE_SCHEMA_VERSION;
