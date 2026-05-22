#![allow(dead_code)] // Transitional CLI split: command-family mapping migrates from cli/mod.rs.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

pub(crate) mod autotune;
pub(crate) mod daemon;
pub(crate) mod monitor;
pub(crate) mod report;
pub(crate) mod rules;
pub(crate) mod service;
