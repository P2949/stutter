//! The crash-consistent append of one history record.
//!
//! Under the history lock: repair a torn tail, rotate a full file, recover the counters, write the
//! record in one `write`, sync it, and commit the totals checkpoint durably. A crash at any point
//! leaves a log whose complete records are all intact and a checkpoint that either covers them or is
//! repaired by replay on the next load, so the counters stay exact.
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom, Write},
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

/// The longest unterminated final record that is trimmed. A torn tail is at most one interrupted
/// record, so anything longer is not a torn write and is never cut.
pub(crate) const MAX_TORN_RECORD_BYTES: u64 = 1024 * 1024;

/// Removes an unterminated final record (a torn write) so the next record starts on its own line.
///
/// Only the last byte is read when the file is well formed, which is every append but the first after
/// a crash; the whole file is never read. A torn tail is found by searching backwards for the last
/// newline, at most `limit` bytes. If none is found within that bound the file is left untouched and
/// the append fails rather than removing records that might be complete.
pub(crate) fn repair_torn_tail(path: &Path, limit: u64) -> anyhow::Result<()> {
    let mut file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("open {}", path.display())),
    };
    let length = file.metadata()?.len();
    if length == 0 {
        return Ok(());
    }
    let mut last = [0_u8; 1];
    file.seek(SeekFrom::Start(length - 1))?;
    file.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(());
    }
    // A torn tail: search backwards in blocks for the last newline, within the bound.
    let floor = length.saturating_sub(limit);
    let mut end = length;
    let mut keep = None;
    while end > floor {
        let start = end.saturating_sub(64 * 1024).max(floor);
        let mut block = vec![0_u8; (end - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut block)?;
        if let Some(index) = block.iter().rposition(|byte| *byte == b'\n') {
            keep = Some(start + index as u64 + 1);
            break;
        }
        end = start;
    }
    let keep = match keep {
        Some(keep) => keep,
        // No newline at all and the whole file fits the bound: the file is one torn record.
        None if length <= limit => 0,
        None => anyhow::bail!(
            "{} ends with an unterminated record longer than {limit} bytes; refusing to cut it",
            path.display()
        ),
    };
    file.set_len(keep)
        .with_context(|| format!("failed to trim a torn record from {}", path.display()))
}

pub(crate) fn append_bounded(
    path: &Path,
    event: &AutotuneHistoryEvent,
    max_bytes: u64,
    hook: &mut dyn FnMut(Stage) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    with_history_lock(path, || {
        repair_torn_tail(path, MAX_TORN_RECORD_BYTES)?;
        let rotated = rotate_if_full(path, max_bytes)?;
        if rotated {
            sync_parent(path)?;
        }
        hook(Stage::AfterRotation)?;
        // Counters as of the log before this record, repaired by replay if a crash left them behind.
        let mut totals = HistoryTotals::load(path)?;

        // A directory entry changes only when the file is created or was just rotated away.
        let created = !path.exists();
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
        // idle machine. Every other record is made durable before the checkpoint can refer to it, and a
        // new file's directory entry is always committed.
        let routine = is_routine_observation(event);
        if !routine || created {
            file.sync_data()?;
        }
        drop(file);
        if !routine || created {
            sync_parent(path)?;
        }
        hook(Stage::AfterRecordSynced)?;

        let before = totals.clone();
        totals.fold(event);
        let uncovered = HistoryTotals::uncovered_bytes(path);
        if routine
            && !created
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
