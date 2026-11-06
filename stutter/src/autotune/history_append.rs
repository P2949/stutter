//! The crash-consistent append of one history record.
//!
//! Under the history lock: repair a torn tail, rotate a full file, recover the counters, write the
//! record in one `write`, sync it, and commit the totals checkpoint durably. A crash at any point
//! leaves a log whose complete records are all intact and a checkpoint that either covers them or is
//! repaired by replay on the next load, so the counters stay exact.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use anyhow::Context;

use super::{
    history::AutotuneHistoryEvent,
    history_rotation::{rotate_if_full, with_history_lock},
    history_totals::{HistoryTotals, sync_parent},
};

/// The points at which a crash can be injected in tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    AfterRotation,
    AfterRecordSynced,
    BeforeCheckpointCommit,
}

/// Removes an unterminated final record (a torn write) so the next record starts on its own line.
fn repair_torn_tail(path: &Path) -> anyhow::Result<()> {
    let Ok(bytes) = fs::read(path) else {
        return Ok(());
    };
    if bytes.is_empty() || bytes.last() == Some(&b'\n') {
        return Ok(());
    }
    let keep = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1) as u64;
    OpenOptions::new()
        .write(true)
        .open(path)?
        .set_len(keep)
        .with_context(|| format!("failed to trim a torn record from {}", path.display()))
}

pub(crate) fn append_bounded(
    path: &Path,
    event: &AutotuneHistoryEvent,
    max_bytes: u64,
    hook: &mut dyn FnMut(Stage) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    with_history_lock(path, || {
        repair_torn_tail(path)?;
        rotate_if_full(path, max_bytes)?;
        sync_parent(path)?;
        hook(Stage::AfterRotation)?;
        // Counters as of the log before this record, repaired by replay if a crash left them behind.
        let mut totals = HistoryTotals::load(path)?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("failed to open autotune history log {}", path.display()))?;
        // The whole record, newline included, goes out in one write.
        let mut record = serde_json::to_vec(event).with_context(|| {
            format!("failed to encode autotune history event {}", path.display())
        })?;
        record.push(b'\n');
        file.write_all(&record).with_context(|| {
            format!("failed to write autotune history event {}", path.display())
        })?;
        // A routine observation (one per second while idle) carries no decision worth a flush: losing
        // one in a power failure costs nothing, and syncing it would put several fsyncs a second on an
        // idle machine. Every other record is made durable before the checkpoint can refer to it.
        let routine = is_routine_observation(event);
        if !routine {
            file.sync_data()?;
        }
        drop(file);
        if !routine {
            sync_parent(path)?;
        }
        hook(Stage::AfterRecordSynced)?;

        let before = totals.clone();
        totals.fold(event);
        let uncovered = HistoryTotals::uncovered_bytes(path);
        if routine
            && totals == before.with_same_position(&totals)
            && uncovered < CHECKPOINT_SLACK_BYTES
        {
            // Nothing the counters track changed and little log is uncovered: replay will recover it.
            return Ok(());
        }
        totals.cover_current(path);
        hook(Stage::BeforeCheckpointCommit)?;
        totals.store(path).with_context(|| {
            format!(
                "failed to commit the autotune history totals for {}",
                path.display()
            )
        })
    })
}

/// How much log may follow the checkpoint before a routine observation advances it; bounds the replay
/// every append and every metrics scrape performs.
pub(crate) const CHECKPOINT_SLACK_BYTES: u64 = 256 * 1024;

/// An `observed` record: the controller looked and decided nothing.
pub(crate) fn is_routine_observation(event: &AutotuneHistoryEvent) -> bool {
    event.decision.decision == "observed" && !event.rollback_performed
}
