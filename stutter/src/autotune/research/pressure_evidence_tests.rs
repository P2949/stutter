use super::*;
use crate::{
    autotune::research::{
        evidence_producers::WorkerExport,
        pressure_control::{FILE, attested, status},
    },
    metrics::LatencyHistogram,
};

const NOW: u64 = 9_000_000;
const ACTION: &str = "cpu-affinity-profile:game-main";

fn population(start: u64, end: u64, events: u64, delay_ns: u64) -> Population {
    let mut histogram = LatencyHistogram::new();
    for _ in 0..events {
        histogram.record(delay_ns);
    }
    Population {
        start_unix: start,
        end_unix: end,
        histogram: histogram.snapshot(),
        max_ns: delay_ns,
    }
}

fn export(transaction: (u64, u64, u64)) -> Vec<Value> {
    let mut export = WorkerExport::default();
    export.transaction("tx-control", ACTION, (5_000, 100), transaction);
    export.records().to_vec()
}

fn good() -> PressureControlEvidence {
    PressureControlEvidence {
        schema: 1,
        binary_sha256: "aa".repeat(32),
        captured_unix: NOW - 10,
        load: LoadRecord {
            pid: stutter_core::ids::Pid::new(4_242),
            start_ticks: 99,
            uid: 1_000,
            workers: 2,
            started_unix: 1_000,
            stopped_unix: 1_300,
            all_joined: true,
        },
        baseline: population(900, 1_000, 1_500, 2_000_000),
        loaded: population(1_010, 1_290, 1_500, 9_000_000),
        action: ActionRecord {
            receipt: serde_json::json!({
                "action": "CpuAffinityProfile", "pid": 5_000, "start_ticks": 100, "uid": 1_000,
                "permitted_unix": 1_040, "applied_unix": 1_050, "settled_unix": 1_250,
                "original": { "cpus_allowed": "0-15" }, "restored": { "cpus_allowed": "0-15" },
            }),
            worker_transactions: export((1_049, 1_050, 1_250)),
        },
    }
}

#[test]
fn every_field_of_the_receipt_is_derived_from_the_raw_record_and_it_attests() {
    let receipt = reconstruct(&good()).unwrap();
    assert!(receipt.load_owned_by_campaign);
    assert_eq!(receipt.scoped_events, 3_000);
    assert!(receipt.baseline_runnable_p95_ms > 0.0);
    assert!(receipt.loaded_runnable_p95_ms > receipt.baseline_runnable_p95_ms);
    assert!(receipt.action_applied && receipt.restored_independently);
    assert_eq!(attested(&receipt, &"aa".repeat(32), NOW), Ok(()));
}

#[test]
fn a_load_that_never_ended_or_was_not_joined_or_a_window_outside_it_cannot_stand() {
    let derived = |edit: &dyn Fn(&mut PressureControlEvidence)| {
        let mut bad = good();
        edit(&mut bad);
        reconstruct(&bad)
    };
    let attests = |edit: &dyn Fn(&mut PressureControlEvidence)| {
        derived(edit).map(|receipt| attested(&receipt, &"aa".repeat(32), NOW))
    };
    // A load whose workers were not all joined, or that has no end, is not one the campaign owned.
    assert_eq!(
        attests(&|e| e.load.all_joined = false),
        Ok(Err("PressureNotOwnedByCampaign"))
    );
    // A load with no end cannot contain the window counted under it.
    assert_eq!(
        derived(&|e| e.load.stopped_unix = e.load.started_unix).err(),
        Some("PressureWindowsNotOrderedAroundTheLoad")
    );
    // The populations must sit before the load and inside it.
    assert_eq!(
        derived(&|e| e.baseline.end_unix = 1_100).err(),
        Some("PressureWindowsNotOrderedAroundTheLoad")
    );
    assert_eq!(
        derived(&|e| e.loaded.end_unix = 1_400).err(),
        Some("PressureWindowsNotOrderedAroundTheLoad")
    );
    // A population that is not a valid canonical one, or an empty interval, is refused.
    assert_eq!(
        derived(&|e| e.loaded.histogram.pop().map(|_| ()).unwrap_or(())).err(),
        Some("PressurePopulationInvalid")
    );
    assert_eq!(
        derived(&|e| e.loaded.start_unix = e.loaded.end_unix).err(),
        Some("PressurePopulationInvalid")
    );
    // Too few events, and a load that did not press, are named by the receipt's own checks.
    assert_eq!(
        attests(&|e| {
            e.baseline = population(900, 1_000, 100, 2_000_000);
            e.loaded = population(1_010, 1_290, 100, 9_000_000);
        }),
        Ok(Err("PressureControlTooFewEvents"))
    );
    assert_eq!(
        attests(&|e| e.loaded = population(1_010, 1_290, 1_500, 1_000_000)),
        Ok(Err("PressureNotObserved"))
    );
}

