use serde_json::{Value, json};

use super::*;
use crate::autotune::research::evidence_producers::WorkerExport;

const ACTION: &str = "cpu-affinity-profile:game-main";

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

/// The receipt of the action on `slot`'s game process, in the gate's own shape.
fn receipt(slot: u64) -> Value {
    let base = 1_100 + slot * 1_000;
    json!({ "pid": 100 + slot, "start_ticks": 50 + slot,
            "permitted_unix": base, "applied_unix": base + 1, "settled_unix": base + 5 })
}

fn receipts() -> Value {
    json!([receipt(0), receipt(1)])
}

/// What the worker exported: for each slot one transaction (prepared, applied, restored) on that slot's process.
fn export(edit: impl FnOnce(&mut WorkerExport)) -> WorkerExport {
    let mut export = WorkerExport::default();
    for slot in 0..2u64 {
        let base = 1_100 + slot * 1_000;
        export.transaction(
            &format!("tx-{slot}"),
            ACTION,
            (100 + slot as u32, 50 + slot),
            (base, base + 1, base + 4),
        );
    }
    edit(&mut export);
    export
}

fn check(records: &[Value]) -> Result<(), &'static str> {
    verify(
        &serde_json::to_vec(records).unwrap(),
        ResearchAction::CpuAffinityProfile,
        &receipts(),
        &processes(),
    )
}

#[test]
fn every_receipt_is_matched_by_one_complete_transaction_on_its_own_process() {
    assert_eq!(check(export(|_| {}).records()), Ok(()));
    // Other actions' records interleaved in the chain do not matter.
    let interleaved = export(|e| {
        e.transaction("other", "cgroup:place:x", (7, 7), (1_150, 1_151, 1_152));
    });
    assert_eq!(check(interleaved.records()), Ok(()));
}

#[test]
fn a_transaction_of_the_same_action_on_another_process_never_stands_in_for_a_receipt() {
    // Slot 1's transaction was issued for an unrelated process in the same interval: the receipt has no transaction.
    let mut export = WorkerExport::default();
    export.transaction("tx-0", ACTION, (100, 50), (1_100, 1_101, 1_104));
    export.transaction("tx-1", ACTION, (9_999, 12), (2_100, 2_101, 2_104));
    assert_eq!(
        check(export.records()),
        Err("WorkerTransactionsDoNotMatchReceipts")
    );
    // The same process but another start time is another process.
    let mut reused = WorkerExport::default();
    reused.transaction("tx-0", ACTION, (100, 50), (1_100, 1_101, 1_104));
    reused.transaction("tx-1", ACTION, (101, 99), (2_100, 2_101, 2_104));
    assert_eq!(
        check(reused.records()),
        Err("WorkerTransactionsDoNotMatchReceipts")
    );
}

#[test]
fn an_apply_no_receipt_accounts_for_is_a_foreign_action() {
    let foreign = export(|e| {
        e.transaction("tx-extra", ACTION, (4_242, 5), (1_300, 1_301, 1_302));
    });
    assert_eq!(
        check(foreign.records()),
        Err("WorkerTransactionsHaveAnUnaccountedApply")
    );
    // One transaction cannot account for two receipts.
    let mut once = WorkerExport::default();
    once.transaction("tx-0", ACTION, (100, 50), (1_100, 1_101, 1_104));
    let both_one_process = json!([receipt(0), {
        "pid": 100, "start_ticks": 50, "permitted_unix": 1_100, "applied_unix": 1_101, "settled_unix": 1_105 }]);
    assert_eq!(
        verify(
            &serde_json::to_vec(once.records()).unwrap(),
            ResearchAction::CpuAffinityProfile,
            &both_one_process,
            &processes()
        ),
        Err("WorkerTransactionsDoNotMatchReceipts")
    );
}

#[test]
fn a_transaction_that_was_not_restored_or_ran_out_of_order_is_refused() {
    let mut unrestored = WorkerExport::default();
    unrestored.push("prepared", "tx-0", ACTION, (100, 50), 1_100);
    unrestored.push("applied", "tx-0", ACTION, (100, 50), 1_101);
    unrestored.transaction("tx-1", ACTION, (101, 51), (2_100, 2_101, 2_104));
    assert_eq!(
        check(unrestored.records()),
        Err("WorkerTransactionNotRestored")
    );
    let mut failed = WorkerExport::default();
    failed.push("prepared", "tx-0", ACTION, (100, 50), 1_100);
    failed.push("applied", "tx-0", ACTION, (100, 50), 1_101);
    failed.push("restore_failed", "tx-0", ACTION, (100, 50), 1_102);
    failed.push("restored", "tx-0", ACTION, (100, 50), 1_103);
    failed.transaction("tx-1", ACTION, (101, 51), (2_100, 2_101, 2_104));
    assert_eq!(check(failed.records()), Err("WorkerTransactionNotRestored"));
    let mut reordered = WorkerExport::default();
    reordered.push("applied", "tx-0", ACTION, (100, 50), 1_101);
    reordered.push("prepared", "tx-0", ACTION, (100, 50), 1_102);
    reordered.push("restored", "tx-0", ACTION, (100, 50), 1_103);
    reordered.transaction("tx-1", ACTION, (101, 51), (2_100, 2_101, 2_104));
    assert_eq!(
        check(reordered.records()),
        Err("WorkerTransactionsDoNotMatchReceipts")
    );
}

