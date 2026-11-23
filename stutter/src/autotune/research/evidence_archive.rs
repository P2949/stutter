//! The resident runtime's archivers for the two durable accounts of an action that are not the gate's: the controller's
//! own journal and the privileged worker's audit log. Both are copied (never edited) beside the action study that
//! the records belong to, in the formats `controller_provenance` and `worker_provenance` join to the gate's receipts.
//!
//! * The controller journal holds only its latest record and replaces it on every transition, so it cannot be read back
//!   later. A read-only tap on its durable writes (`set_journal_tap`) keeps the last record seen for each transaction
//!   as it is written; the tap cannot change or veto a write.
//! * The worker's audit log is an append-only JSON-lines file; lines are read independently (a torn line from a
//!   concurrent writer is skipped, never repaired) and only the worker's own `daemon_privilege` events for the
//!   authorized action's id inside the study's attempts are kept.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use super::{ResearchAction, child_verify::AttemptProcess};
use crate::{
    audit::AuditEvent,
    autotune::controller_journal::{ControllerJournalRecord, set_journal_tap},
};

/// The most transactions kept (the oldest are dropped first).
const MAX_RECORDS: usize = 4_096;
/// The audit file is read from its end up to this many bytes (a campaign's events are recent).
const MAX_AUDIT_BYTES: u64 = 64 * 1024 * 1024;
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

/// The worker's own audit events for this action inside the study's attempts, from the audit file (`path`). A missing or
/// unreadable file has none.
pub fn worker_audit_for(
    path: &Path,
    action: ResearchAction,
    processes: &[AttemptProcess],
) -> Vec<AuditEvent> {
    use std::io::{Read, Seek, SeekFrom};
    let (Some(from), Some(to)) = (
        processes.iter().map(|p| p.started_unix).min(),
        processes.iter().map(|p| p.completed_unix).max(),
    ) else {
        return Vec::new();
    };
    let (from, to) = (
        from.saturating_sub(SLACK_SECONDS),
        to.saturating_add(SLACK_SECONDS),
    );
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let length = file.metadata().map_or(0, |meta| meta.len());
    let start = length.saturating_sub(MAX_AUDIT_BYTES);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_err()
        || file.take(MAX_AUDIT_BYTES).read_to_end(&mut bytes).is_err()
    {
        return Vec::new();
    }
    bytes
        .split(|byte| *byte == b'\n')
        .filter_map(|line| serde_json::from_slice::<AuditEvent>(line).ok())
        .filter(|event| {
            let seconds = u64::try_from(event.unix_nanos / 1_000_000_000).unwrap_or(u64::MAX);
            event.command == "daemon_privilege"
                && (from..=to).contains(&seconds)
                && event
                    .action_id
                    .as_ref()
                    .is_some_and(|id| id.as_str().starts_with(prefix(action)))
        })
        .collect()
}

/// The tap is process-wide, so tests that install it take turns.
#[cfg(test)]
pub(crate) static TAP_TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
#[path = "evidence_archive_tests.rs"]
mod tests;
