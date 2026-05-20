//! Foreground provider parser namespaces.
//!
//! Owns parser module wiring for compositor/window-system payloads. Does not own process execution
//! or root foreground API re-exports.

pub(crate) mod sway;
pub(crate) mod x11;
