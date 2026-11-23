//! The action gate's receipts are the gate's own account of what it permitted and saw. Whether the controller really
//! selected, applied, verified and reverted that action is the controller's own journal's to say, so the campaign
//! keeps an archive of the controller's journal records (`ControllerJournalRecord`, the production type, one final
//! record per transaction) and the replay joins every receipt to its record: the same transaction, the production
//! action's id, the very process, a verified rollback of the recorded token and no fault, all inside the attempt the
//! receipt is attributed to. A receipt the controller's journal does not corroborate, and a journal record for the
//! game that no receipt accounts for, both refuse the study. The archive is copied from the controller's journal by
//! the resident runtime during a physical run; nothing here fabricates one.

use serde_json::Value;

use super::{ResearchAction, child_verify::AttemptProcess};
use crate::autotune::controller_journal::{ControllerJournalRecord, ControllerJournalState};

/// The file, beside the study, holding the archived controller journal records.
pub const FILE: &str = "controller-journal.json";

/// The production action-id prefix of an authorized action.
fn action_prefix(action: ResearchAction) -> &'static str {
    match action {
        ResearchAction::CpuAffinityProfile => "cpu-affinity-profile:",
        ResearchAction::CgroupPlacement => "cgroup:",
    }
}

fn names_process(identity: &str, pid: u64, start_ticks: u64) -> bool {
    let prefix = format!("pid:{pid}:starttime:{start_ticks}");
    identity == prefix || identity.starts_with(&format!("{prefix}:"))
}

/// Why the archived journal does not corroborate these receipts, or `Ok`.
pub fn verify(
    bytes: &[u8],
    action: ResearchAction,
    receipts: &Value,
    processes: &[AttemptProcess],
) -> Result<(), &'static str> {
    let records: Vec<ControllerJournalRecord> =
        serde_json::from_slice(bytes).map_err(|_| "ControllerJournalUnreadable")?;
    let receipts = receipts.as_array().ok_or("ControllerJournalUnreadable")?;
    if records.len() != receipts.len() {
        return Err("ControllerJournalDoesNotMatchReceipts");
    }
    let mut seen = std::collections::BTreeSet::new();
    for receipt in receipts {
        let transaction = receipt["transaction"]
            .as_str()
            .ok_or("ControllerJournalDoesNotMatchReceipts")?;
        let (Some(pid), Some(start_ticks)) =
            (receipt["pid"].as_u64(), receipt["start_ticks"].as_u64())
        else {
            return Err("ControllerJournalDoesNotMatchReceipts");
        };
        let mut matching = records.iter().filter(|record| {
            record
                .experiment_id
                .as_ref()
                .is_some_and(|id| id.as_str() == transaction)
        });
        let (Some(record), None) = (matching.next(), matching.next()) else {
            return Err("ControllerJournalDoesNotMatchReceipts");
        };
        if !seen.insert(transaction.to_owned()) {
            return Err("ControllerJournalDoesNotMatchReceipts");
        }
        if !record
            .action_id
            .as_ref()
            .is_some_and(|id| id.as_str().starts_with(action_prefix(action)))
        {
            return Err("ControllerJournalForAnotherAction");
        }
        if !record
            .target_identity
            .as_deref()
            .is_some_and(|identity| names_process(identity, pid, start_ticks))
        {
            return Err("ControllerJournalForAnotherProcess");
        }
        if record.state != ControllerJournalState::Reverted
            || record.verify_result.as_deref() != Some("rollback_verified")
            || record.rollback_token.is_none()
            || record.fault_reason.is_some()
        {
            return Err("ControllerJournalNotRestored");
        }
        let (Some(begun), Some(ended)) =
            (record.phase_started_unix_nanos, record.updated_unix_nanos)
        else {
            return Err("ControllerJournalNotRestored");
        };
        let process = processes
            .iter()
            .find(|p| u64::from(p.pid.as_u32()) == pid && p.start_ticks == start_ticks)
            .ok_or("ControllerJournalForAnotherProcess")?;
        let seconds = |nanos: u128| u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX);
        if begun > ended
            || seconds(begun) < process.started_unix
            || seconds(ended) > process.completed_unix
        {
            return Err("ControllerJournalOutsideTheAttempt");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "controller_provenance_tests.rs"]
mod tests;
