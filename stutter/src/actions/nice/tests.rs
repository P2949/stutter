
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::actions::{ActionId, RollbackToken, SafetyClass, TaskIdentity, TuningAction};

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

fn action_for(tid: u32, nice: i32) -> NiceAction {
    NiceAction {
        targets: vec![target(tid, "game-thread", 12345)],
        nice,
        policy: NicePolicy::default(),
    }
}

fn temp_proc_root(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-nice-action-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fake_task(proc_root: &Path, tid: u32, comm: &str, nice: i32, starttime_ticks: u64) {
    let task_dir = proc_root.join(tid.to_string());
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
    fs::write(
        task_dir.join("stat"),
        fake_stat_line(tid, comm, nice, starttime_ticks),
    )
    .unwrap();
}

fn fake_stat_line(tid: u32, comm: &str, nice: i32, starttime_ticks: u64) -> String {
    let mut fields = vec!["0".to_owned(); 20];
    fields[0] = "S".to_owned();
    fields[16] = nice.to_string();
    fields[19] = starttime_ticks.to_string();

    format!("{tid} ({comm}) {}", fields.join(" "))
}

#[test]
fn safety_class_is_reversible_medium_risk() {
    assert_eq!(
        action_for(42, 5).safety_class(),
        SafetyClass::ReversibleMediumRisk
    );
}

#[test]
fn action_id_and_description_include_requested_nice() {
    let action = action_for(42, 7);

    assert_eq!(
        action.id(),
        ActionId::new("nice:set:7:targets:1".to_owned())
    );
    assert_eq!(action.describe(), "set nice=7 for task(s) [42]");
}

#[test]
fn parses_nice_and_starttime_from_proc_stat() {
    let stat = fake_stat_line(42, "game-thread", 5, 98765);

    let parsed = parse_stat_nice_and_starttime(&stat).unwrap();

    assert_eq!(parsed, (5, 98765));
}

#[test]
fn preflight_rejects_empty_targets() {
    let action = NiceAction {
        targets: Vec::new(),
        nice: 5,
        policy: NicePolicy::default(),
    };

    let err = action
        .preflight_at(&temp_proc_root("empty-targets"), &NicePolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("requires at least one target task"));
}

#[test]
fn preflight_rejects_nice_outside_linux_range() {
    let action = action_for(42, 20);

    let err = action
        .preflight_at(&temp_proc_root("bad-range"), &NicePolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("outside Linux nice range"));
}

#[test]
fn preflight_rejects_when_policy_disallows_nice_changes() {
    let proc_root = temp_proc_root("policy-disallow");
    write_fake_task(&proc_root, 42, "game-thread", 0, 12345);

    let policy = NicePolicy {
        allow_nice_changes: false,
        ..NicePolicy::default()
    };

    let err = action_for(42, 5)
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("policy does not allow nice changes"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_requested_nice_outside_policy_range() {
    let proc_root = temp_proc_root("policy-range");
    write_fake_task(&proc_root, 42, "game-thread", 0, 12345);

    let policy = NicePolicy {
        allow_nice_changes: true,
        min_nice: 0,
        max_nice: 10,
    };

    let err = action_for(42, -1)
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("outside policy range"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_missing_task() {
    let proc_root = temp_proc_root("missing-task");

    let err = action_for(42, 5)
        .preflight_at(&proc_root, &NicePolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("failed to preflight nice target tid=42"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_starttime_mismatch() {
    let proc_root = temp_proc_root("starttime-mismatch");
    write_fake_task(&proc_root, 42, "game-thread", 0, 99999);

    let err = action_for(42, 5)
        .preflight_at(&proc_root, &NicePolicy::default())
        .unwrap_err();

    assert!(format!("{:#}", err).contains("starttime mismatch"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_warns_on_comm_mismatch_and_missing_process_pid() {
    let proc_root = temp_proc_root("comm-warning");
    write_fake_task(&proc_root, 42, "new-comm", 0, 12345);

    let action = NiceAction {
        targets: vec![target_without_process_pid(42, "old-comm", 12345)],
        nice: 5,
        policy: NicePolicy::default(),
    };

    let warnings = action
        .preflight_at(&proc_root, &NicePolicy::default())
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
    write_fake_task(&proc_root, 42, "game-thread", 0, 12345);

    let state = action_for(42, 5)
        .dry_run_at(&proc_root, &NicePolicy::default())
        .unwrap();

    assert!(!state.applied);
    assert_eq!(state.checked_tasks, 1);
    assert_eq!(state.affected_tasks, 1);
    assert_eq!(state.pending_changes, 1);

    let snapshot = read_target_snapshot_at(&proc_root, &target(42, "game-thread", 12345)).unwrap();
    assert_eq!(snapshot.current_nice, 0);
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn dry_run_reports_zero_pending_when_already_at_requested_nice() {
    let proc_root = temp_proc_root("dry-run-noop");
    write_fake_task(&proc_root, 42, "game-thread", 5, 12345);

    let state = action_for(42, 5)
        .dry_run_at(&proc_root, &NicePolicy::default())
        .unwrap();

    assert!(!state.applied);
    assert_eq!(state.checked_tasks, 1);
    assert_eq!(state.affected_tasks, 0);
    assert_eq!(state.pending_changes, 0);
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn rollback_rejects_wrong_token_kind() {
    let action = action_for(42, 5);
    let token = RollbackToken::IoPrioRestore {
        records: Vec::new(),
    };

    let err = action.rollback(&token).unwrap_err().to_string();

    assert!(err.contains("invalid rollback token"));
    assert!(err.contains("expected nice-restore"));
    assert!(err.contains("actual ioprio-restore"));
}
