#![allow(dead_code)] // Transitional tick-context namespace for session stage extraction.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) mod focus;
pub(crate) mod foreground;
pub(crate) mod frame;
pub(crate) mod hardware;
pub(crate) mod probe;
pub(crate) mod summary;
pub(crate) mod target;
pub(crate) mod telemetry;
