//! The resident runtime's archivers for the two durable accounts of an action that are not the gate's: the controller's
//! own journal and the privileged worker's research export. Both are copied (never edited) beside the action study that
//! the records belong to, in the formats `controller_provenance` and `worker_provenance` join to the gate's receipts.
//!
//! * The controller journal holds only its latest record and replaces it on every transition, so it cannot be read back
//!   later. A read-only tap on its durable writes (`set_journal_tap`) keeps the last record seen for each transaction
//!   as it is written; the tap cannot change or veto a write.
//! * The worker's research export is an append-only, hash-chained JSON-lines file in the worker's runtime directory
//! (`daemon::privilege::research_export`). Its lines are read whole: a line that cannot be parsed is an error, never
//! skipped (only an unterminated last line, a write in progress, is set aside), and what is kept is the contiguous run
//! of the chain that covers the study's attempts, so the replay can tell a complete history from a gap.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use super::{ResearchAction, child_verify::AttemptProcess};
use crate::autotune::controller_journal::{ControllerJournalRecord, set_journal_tap};

/// The most transactions kept (the oldest are dropped first).
const MAX_RECORDS: usize = 4_096;
/// The export is read whole up to this many bytes; a larger file cannot be shown complete and is refused.
const MAX_EXPORT_BYTES: u64 = 64 * 1024 * 1024;
/// The worker's clock and the attempts' are both wall clocks read at different moments.
const SLACK_SECONDS: u64 = 5;

/// The controller's records, shared between the journal tap and the study that archives them.
pub type SharedJournalArchive = Arc<Mutex<JournalArchive>>;

#[derive(Debug, Default)]
pub struct JournalArchive {
    records: Vec<ControllerJournalRecord>,
}

impl JournalArchive {
    /// A record the journal just wrote: the newest one for its transaction replaces the older.
    pub fn record(&mut self, record: &ControllerJournalRecord) {
        let Some(id) = record.experiment_id.as_ref() else {
            return;
        };
        if let Some(existing) = self
            .records
            .iter_mut()
            .find(|known| known.experiment_id.as_ref() == Some(id))
        {
            *existing = record.clone();
            return;
        }
        if self.records.len() >= MAX_RECORDS {
            self.records.remove(0);
        }
        self.records.push(record.clone());
    }

    /// The transactions on one of these processes (the target identity names its pid and start time).
    pub fn for_processes(&self, processes: &[AttemptProcess]) -> Vec<ControllerJournalRecord> {
        self.records
            .iter()
            .filter(|record| {
                record.target_identity.as_deref().is_some_and(|identity| {
                    processes.iter().any(|process| {
                        let prefix = format!(
                            "pid:{}:starttime:{}",
                            process.pid.as_u32(),
                            process.start_ticks
                        );
                        identity == prefix || identity.starts_with(&format!("{prefix}:"))
                    })
                })
            })
            .cloned()
            .collect()
    }
}

/// Keep every record the controller journal durably writes from now on in `archive`.
pub fn install_journal_tap(archive: &SharedJournalArchive) {
    let archive = archive.clone();
    set_journal_tap(Some(Arc::new(move |record| {
        if let Ok(mut archive) = archive.lock() {
            archive.record(record);
        }
    })));
}

fn prefix(action: ResearchAction) -> &'static str {
    match action {
        ResearchAction::CpuAffinityProfile => "cpu-affinity-profile:",
        ResearchAction::CgroupPlacement => "cgroup:",
    }
}

/// The worker's research export: in the worker's runtime directory (the socket's own directory), which the controller
/// can traverse and the worker alone can write. The root-private home and the worker's full audit stay private. A
/// worker with no socket directory has no export, and the empty path reads as unavailable.
pub fn worker_export_path(worker_socket: Option<&Path>) -> std::path::PathBuf {
    worker_socket
        .and_then(Path::parent)
        .map(|dir| dir.join(crate::daemon::privilege::research_export::FILE))
        .unwrap_or_default()
}

/// Whether the worker's export can be read by this process at all (`Err` carries why not).
pub fn worker_audit_readable(path: &Path) -> Result<(), std::io::ErrorKind> {
    std::fs::File::open(path)
        .map(|_| ())
        .map_err(|error| error.kind())
}

/// The contiguous run of the worker's export that covers this action's transactions inside the study's attempts: every
/// record from the first to the last whose time and action fall in the window, other actions' records between them
/// included, so the chain can be checked through. A missing file has none; one that cannot be read whole, or any line of
/// it that is not a record, is an error rather than a silent gap.
pub fn worker_transactions_for(
    path: &Path,
    action: ResearchAction,
    processes: &[AttemptProcess],
) -> Result<Vec<serde_json::Value>, &'static str> {
    use std::io::Read;
    let (Some(from), Some(to)) = (
        processes.iter().map(|p| p.started_unix).min(),
        processes.iter().map(|p| p.completed_unix).max(),
    ) else {
        return Ok(Vec::new());
    };
    let (from, to) = (
        from.saturating_sub(SLACK_SECONDS),
        to.saturating_add(SLACK_SECONDS),
    );
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("WorkerTransactionsUnreadable"),
    };
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_EXPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "WorkerTransactionsUnreadable")?;
    if bytes.len() as u64 > MAX_EXPORT_BYTES {
        return Err("WorkerTransactionsTooLarge");
    }
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    // An unterminated last line is a write in progress, not a record; every other line must be one.
    if !bytes.ends_with(b"\n") {
        lines.pop();
    }
    let mut records = Vec::new();
    for line in lines.into_iter().filter(|line| !line.is_empty()) {
        records.push(
            serde_json::from_slice::<serde_json::Value>(line)
                .map_err(|_| "WorkerTransactionsUnreadable")?,
        );
    }
    let covered = |record: &serde_json::Value| {
        let seconds = record["unix_nanos"]
            .as_str()
            .and_then(|nanos| nanos.parse::<u128>().ok())
            .map_or(u64::MAX, |nanos| {
                u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX)
            });
        (from..=to).contains(&seconds)
            && record["action_id"]
                .as_str()
                .is_some_and(|id| id.starts_with(prefix(action)))
    };
    let first = records.iter().position(covered);
    let last = records.iter().rposition(covered);
    Ok(match (first, last) {
        (Some(first), Some(last)) => records[first..=last].to_vec(),
        _ => Vec::new(),
    })
}

/// The tap is process-wide, so tests that install it take turns.
#[cfg(test)]
pub(crate) static TAP_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
#[path = "evidence_archive_tests.rs"]
mod tests;
