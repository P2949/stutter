use serde_json::{Value, json};

use super::*;

fn processes() -> Vec<AttemptProcess> {
    (0..2)
        .map(|slot| AttemptProcess {
            slot,
            pid: stutter_core::ids::Pid::new(100 + slot),
            start_ticks: 50 + u64::from(slot),
            uid: 1000,
            started_unix: 1_000 + u64::from(slot) * 1_000,
            completed_unix: 1_900 + u64::from(slot) * 1_000,
        })
        .collect()
}

fn receipt(slot: u64) -> Value {
    let base = 1_100 + slot * 1_000;
    json!({ "permitted_unix": base, "applied_unix": base + 1, "settled_unix": base + 5 })
}

fn event(operation: &str, at_seconds: u64, extra: &dyn Fn(&mut Value)) -> Value {
    let mut value = json!({
        "schema_version": 1, "unix_nanos": u128::from(at_seconds) * 1_000_000_000, "command": "daemon_privilege",
        "action_id": "cpu-affinity-profile:game-main", "safety_class": "ReversibleLowRisk", "dry_run": false,
        "success": true, "affected_tasks": 12, "restore_path": null,
        "message": format!("stage=execution verdict=allow reason_code=ok caller_role=privileged_worker transport=socket operation={operation} policy_intent=Apply request=apply action_id=x error_category=ok detail=ok"),
    });
    extra(&mut value);
    value
}

fn archive(edit: impl FnOnce(&mut Vec<Value>)) -> Vec<u8> {
    let mut events = Vec::new();
    for slot in 0..2u64 {
        let base = 1_100 + slot * 1_000;
        events.push(event("apply_action", base + 1, &|_| {}));
        events.push(event("rollback_action", base + 4, &|_| {}));
    }
    edit(&mut events);
    serde_json::to_vec(&events).unwrap()
}

fn check(bytes: &[u8]) -> Result<(), &'static str> {
    let receipts = Value::Array((0..2).map(receipt).collect());
    verify(
        bytes,
        ResearchAction::CpuAffinityProfile,
        &receipts,
        &processes(),
    )
}

#[test]
fn every_receipt_needs_the_workers_own_apply_and_rollback_and_the_worker_did_nothing_else() {
    assert_eq!(check(&archive(|_| {})), Ok(()));
    let refused = |edit: &dyn Fn(&mut Vec<Value>), why: &'static str| {
        assert_eq!(check(&archive(edit)), Err(why));
    };
    // The worker never applied, never rolled back, or did either twice in one window.
    refused(
        &|e| {
            e.remove(0);
        },
        "WorkerAuditDoesNotMatchReceipts",
    );
    refused(
        &|e| {
            e.remove(1);
        },
        "WorkerAuditNotRestored",
    );
    refused(
        &|e| e.push(event("apply_action", 1_102, &|_| {})),
        "WorkerAuditDoesNotMatchReceipts",
    );
    refused(
        &|e| e.push(event("rollback_action", 1_103, &|_| {})),
        "WorkerAuditNotRestored",
    );
    // A refused, failed or dry-run apply is not an apply; an apply that touched nothing changed nothing.
    refused(
        &|e| e[0]["success"] = false.into(),
        "WorkerAuditDoesNotMatchReceipts",
    );
    refused(
        &|e| e[0]["dry_run"] = true.into(),
        "WorkerAuditDoesNotMatchReceipts",
    );
    refused(
        &|e| {
            e[0]["message"] = "stage=policy_validation verdict=deny operation=apply_action ".into()
        },
        "WorkerAuditDoesNotMatchReceipts",
    );
    refused(
        &|e| e[0]["affected_tasks"] = 0.into(),
        "WorkerAuditAppliedNothing",
    );
    // Another action's id is not this action's apply.
    refused(
        &|e| e[0]["action_id"] = "cgroup:place:x".into(),
        "WorkerAuditDoesNotMatchReceipts",
    );
    // A rollback before its apply, or outside the receipt's window, is not its restoration.
    refused(
        &|e| e[1] = event("rollback_action", 1_099, &|_| {}),
        "WorkerAuditNotRestored",
    );
    // An apply during the study's attempts that no receipt accounts for is a foreign action.
    refused(
        &|e| e.push(event("apply_action", 1_800, &|_| {})),
        "WorkerAuditHasAnUnaccountedApply",
    );
    // Events outside every attempt (another day's work in the archive) are not this study's.
    assert_eq!(
        check(&archive(|e| e.push(event("apply_action", 9_000, &|_| {})))),
        Ok(())
    );
    assert_eq!(check(b"{"), Err("WorkerAuditUnreadable"));
}
