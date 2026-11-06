//! Cumulative counters that survive history rotation, restarts and crashes.
//!
//! The Prometheus `_total` metrics must never go backwards, but the history file they used to be
//! computed from is rotated. The counters are folded forward one record at a time under the
//! history lock and kept in a checkpoint next to the log. The checkpoint also names the exact
//! position it covers (the log file's identity and a byte offset), so recovery is idempotent: a
//! crash between the append and the checkpoint write is repaired by replaying the log from the
//! committed offset, and a record is never counted twice or skipped.
//!
//! When the covered file is no longer among the retained generations (the checkpoint was lost after
//! older records were legitimately rotated away) the exact lifetime total is irrecoverable. That is
//! reported as a gap, together with the time from which the counters are exact, and the counters are
//! never silently reset to zero or claimed to start earlier than the surviving evidence.
use std::{
    collections::VecDeque,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::{
    history::AutotuneHistoryEvent,
    history_rotation::previous_generation,
    prometheus_metrics::{is_applied_event, is_blocked_event},
};

/// How many recent applied action ids deduplicate `actions_applied_total`. An action id seen again
/// after this many other distinct applications is counted again.
pub(crate) const APPLIED_ID_WINDOW: usize = 1024;

/// The log position a checkpoint covers: the file's identity and the offset just after its last
/// counted record.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Covered {
    dev: u64,
    ino: u64,
    offset: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryTotals {
    pub rollbacks_total: u64,
    pub actions_applied_total: u64,
    pub actions_blocked_total: u64,
    /// The counters are exact from the record with this time on. Records older than it were either
    /// never seen or are no longer retained.
    #[serde(default)]
    pub counted_since_unix_nanos: Option<u128>,
    /// A stretch of the log could not be accounted for: the counters undercount by an unknown amount.
    #[serde(default)]
    pub gap_detected: bool,
    #[serde(default)]
    recent_applied_ids: VecDeque<String>,
    #[serde(default)]
    pub(crate) covered: Option<Covered>,
}

pub(crate) fn totals_path(history: &Path) -> PathBuf {
    let mut name = history.as_os_str().to_owned();
    name.push(".totals.json");
    PathBuf::from(name)
}

fn identity(path: &Path) -> Option<(u64, u64)> {
    fs::metadata(path).ok().map(|meta| (meta.dev(), meta.ino()))
}

/// The complete (newline-terminated) records from `offset` on, and the offset after the last one.
/// An unterminated tail is a torn write and is neither counted nor consumed.
fn complete_records(
    path: &Path,
    offset: u64,
) -> anyhow::Result<(Vec<AutotuneHistoryEvent>, u64, bool)> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), offset, false));
        }
        Err(error) => return Err(error).with_context(|| format!("open {}", path.display())),
    };
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let end = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let mut events = Vec::new();
    let mut malformed = false;
    for line in bytes[..end].split(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<AutotuneHistoryEvent>(line) {
            Ok(event) if event.validate_identity_strings().is_ok() => events.push(event),
            _ => malformed = true,
        }
    }
    Ok((events, offset + end as u64, malformed))
}

impl HistoryTotals {
    pub(crate) fn fold(&mut self, event: &AutotuneHistoryEvent) {
        self.counted_since_unix_nanos
            .get_or_insert(event.unix_nanos);
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

    fn replay(&mut self, path: &Path, offset: u64) -> anyhow::Result<u64> {
        let (events, next, malformed) = complete_records(path, offset)?;
        for event in &events {
            self.fold(event);
        }
        // A complete record that cannot be parsed is evidence of damage: say so, do not hide it.
        self.gap_detected |= malformed;
        Ok(next)
    }

    /// Counts the retained generations from their start. Used when there is no usable checkpoint.
    fn rebuild(history: &Path) -> anyhow::Result<Self> {
        let mut totals = Self::default();
        let previous = previous_generation(history);
        // With no checkpoint, records older than the retained generations may already be gone, so a
        // rotated log can only promise exactness from its oldest surviving record.
        totals.gap_detected = previous.exists();
        // A generation far above the rotation limit predates rotation (an old unbounded log);
        // parsing it would stall the writer, so the counters start after it.
        let usable = |path: &Path| {
            fs::metadata(path)
                .map(|meta| meta.len() <= 2 * super::history_rotation::HISTORY_MAX_BYTES)
                .unwrap_or(false)
        };
        if usable(&previous) {
            totals.replay(&previous, 0)?;
        }
        let end = if usable(history) {
            totals.replay(history, 0)?
        } else {
            fs::metadata(history).map_or(0, |meta| meta.len())
        };
        totals.covered = identity(history).map(|(dev, ino)| Covered {
            dev,
            ino,
            offset: end,
        });
        Ok(totals)
    }

    /// The checkpoint brought up to date with the log, in memory. Never writes. Idempotent: it counts
    /// exactly the records after the covered position.
    pub fn load(history: &Path) -> anyhow::Result<Self> {
        let stored = fs::read(totals_path(history))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok());
        let Some(mut totals) = stored else {
            return Self::rebuild(history);
        };
        let Some(covered) = totals.covered else {
            return Self::rebuild(history);
        };
        let previous = previous_generation(history);
        let current_id = identity(history);
        let previous_id = identity(&previous);
        let here = Some((covered.dev, covered.ino));
        if current_id == here {
            let end = totals.replay(history, covered.offset)?;
            totals.covered = Some(Covered {
                offset: end,
                ..covered
            });
        } else if previous_id == here {
            // The covered file was rotated away after its last counted record: finish it, then
            // count the new current file from its start.
            totals.replay(&previous, covered.offset)?;
            let end = totals.replay(history, 0)?;
            totals.covered = identity(history).map(|(dev, ino)| Covered {
                dev,
                ino,
                offset: end,
            });
        } else if current_id.is_none() && previous_id.is_none() {
            totals.covered = None;
        } else {
            // The covered file is gone: how many records it held after the checkpoint is unknown.
            // Do not guess and do not double count; keep what is certain and mark the gap.
            totals.gap_detected = true;
            let end = fs::metadata(history).map_or(0, |meta| meta.len());
            totals.covered = current_id.map(|(dev, ino)| Covered {
                dev,
                ino,
                offset: end,
            });
        }
        Ok(totals)
    }

    /// Points the checkpoint at the end of the current log file. Callers hold the history lock and
    /// have just appended and synced the record.
    pub(crate) fn cover_current(&mut self, history: &Path) {
        self.covered = identity(history).map(|(dev, ino)| Covered {
            dev,
            ino,
            offset: fs::metadata(history).map_or(0, |meta| meta.len()),
        });
    }

    /// Commits the checkpoint durably: write, sync, rename over the old one, sync the directory.
    pub(crate) fn store(&self, history: &Path) -> anyhow::Result<()> {
        let path = totals_path(history);
        let mut staging = path.as_os_str().to_owned();
        staging.push(".tmp");
        let staging = PathBuf::from(staging);
        let mut file = fs::File::create(&staging)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&staging, &path)?;
        sync_parent(&path)
    }
}

/// Makes a rename or creation in the history directory durable.
pub(crate) fn sync_parent(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}
