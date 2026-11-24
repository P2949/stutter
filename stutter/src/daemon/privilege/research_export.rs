//! The worker's own account of each transaction's lifecycle, exported for a research campaign to verify.
//!
//! The privileged worker keeps its audit and token ledger private to root. This is a separate, narrow export: one
//! append-only file, written only by the worker, in the worker's runtime directory (traversable but not writable by
//! the controller), holding nothing but the transaction lifecycle of the worker-issued tokens. Each record names the
//! exact transaction (the digest of the token the worker issued), the event, the operation and the target process
//! identity the plan was frozen on, and chains to the one before it by hash, so a reader can tell a missing, altered
//! or reordered record from a complete history. The export never gates a mutation: a failed write is counted and
//! carried on the next record as `dropped_before`, so a gap is visible without ever blocking the worker.

use std::{
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde_json::{Value, json};

/// The export file inside the worker's runtime directory.
pub const FILE: &str = "research-transactions.jsonl";
/// The `prev` of the first record of a chain.
pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// What happened to a transaction, as the worker itself saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The token was issued and durable before the controller's prepare answer and before the first write.
    Prepared,
    /// The commit completed: the mutation was applied.
    Applied,
    /// The restoration completed and the token is settled.
    Restored,
    /// The restoration was refused or did not settle; the token stays outstanding.
    RestoreFailed,
    /// The prepared transaction was abandoned before its first mutable write.
    Voided,
}

impl Event {
    pub fn as_str(self) -> &'static str {
        match self {
            Event::Prepared => "prepared",
            Event::Applied => "applied",
            Event::Restored => "restored",
            Event::RestoreFailed => "restore_failed",
            Event::Voided => "voided",
        }
    }
}

/// The hash a record chains to: the digest of its serialized body, without its own `hash` member.
pub fn record_hash(body: &Value) -> String {
    stutter_core::hash::sha256_hex(&serde_json::to_vec(body).unwrap_or_default())
}

#[derive(Debug, Default)]
struct State {
    loaded: bool,
    seq: u64,
    prev: String,
    dropped: u64,
    restarted: bool,
    /// The file ends in a line with no newline (a torn write): the next record starts on a line of its own.
    torn_tail: bool,
}

#[derive(Debug)]
pub(super) struct ResearchExport {
    path: Option<PathBuf>,
    state: Mutex<State>,
}

impl ResearchExport {
    /// No export (a worker without a runtime directory of its own).
    pub(super) fn disabled() -> Self {
        Self {
            path: None,
            state: Mutex::new(State::default()),
        }
    }

    /// The export inside `dir`, the worker's runtime directory.
    pub(super) fn in_dir(dir: &Path) -> Self {
        Self {
            path: Some(dir.join(FILE)),
            state: Mutex::new(State::default()),
        }
    }

    /// Append one lifecycle record. Never fails the caller.
    pub(super) fn record(&self, input: Input<'_>) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if !state.loaded {
            Self::load(path, &mut state);
        }
        let mut body = json!({
            "seq": state.seq + 1,
            "prev": state.prev,
            "unix_nanos": crate::audit::unix_nanos_now().to_string(),
            "boot_id": input.boot_id,
            "event": input.event.as_str(),
            "transaction": input.transaction,
            "action_id": input.action_id,
            "operation": input.operation,
            "process": input.process.map(|(pid, start_ticks)| json!({ "pid": pid, "start_ticks": start_ticks })),
            "success": input.success,
            "dropped_before": state.dropped,
        });
        if state.restarted {
            body["chain_restarted"] = json!(true);
        }
        let hash = record_hash(&body);
        body["hash"] = json!(hash);
        let mut line = Vec::new();
        if state.torn_tail {
            line.push(b'\n');
        }
        line.extend(serde_json::to_vec(&body).unwrap_or_default());
        line.push(b'\n');
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o644)
            .open(path)
            .and_then(|mut file| {
                // The worker runs under umask 0077: the mode is set explicitly so the controller can read the export.
                file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
                file.write_all(&line)?;
                file.sync_data()
            });
        match written {
            Ok(()) => {
                state.seq += 1;
                state.prev = hash;
                state.dropped = 0;
                state.restarted = false;
                state.torn_tail = false;
            }
            Err(error) => {
                state.dropped += 1;
                log::warn!("research transaction export could not be written: {error}");
            }
        }
    }

    /// Continue the chain of a file that already exists (a worker restart), or start a new one. A file whose last line
    /// cannot be read as a record is not continued silently: the new chain says it restarted.
    fn load(path: &Path, state: &mut State) {
        state.loaded = true;
        state.prev = GENESIS.to_owned();
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        state.torn_tail = !text.is_empty() && !text.ends_with('\n');
        let Some(last) = text.lines().rev().find(|line| !line.trim().is_empty()) else {
            return;
        };
        let parsed = serde_json::from_str::<Value>(last)
            .ok()
            .and_then(|value| Some((value["seq"].as_u64()?, value["hash"].as_str()?.to_owned())));
        match parsed {
            Some((seq, hash)) => {
                state.seq = seq;
                state.prev = hash;
            }
            None => state.restarted = true,
        }
    }
}

/// What a lifecycle record carries.
pub(super) struct Input<'a> {
    pub event: Event,
    pub transaction: &'a str,
    pub boot_id: &'a str,
    pub action_id: &'a str,
    pub operation: String,
    /// The target process the plan was frozen on: pid and start ticks.
    pub process: Option<(u32, u64)>,
    pub success: bool,
}

#[cfg(test)]
#[path = "research_export_tests.rs"]
mod tests;
