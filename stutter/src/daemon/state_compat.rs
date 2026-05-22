#![allow(dead_code)] // Transitional daemon state compatibility namespace.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) const CURRENT_DAEMON_STATE_SCHEMA_VERSION: u32 =
    crate::daemon::state::DAEMON_STATE_SCHEMA_VERSION;