#[test]
fn the_action_counts_only_when_the_gate_and_the_worker_agree_it_was_held_and_restored_on_that_process()
 {
    let attests = |edit: &dyn Fn(&mut PressureControlEvidence)| {
        let mut bad = good();
        edit(&mut bad);
        attested(&reconstruct(&bad).unwrap(), &"aa".repeat(32), NOW)
    };
    // Held outside the loaded window: not applied under the load.
    assert_eq!(
        attests(&|e| e.action.receipt["settled_unix"] = 1_500.into()),
        Err("PressureControlActionNotApplied")
    );
    assert_eq!(
        attests(&|e| e.action.receipt["applied_unix"] = 1_005.into()),
        Err("PressureControlActionNotApplied")
    );
    // The worker never shows it restored, or shows another process's transaction, or nothing at all.
    assert_eq!(
        attests(&|e| {
            e.action.worker_transactions.pop();
        }),
        Err("PressureControlActionNotApplied")
    );
    assert_eq!(
        attests(&|e| e.action.receipt["pid"] = 6_000.into()),
        Err("PressureControlActionNotApplied")
    );
    assert_eq!(
        attests(&|e| e.action.worker_transactions.clear()),
        Err("PressureControlActionNotApplied")
    );
    // Applied and worker-confirmed, but the kernel state after is not the state before: not restored.
    assert_eq!(
        attests(&|e| e.action.receipt["restored"] = serde_json::json!({ "cpus_allowed": "0-3" })),
        Err("PressureControlNotRestored")
    );
    assert_eq!(
        attests(&|e| e.action.receipt["original"] = Value::Null),
        Err("PressureControlNotRestored")
    );
}

#[test]
fn the_status_is_derived_from_the_raw_record_and_a_receipt_beside_it_must_say_the_same() {
    let me = "aa".repeat(32);
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(status(dir.path(), &me, NOW), Err("NoPressureControl"));
    let write = |name: &str, bytes: Vec<u8>| std::fs::write(dir.path().join(name), bytes).unwrap();
    write(EVIDENCE_FILE, b"{".to_vec());
    assert_eq!(status(dir.path(), &me, NOW), Err("PressureControlUnusable"));
    write(EVIDENCE_FILE, serde_json::to_vec(&good()).unwrap());
    assert_eq!(status(dir.path(), &me, NOW), Ok(()));
    // A receipt that states what the record supports is accepted; one that claims more is not.
    let derived = reconstruct(&good()).unwrap();
    write(FILE, serde_json::to_vec(&derived).unwrap());
    assert_eq!(status(dir.path(), &me, NOW), Ok(()));
    let mut inflated = derived;
    inflated.scoped_events += 1;
    write(FILE, serde_json::to_vec(&inflated).unwrap());
    assert_eq!(
        status(dir.path(), &me, NOW),
        Err("PressureControlNotFromEvidence")
    );
    // A receipt with no raw record behind it is nothing.
    std::fs::remove_file(dir.path().join(EVIDENCE_FILE)).unwrap();
    assert_eq!(status(dir.path(), &me, NOW), Err("NoPressureControl"));
}
