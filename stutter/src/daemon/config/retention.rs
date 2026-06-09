use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRetentionConfig {
    pub max_history_events: usize,
    pub max_state_snapshots: usize,
    pub retain_crash_diagnostics: bool,
}

impl Default for DaemonRetentionConfig {
    fn default() -> Self {
        Self {
            max_history_events: 10_000,
            max_state_snapshots: 16,
            retain_crash_diagnostics: true,
        }
    }
}
