//! Test modules for `foreground` split by foreground behavior area.
//!
//! Owns foreground test module wiring and shared environment/provider test helpers.
//! Does not own production foreground behavior.

mod hyprland;
mod redaction;
mod resolver;
mod sway_parse;
mod x11_parse;

use super::{ForegroundProvider, ForegroundSource, ForegroundWindowSnapshot};

struct SequenceProvider {
    source: ForegroundSource,
    snapshots: Vec<ForegroundWindowSnapshot>,
    index: usize,
}

impl SequenceProvider {
    fn new(source: ForegroundSource, snapshots: Vec<ForegroundWindowSnapshot>) -> Self {
        Self {
            source,
            snapshots,
            index: 0,
        }
    }
}

impl ForegroundProvider for SequenceProvider {
    fn source(&self) -> ForegroundSource {
        self.source
    }

    fn sample(&mut self, elapsed_ms: u64) -> ForegroundWindowSnapshot {
        let mut snapshot = self.snapshots.get(self.index).cloned().unwrap_or_else(|| {
            ForegroundWindowSnapshot::unavailable(
                elapsed_ms,
                self.source,
                "sequence provider exhausted",
            )
        });

        self.index = self.index.saturating_add(1);
        snapshot.elapsed_ms = elapsed_ms;
        snapshot
    }
}

pub(super) unsafe fn restore_env_var(name: &str, value: Option<std::ffi::OsString>) {
    if let Some(value) = value {
        unsafe { std::env::set_var(name, value) };
    } else {
        unsafe { std::env::remove_var(name) };
    }
}
