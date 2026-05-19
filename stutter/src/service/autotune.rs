#![allow(dead_code)] // Transitional service boundary; agent/CLI call sites migrate incrementally.

#[derive(Clone, Debug, Default)]
pub(crate) struct AutotuneService;

impl AutotuneService {
    pub(crate) fn new() -> Self {
        Self
    }
}
