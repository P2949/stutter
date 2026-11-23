use serde_json::{Value, json};

use super::*;

fn processes() -> Vec<AttemptProcess> {
    (0..2)
        .map(|slot| AttemptProcess {
            slot,
            pid: stutter_core::ids::Pid::new(100 + slot),
            start_ticks: 50 + u64::from(slot),
            uid: 1000,
            started_unix: 1_000,
            completed_unix: 2_200,
        })
        .collect()
}

fn receipt(slot: u32) -> Value {
    json!({ "transaction": format!("exp-{slot}"), "pid": 100 + slot, "start_ticks": 50 + slot })
}

fn record(slot: u32) -> Value {
    json!({
        "boot_id": "b", "schema_version": 1, "state": "reverted",
        "experiment_id": format!("exp-{slot}"),
        "action_id": "cpu-affinity-profile:game-main",
        "target_identity": format!("pid:{}:starttime:{}:active_tasks:12", 100 + slot, 50 + slot),
        "verify_result": "rollback_verified",
        "rollback_token": { "kind": "cpu-affinity-restore-file", "restore_path": "/x/restore.json", "affected_tasks": 12 },
        "phase_started_unix_nanos": 1_100_000_000_000u64, "updated_unix_nanos": 1_150_000_000_000u64,
    })
}

fn check(edit: impl FnOnce(&mut Vec<Value>)) -> Result<(), &'static str> {
    let mut records: Vec<Value> = (0..2).map(record).collect();
    edit(&mut records);
    let receipts = Value::Array((0..2).map(receipt).collect());
    verify(
        &serde_json::to_vec(&records).unwrap(),
        ResearchAction::CpuAffinityProfile,
        &receipts,
        &processes(),
    )
}

#[test]
fn every_receipt_needs_the_controllers_own_corroborating_record_and_no_record_goes_unaccounted() {
    assert_eq!(check(|_| {}), Ok(()));
    let refused = |edit: &dyn Fn(&mut Vec<Value>), why: &'static str| {
        assert_eq!(check(|records| edit(records)), Err(why));
    };
    refused(
        &|r| {
            r.pop();
        },
        "ControllerJournalDoesNotMatchReceipts",
    );
    refused(
        &|r| r.push(record(0)),
        "ControllerJournalDoesNotMatchReceipts",
    );
    refused(
        &|r| r[1] = record(0),
        "ControllerJournalDoesNotMatchReceipts",
    );
    refused(
        &|r| r[0]["experiment_id"] = "exp-9".into(),
        "ControllerJournalDoesNotMatchReceipts",
    );
    refused(
        &|r| r[0]["action_id"] = "cgroup:place:x".into(),
        "ControllerJournalForAnotherAction",
    );
    refused(
        &|r| r[0]["target_identity"] = "pid:999:starttime:1".into(),
        "ControllerJournalForAnotherProcess",
    );
    // A prefix of the pid is not the pid.
    refused(
        &|r| r[0]["target_identity"] = "pid:1000:starttime:50".into(),
        "ControllerJournalForAnotherProcess",
    );
    refused(
        &|r| r[0]["state"] = "keeping".into(),
        "ControllerJournalNotRestored",
    );
    refused(
        &|r| r[0]["verify_result"] = "rollback_failed".into(),
        "ControllerJournalNotRestored",
    );
    refused(
        &|r| {
            r[0].as_object_mut().unwrap().remove("rollback_token");
        },
        "ControllerJournalNotRestored",
    );
    refused(
        &|r| r[0]["fault_reason"] = "restore failed".into(),
        "ControllerJournalNotRestored",
    );
    refused(
        &|r| {
            r[0].as_object_mut().unwrap().remove("updated_unix_nanos");
        },
        "ControllerJournalNotRestored",
    );
    refused(
        &|r| r[0]["updated_unix_nanos"] = 9_000_000_000_000u64.into(),
        "ControllerJournalOutsideTheAttempt",
    );
    refused(
        &|r| r[0]["phase_started_unix_nanos"] = 1_000u64.into(),
        "ControllerJournalOutsideTheAttempt",
    );
    refused(
        &|r| r[0]["phase_started_unix_nanos"] = 1_900_000_000_000u64.into(),
        "ControllerJournalOutsideTheAttempt",
    );
    assert_eq!(
        verify(
            b"{",
            ResearchAction::CpuAffinityProfile,
            &json!([]),
            &processes()
        ),
        Err("ControllerJournalUnreadable")
    );
}
