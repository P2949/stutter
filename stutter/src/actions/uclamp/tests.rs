//! Tests extracted from the parent module to keep production files below the architecture size gate.

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

fn target(tid: u32, comm: &str, starttime_ticks: u64) -> TaskIdentity {
    TaskIdentity {
        tid: tid.into(),
        process_pid: Some((tid).into()),
        comm: Some(comm.to_owned()),
        starttime_ticks: Some(starttime_ticks),
    }
}

fn target_without_process_pid(tid: u32, comm: &str, starttime_ticks: u64) -> TaskIdentity {
    TaskIdentity {
        tid: tid.into(),
        process_pid: None,
        comm: Some(comm.to_owned()),
        starttime_ticks: Some(starttime_ticks),
    }
}

fn action_for(tid: u32, min: Option<u32>, max: Option<u32>) -> UclampAction {
    UclampAction {
        targets: vec![target(tid, "game-thread", 12345)],
        values: UclampValues {
            sched_util_min: min,
            sched_util_max: max,
        },
    }
}

fn temp_proc_root(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-uclamp-action-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fake_task(
    proc_root: &Path,
    tid: u32,
    comm: &str,
    starttime_ticks: u64,
    util_min: u32,
    util_max: u32,
) {
    let task_dir = proc_root.join(tid.to_string());
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
    fs::write(
        task_dir.join("stat"),
        fake_stat_line(tid, comm, starttime_ticks),
    )
    .unwrap();
    fs::write(
            task_dir.join("sched"),
            format!(
                "game-thread ({tid}, #threads: 1)\n-------------------------------------------------------------------\nuclamp.min                                   :                  {util_min}\nuclamp.max                                   :                  {util_max}\neffective uclamp.min                         :                  {util_min}\neffective uclamp.max                         :                  {util_max}\n"
            ),
        )
        .unwrap();
}

fn fake_stat_line(tid: u32, comm: &str, starttime_ticks: u64) -> String {
    let mut fields = vec!["0".to_owned(); 20];
    fields[0] = "S".to_owned();
    fields[19] = starttime_ticks.to_string();

    format!("{tid} ({comm}) {}", fields.join(" "))
}

#[test]
fn safety_class_is_reversible_medium_risk() {
    assert_eq!(
        action_for(42, Some(128), Some(1024)).safety_class(),
        SafetyClass::ReversibleMediumRisk
    );
}

#[test]
fn action_id_and_description_include_requested_values() {
    let action = action_for(42, Some(128), None);

    assert_eq!(
        action.id(),
        ActionId::new("uclamp:set:min=128:max=keep:targets:1".to_owned())
    );
    assert_eq!(
        action.describe(),
        "set uclamp min=128 max=keep for task(s) [42]"
    );
}

#[test]
fn parses_starttime_from_proc_stat() {
    let stat = fake_stat_line(42, "game-thread", 98765);

    let parsed = parse_stat_starttime(&stat).unwrap();

    assert_eq!(parsed, 98765);
}

#[test]
fn parses_uclamp_values_from_sched() {
    let sched = "uclamp.min                                   :                  128\nuclamp.max                                   :                  1024\neffective uclamp.min                         :                  128\neffective uclamp.max                         :                  1024\n";

    let values = parse_sched_uclamp(sched).unwrap();

    assert_eq!(
        values,
        UclampCurrentValues {
            sched_util_min: 128,
            sched_util_max: 1024
        }
    );
}

#[test]
fn preflight_rejects_empty_targets() {
    let action = UclampAction {
        targets: Vec::new(),
        values: UclampValues {
            sched_util_min: Some(128),
            sched_util_max: None,
        },
    };

    let err = action
        .preflight_at(&temp_proc_root("empty-targets"), &UclampPolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("requires at least one explicit target task"));
}

#[test]
fn preflight_rejects_empty_requested_values() {
    let action = action_for(42, None, None);

    let err = action
        .preflight_at(&temp_proc_root("empty-values"), &UclampPolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("requires sched_util_min, sched_util_max, or both"));
}

#[test]
fn preflight_rejects_values_outside_uclamp_range() {
    let action = action_for(42, Some(1025), None);

    let err = action
        .preflight_at(&temp_proc_root("bad-range"), &UclampPolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("outside uclamp range"));
}

#[test]
fn preflight_rejects_min_greater_than_max() {
    let action = action_for(42, Some(900), Some(100));

    let err = action
        .preflight_at(&temp_proc_root("bad-min-max"), &UclampPolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("greater than sched_util_max"));
}

#[test]
fn preflight_rejects_when_policy_disallows_uclamp_changes() {
    let proc_root = temp_proc_root("policy-disallow");
    write_fake_task(&proc_root, 42, "game-thread", 12345, 0, 1024);

    let policy = UclampPolicy {
        allow_uclamp_changes: false,
        ..UclampPolicy::default()
    };

    let err = action_for(42, Some(128), None)
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("policy does not allow uclamp changes"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_when_policy_disallows_per_task_control() {
    let proc_root = temp_proc_root("policy-no-per-task");
    write_fake_task(&proc_root, 42, "game-thread", 12345, 0, 1024);

    let policy = UclampPolicy {
        allow_per_task: false,
        allow_cgroup: true,
        ..UclampPolicy::default()
    };

    let err = action_for(42, Some(128), None)
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("policy does not allow per-task uclamp changes"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_requested_values_outside_policy_range() {
    let proc_root = temp_proc_root("policy-range");
    write_fake_task(&proc_root, 42, "game-thread", 12345, 0, 1024);

    let policy = UclampPolicy {
        allow_uclamp_changes: true,
        min_allowed_util_min: 0,
        max_allowed_util_min: 256,
        min_allowed_util_max: 512,
        max_allowed_util_max: 1024,
        allow_per_task: true,
        allow_cgroup: false,
    };

    let err = action_for(42, Some(512), None)
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("requested sched_util_min 512 is outside policy range"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_missing_task() {
    let proc_root = temp_proc_root("missing-task");

    let err = action_for(42, Some(128), None)
        .preflight_at(&proc_root, &UclampPolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("failed to preflight uclamp target tid=42"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_starttime_mismatch() {
    let proc_root = temp_proc_root("starttime-mismatch");
    write_fake_task(&proc_root, 42, "game-thread", 99999, 0, 1024);

    let err = action_for(42, Some(128), None)
        .preflight_at(&proc_root, &UclampPolicy::default())
        .unwrap_err();

    assert!(format!("{:#}", err).contains("starttime mismatch"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_warns_on_comm_mismatch_and_missing_process_pid() {
    let proc_root = temp_proc_root("comm-warning");
    write_fake_task(&proc_root, 42, "new-comm", 12345, 0, 1024);

    let action = UclampAction {
        targets: vec![target_without_process_pid(42, "old-comm", 12345)],
        values: UclampValues {
            sched_util_min: Some(128),
            sched_util_max: None,
        },
    };

    let warnings = action
        .preflight_at(&proc_root, &UclampPolicy::default())
        .unwrap();

    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("comm changed"))
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.message.contains("no process_pid identity"))
    );
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn dry_run_counts_pending_changes_without_mutating() {
    let proc_root = temp_proc_root("dry-run");
    write_fake_task(&proc_root, 42, "game-thread", 12345, 0, 1024);

    let state = action_for(42, Some(128), Some(900))
        .dry_run_at(&proc_root, &UclampPolicy::default())
        .unwrap();

    assert!(!state.applied);
    assert_eq!(state.checked_tasks, 1);
    assert_eq!(state.affected_tasks, 1);
    assert_eq!(state.pending_changes, 1);

    let snapshot = read_task_uclamp_from_sched_at(&proc_root, 42).unwrap();
    assert_eq!(
        snapshot,
        UclampCurrentValues {
            sched_util_min: 0,
            sched_util_max: 1024
        }
    );
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn dry_run_reports_zero_pending_when_already_at_requested_values() {
    let proc_root = temp_proc_root("dry-run-noop");
    write_fake_task(&proc_root, 42, "game-thread", 12345, 128, 900);

    let state = action_for(42, Some(128), Some(900))
        .dry_run_at(&proc_root, &UclampPolicy::default())
        .unwrap();

    assert!(!state.applied);
    assert_eq!(state.checked_tasks, 1);
    assert_eq!(state.affected_tasks, 0);
    assert_eq!(state.pending_changes, 0);
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn rollback_rejects_wrong_token_kind() {
    let action = action_for(42, Some(128), None);
    let token = RollbackToken::IoPrioRestore {
        records: Vec::new(),
    };

    let err = action.rollback(&token).unwrap_err().to_string();

    assert!(err.contains("rollback token is not a uclamp restore token"));
}

#[test]
fn rollback_token_reports_affected_tasks() {
    let token = RollbackToken::UclampRestore {
        records: vec![
            UclampRestoreRecord::new(
                TaskRestoreIdentity::observed(1, None, Some("test".to_owned()), None, None),
                0,
                1024,
            ),
            UclampRestoreRecord::new(
                TaskRestoreIdentity::observed(2, None, Some("test".to_owned()), None, None),
                128,
                900,
            ),
        ],
    };

    assert_eq!(token.affected_tasks(), 2);
    assert!(token.restore_path().is_none());
}
