//! The privileged worker is the only process that applies and restores the action, and it writes its own boundary
//! decisions to the audit log (`daemon_privilege` events: the stage, the verdict, the operation and the action id,
//! with a timestamp from its own clock). The gate's receipts and the controller's journal are both the controller side's
//! account; this is the worker's. The campaign keeps an archive of the worker's audit events for the study's window
//! (the production `AuditEvent` type), and the replay joins it to every receipt: inside that receipt's window the worker
//! allowed exactly one apply of the authorized action's id that touched at least one task and exactly one rollback after
//! it, both successful and neither a dry run, and the worker allowed no other apply of that action during the study's
//! attempts, so nothing it did is unaccounted for. The archive is copied from the worker's audit log by the resident
//! runtime during a physical run; nothing here fabricates one.

use serde_json::Value;

use super::{ResearchAction, child_verify::AttemptProcess};
use crate::audit::AuditEvent;

/// The file, beside the study, holding the archived worker audit events.
pub const FILE: &str = "worker-audit.json";
/// The worker's clock and the gate's are both whole-second wall clocks read at different moments.
const SLACK_SECONDS: u64 = 2;

fn prefix(action: ResearchAction) -> &'static str {
    match action {
        ResearchAction::CpuAffinityProfile => "cpu-affinity-profile:",
        ResearchAction::CgroupPlacement => "cgroup:",
    }
}

fn operation_of(event: &AuditEvent, operation: &str) -> bool {
    event.command == "daemon_privilege"
        && event.message.contains(&format!("operation={operation} "))
        && event.message.contains("verdict=allow")
}

fn seconds(event: &AuditEvent) -> u64 {
    u64::try_from(event.unix_nanos / 1_000_000_000).unwrap_or(u64::MAX)
}

/// Why the archived worker audit does not corroborate these receipts, or `Ok`.
pub fn verify(
    bytes: &[u8],
    action: ResearchAction,
    receipts: &Value,
    processes: &[AttemptProcess],
) -> Result<(), &'static str> {
    let events: Vec<AuditEvent> =
        serde_json::from_slice(bytes).map_err(|_| "WorkerAuditUnreadable")?;
    let receipts = receipts.as_array().ok_or("WorkerAuditUnreadable")?;
    let mine: Vec<&AuditEvent> = events
        .iter()
        .filter(|event| {
            event
                .action_id
                .as_ref()
                .is_some_and(|id| id.as_str().starts_with(prefix(action)))
        })
        .collect();
    let applies: Vec<&&AuditEvent> = mine
        .iter()
        .filter(|event| operation_of(event, "apply_action") && event.success && !event.dry_run)
        .collect();
    let rollbacks: Vec<&&AuditEvent> = mine
        .iter()
        .filter(|event| operation_of(event, "rollback_action") && event.success && !event.dry_run)
        .collect();
    let mut accounted_applies = 0usize;
    for receipt in receipts {
        let (Some(permitted), Some(settled)) = (
            receipt["permitted_unix"].as_u64(),
            receipt["settled_unix"].as_u64(),
        ) else {
            return Err("WorkerAuditDoesNotMatchReceipts");
        };
        let (from, to) = (
            permitted.saturating_sub(SLACK_SECONDS),
            settled.saturating_add(SLACK_SECONDS),
        );
        let within = |event: &&&AuditEvent| (from..=to).contains(&seconds(event));
        let mut applied: Vec<_> = applies.iter().filter(|event| within(event)).collect();
        let mut restored: Vec<_> = rollbacks.iter().filter(|event| within(event)).collect();
        let (Some(apply), None) = (applied.pop(), applied.pop()) else {
            return Err("WorkerAuditDoesNotMatchReceipts");
        };
        let (Some(rollback), None) = (restored.pop(), restored.pop()) else {
            return Err("WorkerAuditNotRestored");
        };
        if apply.affected_tasks == 0 {
            return Err("WorkerAuditAppliedNothing");
        }
        if rollback.unix_nanos < apply.unix_nanos {
            return Err("WorkerAuditNotRestored");
        }
        accounted_applies += 1;
    }
    // An apply the worker allowed during the study's attempts that no receipt accounts for is a foreign action.
    let during_attempts = |event: &&&AuditEvent| {
        processes.iter().any(|process| {
            (process.started_unix..=process.completed_unix).contains(&seconds(event))
        })
    };
    if applies
        .iter()
        .filter(|event| during_attempts(event))
        .count()
        != accounted_applies
    {
        return Err("WorkerAuditHasAnUnaccountedApply");
    }
    Ok(())
}

#[cfg(test)]
#[path = "worker_provenance_tests.rs"]
mod tests;
