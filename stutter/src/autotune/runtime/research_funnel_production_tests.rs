//! The funnel as the resident session really builds it: the production `build`, not a test builder.

use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::autotune::{
    controller_journal::{
        ControllerJournalRecord, ControllerJournalState, set_journal_tap,
        write_controller_journal_record,
    },
    research::evidence_archive::TAP_TEST_LOCK,
};

fn record(pid: u32) -> ControllerJournalRecord {
    serde_json::from_value(serde_json::json!({
        "boot_id": stutter_core::boot::current_boot_id().unwrap(), "schema_version": 1,
        "state": ControllerJournalState::Reverted, "experiment_id": "production-exp-1",
        "action_id": "cpu-affinity-profile:game",
        "target_identity": format!("pid:{pid}:starttime:77:active_tasks:12"),
        "verify_result": "rollback_verified",
        "rollback_token": { "kind": "cpu-affinity-restore-file", "restore_path": "/x", "affected_tasks": 12 },
        "phase_started_unix_nanos": 1_000_000_000_000u64, "updated_unix_nanos": 1_001_000_000_000u64,
    }))
    .unwrap()
}

/// The production funnel builder hands the runtime a shared resident observer, and installs the journal tap that
/// delivers the controller's own durable records to the archive each action study is sealed with.
#[test]
fn the_production_funnel_is_built_with_its_resident_observer_and_its_journal_archive_wired() {
    let _turn = TAP_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let campaign = tempfile::tempdir().unwrap();
    let study = tempfile::tempdir().unwrap();
    for dir in [&campaign, &study] {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let meta = std::fs::metadata(study.path()).unwrap();
    let root = crate::autotune::research::StudyRootIdentity {
        device: std::os::unix::fs::MetadataExt::dev(&meta),
        inode: std::os::unix::fs::MetadataExt::ino(&meta),
    };
    let (mut intent, _) = fixture();
    intent.study_root = Some(root);
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    let file = std::fs::File::open(campaign.path()).unwrap();
    let host = Box::new(Fake(Arc::new(Mutex::new(Script::default()))));
    let funnel = crate::autotune::runtime::research_control::funnel::build(
        study.path(),
        None,
        &intent,
        &file,
        host,
    )
    .expect("the production funnel builds");
    assert!(funnel.resident().is_some(), "the observer is shared");
    assert!(funnel.telemetry().is_some());
    let archive = funnel.provenance_archive().expect("provenance is wired");

    let journal = study.path().join("controller_journal.json");
    write_controller_journal_record(&journal, &record(32_001)).unwrap();
    let process = crate::autotune::research::child_verify::AttemptProcess {
        slot: 0,
        pid: stutter_core::ids::Pid::new(32_001),
        start_ticks: 77,
        uid: 1000,
        started_unix: 900,
        completed_unix: 1_100,
    };
    let seen = archive.lock().unwrap().for_processes(&[process]);
    set_journal_tap(None);
    assert_eq!(seen.len(), 1, "the controller's record reached the archive");
}
