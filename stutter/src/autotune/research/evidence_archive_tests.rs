use super::*;
use crate::autotune::controller_journal::{
    ControllerJournalRecord, ControllerJournalState, write_controller_journal_record,
};

fn record(id: &str, state: ControllerJournalState, pid: u32) -> ControllerJournalRecord {
    serde_json::from_value(serde_json::json!({
        "boot_id": stutter_core::boot::current_boot_id().unwrap(), "schema_version": 1,
        "state": state, "experiment_id": id, "action_id": "cpu-affinity-profile:game",
        "target_identity": format!("pid:{pid}:starttime:77:active_tasks:12"),
        "verify_result": "rollback_verified",
        "rollback_token": { "kind": "cpu-affinity-restore-file", "restore_path": "/x", "affected_tasks": 12 },
        "phase_started_unix_nanos": 1_000_000_000_000u64, "updated_unix_nanos": 1_001_000_000_000u64,
    }))
    .unwrap()
}

fn process(pid: u32) -> AttemptProcess {
    AttemptProcess {
        slot: 0,
        pid: stutter_core::ids::Pid::new(pid),
        start_ticks: 77,
        uid: 1000,
        started_unix: 900,
        completed_unix: 1_100,
    }
}

#[test]
fn the_tap_keeps_the_last_record_the_journal_durably_wrote_for_each_transaction_and_nothing_else() {
    let _turn = TAP_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("controller_journal.json");
    let archive: SharedJournalArchive = Default::default();
    install_journal_tap(&archive);
    // The controller walks one transaction through its states and another on a different process.
    for state in [
        ControllerJournalState::Applying,
        ControllerJournalState::Applied,
        ControllerJournalState::Reverted,
    ] {
        write_controller_journal_record(&path, &record("tap-exp-1", state, 31_001)).unwrap();
    }
    write_controller_journal_record(
        &path,
        &record("tap-exp-2", ControllerJournalState::Reverted, 31_002),
    )
    .unwrap();
    // A clean record (no transaction) is not one.
    write_controller_journal_record(&path, &ControllerJournalRecord::clean()).unwrap();
    let mine = archive.lock().unwrap().for_processes(&[process(31_001)]);
    set_journal_tap(None);
    assert_eq!(mine.len(), 1);
    assert_eq!(
        mine[0].state,
        ControllerJournalState::Reverted,
        "the last state written"
    );
    assert_eq!(
        mine[0].experiment_id.as_ref().unwrap().as_str(),
        "tap-exp-1"
    );
    // The process it is about, and no other.
    assert!(
        archive
            .lock()
            .unwrap()
            .for_processes(&[process(40_000)])
            .is_empty()
    );
}

#[test]
fn the_worker_export_is_read_whole_and_the_run_covering_the_study_is_kept_with_its_neighbours() {
    use crate::autotune::research::evidence_producers::WorkerExport;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("research-transactions.jsonl");
    let action = "cpu-affinity-profile:game";
    let mut export = WorkerExport::default();
    // Before the study, in it (with another action's transaction between), and long after it.
    export.transaction("before", action, (1, 1), (100, 101, 102));
    export.transaction("mine", action, (40_000, 77), (1_000, 1_001, 1_002));
    export.transaction("other", "cgroup:place:x", (2, 2), (1_003, 1_004, 1_005));
    export.transaction("mine-too", action, (40_000, 77), (1_050, 1_051, 1_052));
    export.transaction("after", action, (3, 3), (50_000, 50_001, 50_002));
    let mut text = String::new();
    for record in export.records() {
        text.push_str(&format!("{record}\n"));
    }
    std::fs::write(&path, &text).unwrap();
    let kept = worker_transactions_for(
        &path,
        ResearchAction::CpuAffinityProfile,
        &[process(40_000)],
    )
    .unwrap();
    let names: Vec<_> = kept
        .iter()
        .map(|r| r["transaction"].as_str().unwrap().to_owned())
        .collect();
    // The whole contiguous run from the first to the last covered record, other actions' records included.
    assert_eq!(
        names,
        [
            "mine", "mine", "mine", "other", "other", "other", "mine-too", "mine-too", "mine-too"
        ]
    );
    // No file, or no attempt, has none.
    assert_eq!(
        worker_transactions_for(
            &dir.path().join("absent"),
            ResearchAction::CpuAffinityProfile,
            &[process(1)]
        ),
        Ok(Vec::new())
    );
    assert_eq!(
        worker_transactions_for(&path, ResearchAction::CpuAffinityProfile, &[]),
        Ok(Vec::new())
    );
    // A line that is not a record is an error, never a silent gap; only an unterminated last line (a write in
    // progress) is set aside.
    let mut torn = text.clone();
    torn.push_str("{\"seq\":99,\"ha");
    std::fs::write(&path, &torn).unwrap();
    assert!(
        worker_transactions_for(
            &path,
            ResearchAction::CpuAffinityProfile,
            &[process(40_000)]
        )
        .is_ok()
    );
    let garbled = text.replacen('\n', "\nnot a record\n", 1);
    std::fs::write(&path, garbled).unwrap();
    assert_eq!(
        worker_transactions_for(
            &path,
            ResearchAction::CpuAffinityProfile,
            &[process(40_000)]
        ),
        Err("WorkerTransactionsUnreadable")
    );
}

#[test]
fn the_workers_export_is_found_in_its_runtime_directory_and_an_unreadable_one_says_why() {
    use std::path::Path;
    // The export is beside the worker's socket, in the directory the controller can traverse; the worker's private
    // home and its full audit are not used.
    assert_eq!(
        worker_export_path(Some(Path::new("/run/stutter-worker-p2949/worker.sock"))),
        Path::new("/run/stutter-worker-p2949/research-transactions.jsonl")
    );
    // No socket means no export, and the empty path reads as unavailable.
    let none = worker_export_path(None);
    assert_eq!(none, Path::new(""));
    assert_eq!(
        worker_audit_readable(&none),
        Err(std::io::ErrorKind::NotFound)
    );
    assert_eq!(
        worker_audit_readable(Path::new("/proc/self/status")),
        Ok(())
    );
    assert_eq!(
        worker_audit_readable(Path::new("/proc/self/definitely-absent")),
        Err(std::io::ErrorKind::NotFound)
    );
}
