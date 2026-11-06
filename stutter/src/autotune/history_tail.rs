//! Bounded reads of the history log for routine readers (status, the TUI panel, metrics, the
//! daemon's recent-decision list). They only need the most recent events, so they read a bounded
//! number of bytes from the end of the file instead of deserializing a whole 128 MiB segment.
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use anyhow::Context;

use super::history::AutotuneHistoryEvent;

/// The most bytes a tail read examines. Several thousand records fit in it.
pub(crate) const TAIL_READ_BYTES: u64 = 8 * 1024 * 1024;

/// The most recent `max_events` events of the current history file.
///
/// A torn final record (a writer died mid-line) is ignored. A malformed complete record is an
/// error, exactly as in the full reader. The first line of a window that starts mid-file is
/// partial and is dropped.
pub fn read_autotune_history_tail(
    path: &Path,
    max_events: usize,
) -> anyhow::Result<Vec<AutotuneHistoryEvent>> {
    read_tail_with_limit(path, max_events, TAIL_READ_BYTES)
}

pub(crate) fn read_tail_with_limit(
    path: &Path,
    max_events: usize,
    max_bytes: u64,
) -> anyhow::Result<Vec<AutotuneHistoryEvent>> {
    if max_events == 0 || !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open autotune history log {}", path.display()))?;
    let length = file.metadata()?.len();
    let start = length.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.split('\n').collect();
    // The text after the last newline is an unterminated record: a torn write, or none at all.
    let torn_tail = lines.pop().unwrap_or("");
    if start > 0 && !lines.is_empty() {
        // The window began mid-line.
        lines.remove(0);
    }
    let mut events = std::collections::VecDeque::with_capacity(max_events.min(4096));
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str::<AutotuneHistoryEvent>(line).with_context(|| {
            format!("failed to parse autotune history event {}", path.display())
        })?;
        event
            .validate_identity_strings()
            .with_context(|| format!("invalid history identity strings in {}", path.display()))?;
        if events.len() == max_events {
            events.pop_front();
        }
        events.push_back(event);
    }
    // A complete final record that merely lacks its newline is still a record.
    if !torn_tail.trim().is_empty()
        && let Ok(event) = serde_json::from_str::<AutotuneHistoryEvent>(torn_tail)
        && event.validate_identity_strings().is_ok()
    {
        if events.len() == max_events {
            events.pop_front();
        }
        events.push_back(event);
    }
    Ok(events.into())
}
