//! The privileged worker is the only process that applies and restores the action. It exports its own account of every
//! transaction it issued (`daemon::privilege::research_export`): an append-only, hash-chained record of each
//! transaction's lifecycle naming the exact transaction (the digest of the token the worker issued), the operation and
//! the target process the plan was frozen on. The gate's receipts and the controller's journal are the controller
//! side's account; this is the worker's. The campaign archives the contiguous run of the export that covers the
//! study's attempts, and the replay checks, before it joins anything:
//!
//! - the archived run is internally intact: every record's hash matches its body, each chains to the one before it, the
//!   sequence has no gap, and no record says an earlier write was dropped or the chain restarted;
//!
//! and then for every receipt:
//!
//! - exactly one transaction of the authorized action was issued for the receipt's own process (pid and start
//!   ticks), and its lifecycle is, in order, prepared, applied and restored, each successful, inside the receipt's
//!   window; a transaction of the same action on another process never stands in for it;
//! - no other transaction of the action was applied during the study's attempts, so nothing the worker did is
//!   unaccounted for.
//!
//! The archive is copied from the worker's export by the resident runtime during a physical run; nothing here
//! fabricates one.

use serde_json::Value;

use super::{ResearchAction, child_verify::AttemptProcess};
use crate::daemon::privilege::research_export::{GENESIS, record_hash};

/// The file, beside the study, holding the archived run of the worker's transaction export.
pub const FILE: &str = "worker-transactions.json";
/// The worker's clock and the gate's are both whole-second wall clocks read at different moments.
const SLACK_SECONDS: u64 = 2;

fn prefix(action: ResearchAction) -> &'static str {
    match action {
        ResearchAction::CpuAffinityProfile => "cpu-affinity-profile:",
        ResearchAction::CgroupPlacement => "cgroup:",
    }
}

fn seconds(record: &Value) -> u64 {
    record["unix_nanos"]
        .as_str()
        .and_then(|nanos| nanos.parse::<u128>().ok())
        .map_or(u64::MAX, |nanos| {
            u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX)
        })
}

/// Why the archived run is not an intact, gapless stretch of the worker's chain, or `Ok`.
fn intact(records: &[Value]) -> Result<(), &'static str> {
    let mut expected: Option<(u64, String)> = None;
    for record in records {
        let mut body = record.clone();
        let Some(hash) = body
            .as_object_mut()
            .and_then(|object| object.remove("hash"))
            .and_then(|hash| hash.as_str().map(str::to_owned))
        else {
            return Err("WorkerTransactionsUnreadable");
        };
        let (Some(seq), Some(prev)) = (body["seq"].as_u64(), body["prev"].as_str()) else {
            return Err("WorkerTransactionsUnreadable");
        };
        if record_hash(&body) != hash {
            return Err("WorkerTransactionChainBroken");
        }
        if let Some((last_seq, last_hash)) = &expected
            && (seq != last_seq + 1 || prev != last_hash)
        {
            return Err("WorkerTransactionChainBroken");
        }
        if body["dropped_before"].as_u64().unwrap_or(0) != 0 {
            return Err("WorkerTransactionExportLossy");
        }
        if body.get("chain_restarted").is_some() || (prev == GENESIS && seq != 1) {
            return Err("WorkerTransactionExportLossy");
        }
        expected = Some((seq, hash));
    }
    Ok(())
}