#[test]
fn an_altered_dropped_or_lossy_export_is_refused_before_anything_is_joined() {
    let intact = export(|_| {}).records().to_vec();
    assert_eq!(check(&intact), Ok(()));
    let refused = |records: Vec<Value>, why: &'static str| assert_eq!(check(&records), Err(why));
    // A record changed in place.
    let mut altered = intact.clone();
    altered[1]["success"] = json!(false);
    refused(altered, "WorkerTransactionChainBroken");
    // A record removed from the middle.
    let mut dropped = intact.clone();
    dropped.remove(2);
    refused(dropped, "WorkerTransactionChainBroken");
    // The worker said an earlier write was dropped, or that its chain restarted.
    let reseal = |edit: &dyn Fn(&mut Value)| {
        let mut export = WorkerExport::default();
        export.transaction("tx-0", ACTION, (100, 50), (1_100, 1_101, 1_104));
        export.transaction("tx-1", ACTION, (101, 51), (2_100, 2_101, 2_104));
        let mut records = export.records().to_vec();
        let mut body = records[2].clone();
        body.as_object_mut().unwrap().remove("hash");
        edit(&mut body);
        body["hash"] = json!(crate::daemon::privilege::research_export::record_hash(&{
            let mut b = body.clone();
            b.as_object_mut().unwrap().remove("hash");
            b
        }));
        records[2] = body;
        // Later records chain to the old hash, so re-chain them too.
        let mut prev = records[2]["hash"].as_str().unwrap().to_owned();
        for record in records.iter_mut().skip(3) {
            record["prev"] = json!(prev);
            let mut b = record.clone();
            b.as_object_mut().unwrap().remove("hash");
            let hash = crate::daemon::privilege::research_export::record_hash(&b);
            record["hash"] = json!(hash);
            prev = hash;
        }
        records
    };
    refused(
        reseal(&|body| body["dropped_before"] = json!(3)),
        "WorkerTransactionExportLossy",
    );
    refused(
        reseal(&|body| body["chain_restarted"] = json!(true)),
        "WorkerTransactionExportLossy",
    );
    // Not records at all.
    assert_eq!(
        verify(
            b"not json",
            ResearchAction::CpuAffinityProfile,
            &receipts(),
            &processes()
        ),
        Err("WorkerTransactionsUnreadable")
    );
}

/// The controller's transaction is the worker's only when the token the controller kept names it.
#[test]
fn the_controllers_token_digest_must_be_the_transaction_the_worker_applied_on_that_process() {
    use crate::autotune::research::evidence_producers::{
        controller_journal_record, worker_transaction_of,
    };
    let receipts = json!([
        { "transaction": "exp-0", "pid": 100, "start_ticks": 50, "permitted_unix": 1_100, "settled_unix": 1_105 },
        { "transaction": "exp-1", "pid": 101, "start_ticks": 51, "permitted_unix": 2_100, "settled_unix": 2_105 },
    ]);
    let journal = |names: [&str; 2]| {
        serde_json::to_vec(&json!([
            controller_journal_record(names[0], ACTION, (100, 50), (1_100, 1_101)),
            controller_journal_record(names[1], ACTION, (101, 51), (2_100, 2_101)),
        ]))
        .unwrap()
    };
    let worker = |names: [&str; 2]| {
        let mut export = WorkerExport::default();
        for (slot, name) in names.iter().enumerate() {
            let base = 1_100 + slot as u64 * 1_000;
            export.transaction(
                &worker_transaction_of(name),
                ACTION,
                (100 + slot as u32, 50 + slot as u64),
                (base, base + 1, base + 4),
            );
        }
        serde_json::to_vec(export.records()).unwrap()
    };
    let bind = |j: &[u8], w: &[u8]| bind_controller_tokens(j, &receipts, w);
    let same = ["exp-0", "exp-1"];
    assert_eq!(bind(&journal(same), &worker(same)), Ok(()));
    // The worker applied a different transaction on the very process: process identity alone would have passed.
    assert_eq!(
        bind(&journal(same), &worker(["exp-0", "exp-other"])),
        Err("ControllerTransactionIsNotTheWorkers")
    );
    // Two receipts cannot share one worker transaction.
    assert_eq!(
        bind(&journal(["exp-0", "exp-0"]), &worker(["exp-0", "exp-0"])),
        Err("ControllerTransactionIsNotTheWorkers")
    );
    // A journal that kept no token has nothing to bind.
    let tokenless = serde_json::to_vec(&json!([
        { "boot_id": "b", "schema_version": 1, "state": "reverted", "experiment_id": "exp-0", "action_id": ACTION },
        controller_journal_record("exp-1", ACTION, (101, 51), (2_100, 2_101)),
    ]))
    .unwrap();
    assert_eq!(
        bind(&tokenless, &worker(same)),
        Err("ControllerTransactionIsNotTheWorkers")
    );
}
