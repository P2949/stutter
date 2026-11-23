use std::io::Write;

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
fn the_worker_audit_is_read_line_by_line_and_only_its_own_events_for_the_action_inside_the_study_are_kept()
 {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("actions.jsonl");
    let event = |command: &str, action: &str, at: u64| {
        serde_json::json!({
            "schema_version": 1, "unix_nanos": u128::from(at) * 1_000_000_000, "command": command,
            "action_id": action, "dry_run": false, "success": true, "affected_tasks": 3, "restore_path": null,
            "message": "stage=execution verdict=allow operation=apply_action ",
        })
        .to_string()
    };
    let mut file = std::fs::File::create(&path).unwrap();
    for line in [
        event("daemon_privilege", "cpu-affinity-profile:game", 1_000),
        event("daemon_privilege", "cgroup:place:x", 1_001),
        event("agent", "cpu-affinity-profile:game", 1_002),
        event("daemon_privilege", "cpu-affinity-profile:game", 50_000),
        // A torn line from a concurrent writer is skipped, never repaired.
        "{\"schema_version\":1,\"unix_nanos\":1000000000000,\"comm".to_owned(),
        event("daemon_privilege", "cpu-affinity-profile:game", 1_050),
    ] {
        writeln!(file, "{line}").unwrap();
    }
    drop(file);
    let kept = worker_audit_for(&path, ResearchAction::CpuAffinityProfile, &[process(1)]);
    assert_eq!(
        kept.iter()
            .map(|e| e.unix_nanos / 1_000_000_000)
            .collect::<Vec<_>>(),
        vec![1_000, 1_050]
    );
    // No file, or no attempt, has none.
    assert!(
        worker_audit_for(
            &dir.path().join("absent"),
            ResearchAction::CpuAffinityProfile,
            &[process(1)]
        )
        .is_empty()
    );
    assert!(worker_audit_for(&path, ResearchAction::CpuAffinityProfile, &[]).is_empty());
}

#[test]
fn the_workers_audit_is_found_under_its_own_private_home_and_an_unreadable_one_says_why() {
    use std::path::Path;
    // The socket directory names the user the worker serves; its audit is under that worker's own home.
    assert_eq!(
        worker_audit_path(Some(Path::new("/run/stutter-worker-p2949/worker.sock"))),
        Path::new("/var/lib/stutter/worker-p2949/.local/state/stutter/audit/actions.jsonl")
    );
    // A socket that names no user, or one trying to climb out of the state directory, uses the controller's default.
    for socket in [
        None,
        Some("/tmp/worker.sock"),
        Some("/run/stutter-worker-/worker.sock"),
        Some("/run/stutter-worker-a/b/worker.sock"),
    ] {
        assert_eq!(
            worker_audit_path(socket.map(Path::new)),
            crate::audit::default_audit_log_path(),
            "{socket:?}"
        );
    }
    assert_eq!(
        worker_audit_readable(Path::new("/proc/self/status")),
        Ok(())
    );
    assert_eq!(
        worker_audit_readable(Path::new("/proc/self/definitely-absent")),
        Err(std::io::ErrorKind::NotFound)
    );
}
