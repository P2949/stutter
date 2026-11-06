//! Cumulative counters that survive history rotation and restarts.
//!
//! The Prometheus `_total` metrics must never go backwards, but the history file they used to be
//! computed from is rotated. The counters are therefore folded forward one event at a time under
//! the history lock and kept in a small checkpoint next to the log. When the checkpoint is
//! missing (first run, or a failed write) it is rebuilt once from the retained generations.
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    history::{AutotuneHistoryEvent, read_autotune_history_events},
    history_rotation::previous_generation,
    prometheus_metrics::{is_applied_event, is_blocked_event},
};

/// How many recent applied action ids deduplicate `actions_applied_total`. An action id seen again
/// after this many other distinct applications is counted again.
pub(crate) const APPLIED_ID_WINDOW: usize = 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryTotals {
    pub rollbacks_total: u64,
    pub actions_applied_total: u64,
    pub actions_blocked_total: u64,
    #[serde(default)]
    recent_applied_ids: VecDeque<String>,
}

pub(crate) fn totals_path(history: &Path) -> PathBuf {
    let mut name = history.as_os_str().to_owned();
    name.push(".totals.json");
    PathBuf::from(name)
}

impl HistoryTotals {
    pub(crate) fn fold(&mut self, event: &AutotuneHistoryEvent) {
        if event.rollback_performed {
            self.rollbacks_total = self.rollbacks_total.saturating_add(1);
        }
        if is_blocked_event(event) {
            self.actions_blocked_total = self.actions_blocked_total.saturating_add(1);
        }
        if is_applied_event(event) {
            let id = event
                .action_id
                .as_ref()
                .map(|action_id| action_id.as_str().to_owned())
                .unwrap_or_else(|| format!("event:{}", event.unix_nanos));
            if !self.recent_applied_ids.contains(&id) {
                self.actions_applied_total = self.actions_applied_total.saturating_add(1);
                if self.recent_applied_ids.len() == APPLIED_ID_WINDOW {
                    self.recent_applied_ids.pop_front();
                }
                self.recent_applied_ids.push_back(id);
            }
        }
    }

    /// Rebuilds the counters from the retained generations, oldest first.
    pub(crate) fn rebuild(history: &Path) -> anyhow::Result<Self> {
        let mut totals = Self::default();
        for path in [previous_generation(history), history.to_path_buf()] {
            // A generation far above the rotation limit predates rotation (an old unbounded log);
            // parsing it would stall the writer, so the counters start after it.
            let oversized = fs::metadata(&path)
                .map(|meta| meta.len() > 2 * super::history_rotation::HISTORY_MAX_BYTES)
                .unwrap_or(false);
            if oversized {
                continue;
            }
            for event in read_autotune_history_events(&path)? {
                totals.fold(&event);
            }
        }
        Ok(totals)
    }

    /// The checkpoint, or a rebuild when it is missing or unreadable. Never writes.
    pub fn load(history: &Path) -> anyhow::Result<Self> {
        match fs::read(totals_path(history)) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(totals) => Ok(totals),
                Err(_) => Self::rebuild(history),
            },
            Err(_) => Self::rebuild(history),
        }
    }

    /// Writes the checkpoint atomically. Callers hold the history lock.
    pub(crate) fn store(&self, history: &Path) -> anyhow::Result<()> {
        let path = totals_path(history);
        let mut staging = path.as_os_str().to_owned();
        staging.push(".tmp");
        let staging = PathBuf::from(staging);
        fs::write(&staging, serde_json::to_vec(self)?)?;
        fs::rename(&staging, &path)?;
        Ok(())
    }
}