/// Why the archived worker transactions do not corroborate these receipts, or `Ok`.
pub fn verify(
    bytes: &[u8],
    action: ResearchAction,
    receipts: &Value,
    processes: &[AttemptProcess],
) -> Result<(), &'static str> {
    let records: Vec<Value> =
        serde_json::from_slice(bytes).map_err(|_| "WorkerTransactionsUnreadable")?;
    let receipts = receipts.as_array().ok_or("WorkerTransactionsUnreadable")?;
    intact(&records)?;
    let mine: Vec<&Value> = records
        .iter()
        .filter(|record| {
            record["action_id"]
                .as_str()
                .is_some_and(|id| id.starts_with(prefix(action)))
        })
        .collect();
    let mut claimed: Vec<&str> = Vec::new();
    for receipt in receipts {
        let (Some(permitted), Some(settled), Some(pid), Some(start_ticks)) = (
            receipt["permitted_unix"].as_u64(),
            receipt["settled_unix"].as_u64(),
            receipt["pid"].as_u64(),
            receipt["start_ticks"].as_u64(),
        ) else {
            return Err("WorkerTransactionsDoNotMatchReceipts");
        };
        let (from, to) = (
            permitted.saturating_sub(SLACK_SECONDS),
            settled.saturating_add(SLACK_SECONDS),
        );
        // The transactions of this action issued for exactly this process.
        let mut transactions: Vec<&str> = mine
            .iter()
            .filter(|record| {
                record["process"]["pid"].as_u64() == Some(pid)
                    && record["process"]["start_ticks"].as_u64() == Some(start_ticks)
            })
            .filter_map(|record| record["transaction"].as_str())
            .collect();
        transactions.sort_unstable();
        transactions.dedup();
        let Some(&transaction) = transactions.iter().find(|transaction| {
            mine.iter().any(|record| {
                record["transaction"].as_str() == Some(**transaction)
                    && record["event"] == "applied"
                    && (from..=to).contains(&seconds(record))
            })
        }) else {
            return Err("WorkerTransactionsDoNotMatchReceipts");
        };
        if claimed.contains(&transaction) {
            return Err("WorkerTransactionsDoNotMatchReceipts");
        }
        claimed.push(transaction);
        // Its lifecycle, in the worker's own order: prepared, then applied, then restored, each successful.
        let events: Vec<(&str, bool)> = mine
            .iter()
            .filter(|record| record["transaction"].as_str() == Some(transaction))
            .map(|record| {
                (
                    record["event"].as_str().unwrap_or(""),
                    record["success"] == true,
                )
            })
            .collect();
        let position = |name: &str| events.iter().position(|(event, _)| *event == name);
        match (
            position("prepared"),
            position("applied"),
            position("restored"),
        ) {
            (Some(prepared), Some(applied), Some(restored))
                if prepared < applied && applied < restored =>
            {
                if events
                    .iter()
                    .any(|(event, ok)| matches!(*event, "prepared" | "applied" | "restored") && !ok)
                    || events.iter().any(|(event, _)| *event == "restore_failed")
                {
                    return Err("WorkerTransactionNotRestored");
                }
            }
            (Some(_), Some(_), None) => return Err("WorkerTransactionNotRestored"),
            _ => return Err("WorkerTransactionsDoNotMatchReceipts"),
        }
    }
    // A transaction of the action applied during the study's attempts that no receipt claims is a foreign action.
    let during_attempts = |record: &&&Value| {
        processes.iter().any(|process| {
            (process.started_unix..=process.completed_unix).contains(&seconds(record))
        })
    };
    let foreign = mine
        .iter()
        .filter(|record| record["event"] == "applied")
        .filter(during_attempts)
        .filter(|record| {
            record["transaction"]
                .as_str()
                .is_none_or(|transaction| !claimed.contains(&transaction))
        })
        .count();
    if foreign != 0 {
        return Err("WorkerTransactionsHaveAnUnaccountedApply");
    }
    Ok(())
}

/// Whether the controller's transaction for every receipt is the very transaction the worker issued: the digest of the
/// rollback token the controller's journal kept is the transaction the worker's export applied on the receipt's own
/// process, inside the receipt's window, and no two receipts share one. Process identity alone could pair a receipt with
/// a different transaction on the same process; the token digest is what the worker itself named.
pub fn bind_controller_tokens(
    journal: &[u8],
    receipts: &Value,
    worker: &[u8],
) -> Result<(), &'static str> {
    use crate::autotune::controller_journal::ControllerJournalRecord;
    const REFUSED: &str = "ControllerTransactionIsNotTheWorkers";
    let journal: Vec<ControllerJournalRecord> =
        serde_json::from_slice(journal).map_err(|_| "ControllerJournalUnreadable")?;
    let worker: Vec<Value> =
        serde_json::from_slice(worker).map_err(|_| "WorkerTransactionsUnreadable")?;
    let receipts = receipts.as_array().ok_or(REFUSED)?;
    let mut digests: Vec<String> = Vec::new();
    for receipt in receipts {
        let (Some(name), Some(permitted), Some(settled), Some(pid), Some(start_ticks)) = (
            receipt["transaction"].as_str(),
            receipt["permitted_unix"].as_u64(),
            receipt["settled_unix"].as_u64(),
            receipt["pid"].as_u64(),
            receipt["start_ticks"].as_u64(),
        ) else {
            return Err(REFUSED);
        };
        let token = journal
            .iter()
            .find(|record| {
                record
                    .experiment_id
                    .as_ref()
                    .is_some_and(|id| id.as_str() == name)
            })
            .and_then(|record| record.rollback_token.as_ref())
            .ok_or(REFUSED)?;
        let digest = crate::daemon::privilege::rollback_snapshot_digest(token);
        let (from, to) = (
            permitted.saturating_sub(SLACK_SECONDS),
            settled.saturating_add(SLACK_SECONDS),
        );
        let applied = worker.iter().any(|record| {
            record["transaction"].as_str() == Some(digest.as_str())
                && record["event"] == "applied"
                && record["success"] == true
                && record["process"]["pid"].as_u64() == Some(pid)
                && record["process"]["start_ticks"].as_u64() == Some(start_ticks)
                && (from..=to).contains(&seconds(record))
        });
        if !applied || digests.contains(&digest) {
            return Err(REFUSED);
        }
        digests.push(digest);
    }
    Ok(())
}

#[cfg(test)]
#[path = "worker_provenance_tests.rs"]
mod tests;
