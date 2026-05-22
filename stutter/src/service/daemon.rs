#![allow(dead_code)] // Transitional service boundary; agent/CLI call sites migrate incrementally.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

#[derive(Clone, Debug, Default)]
pub(crate) struct DaemonService;

impl DaemonService {
    pub(crate) fn new() -> Self {
        Self
    }
}
