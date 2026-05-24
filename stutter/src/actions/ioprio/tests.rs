//! Tests extracted from the parent module to keep production files below the architecture size gate.

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

fn target(tid: u32, comm: &str, starttime_ticks: u64) -> TaskIdentity {
    TaskIdentity {
        tid,
        process_pid: Some(tid),
        comm: Some(comm.to_owned()),
        starttime_ticks: Some(starttime_ticks),
    }
}

fn target_without_process_pid(tid: u32, comm: &str, starttime_ticks: u64) -> TaskIdentity {
    TaskIdentity {
        tid,
        process_pid: None,
        comm: Some(comm.to_owned()),
        starttime_ticks: Some(starttime_ticks),
    }
}

fn action_for(tid: u32, ioprio: IoPrioValue) -> IoPrioAction {
    IoPrioAction {
        targets: vec![target(tid, "storage-worker", 12345)],
        ioprio,
        policy: permissive_evidence_policy(),
    }
}

fn permissive_evidence_policy() -> IoPrioPolicy {
    IoPrioPolicy {
        allow_ioprio_changes: true,
        allow_realtime_class: false,
        allow_none_class: false,
        max_best_effort_level: 7,
        require_strong_block_io_evidence: true,
        strong_block_io_evidence: true,
    }
}

fn temp_proc_root(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-ioprio-action-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fake_task(proc_root: &Path, tid: u32, comm: &str, starttime_ticks: u64) {
    let task_dir = proc_root.join(tid.to_string());
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
    fs::write(
        task_dir.join("stat"),
        fake_stat_line(tid, comm, starttime_ticks),
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
        action_for(42, IoPrioValue::best_effort(6)).safety_class(),
        SafetyClass::ReversibleMediumRisk
    );
}

#[test]
fn action_id_and_description_include_requested_ioprio() {
    let action = action_for(42, IoPrioValue::best_effort(6));

    assert_eq!(
        action.id(),
        ActionId::new("ioprio:set:best-effort:6:targets:1".to_owned())
    );
    assert_eq!(
        action.describe(),
        "set I/O priority best-effort:6 for task(s) [42]"
    );
}

#[test]
fn encodes_and_decodes_best_effort_ioprio() {
    let value = IoPrioValue::best_effort(4);
    let encoded = value.encode().unwrap();

    assert_eq!(encoded, (2 << IOPRIO_CLASS_SHIFT) | 4);
    assert_eq!(IoPrioValue::decode(encoded).unwrap(), value);
}

#[test]
fn encodes_and_decodes_idle_ioprio() {
    let value = IoPrioValue::idle();
    let encoded = value.encode().unwrap();

    assert_eq!(encoded, 3 << IOPRIO_CLASS_SHIFT);
    assert_eq!(IoPrioValue::decode(encoded).unwrap(), value);
}

#[test]
fn rejects_invalid_level_for_idle_class() {
    let err = IoPrioValue {
        class: IoPrioClass::Idle,
        level: Some(1),
    }
    .encode()
    .unwrap_err()
    .to_string();

    assert!(err.contains("must not specify a level"));
}

#[test]
fn rejects_missing_level_for_best_effort_class() {
    let err = IoPrioValue {
        class: IoPrioClass::BestEffort,
        level: None,
    }
    .encode()
    .unwrap_err()
    .to_string();

    assert!(err.contains("requires level 0..=7"));
}

#[test]
fn rejects_level_above_seven() {
    let err = IoPrioValue::best_effort(8)
        .encode()
        .unwrap_err()
        .to_string();

    assert!(err.contains("outside range 0..=7"));
}

#[test]
fn parses_starttime_from_proc_stat() {
    let stat = fake_stat_line(42, "storage-worker", 98765);

    let parsed = parse_stat_starttime(&stat).unwrap();

    assert_eq!(parsed, 98765);
}

#[test]
fn preflight_rejects_empty_targets() {
    let action = IoPrioAction {
        targets: Vec::new(),
        ioprio: IoPrioValue::best_effort(6),
        policy: permissive_evidence_policy(),
    };

    let err = action
        .preflight_at(
            &temp_proc_root("empty-targets"),
            &permissive_evidence_policy(),
        )
        .unwrap_err()
        .to_string();

    assert!(err.contains("requires at least one explicit target task"));
}

#[test]
fn preflight_rejects_when_policy_disallows_ioprio_changes() {
    let proc_root = temp_proc_root("policy-disallow");
    write_fake_task(&proc_root, 42, "storage-worker", 12345);

    let policy = IoPrioPolicy {
        allow_ioprio_changes: false,
        strong_block_io_evidence: true,
        ..IoPrioPolicy::default()
    };

    let err = action_for(42, IoPrioValue::best_effort(6))
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("policy does not allow I/O priority changes"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_without_strong_block_io_evidence() {
    let proc_root = temp_proc_root("no-evidence");
    write_fake_task(&proc_root, 42, "storage-worker", 12345);

    let policy = IoPrioPolicy {
        allow_ioprio_changes: true,
        require_strong_block_io_evidence: true,
        strong_block_io_evidence: false,
        ..IoPrioPolicy::default()
    };

    let err = action_for(42, IoPrioValue::best_effort(6))
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("strong block I/O evidence is required"));
    assert!(err.contains("investigate-first"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_realtime_class_by_default_policy() {
    let proc_root = temp_proc_root("realtime-policy");
    write_fake_task(&proc_root, 42, "storage-worker", 12345);

    let err = action_for(
        42,
        IoPrioValue {
            class: IoPrioClass::Realtime,
            level: Some(0),
        },
    )
    .preflight_at(&proc_root, &permissive_evidence_policy())
    .unwrap_err()
    .to_string();

    assert!(err.contains("does not allow realtime I/O priority class"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_none_class_by_default_policy() {
    let proc_root = temp_proc_root("none-policy");
    write_fake_task(&proc_root, 42, "storage-worker", 12345);

    let err = action_for(42, IoPrioValue::none())
        .preflight_at(&proc_root, &permissive_evidence_policy())
        .unwrap_err()
        .to_string();

    assert!(err.contains("does not allow resetting I/O priority to class none"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_best_effort_level_above_policy_maximum() {
    let proc_root = temp_proc_root("best-effort-policy");
    write_fake_task(&proc_root, 42, "storage-worker", 12345);

    let policy = IoPrioPolicy {
        max_best_effort_level: 3,
        ..permissive_evidence_policy()
    };

    let err = action_for(42, IoPrioValue::best_effort(6))
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("exceeds policy maximum"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_missing_task() {
    let proc_root = temp_proc_root("missing-task");

    let err = action_for(42, IoPrioValue::best_effort(6))
        .preflight_at(&proc_root, &permissive_evidence_policy())
        .unwrap_err()
        .to_string();

    assert!(err.contains("failed to preflight ioprio target tid=42"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn preflight_rejects_starttime_mismatch_before_ioprio_read() {
    let proc_root = temp_proc_root("starttime-mismatch");
    write_fake_task(&proc_root, 42, "storage-worker", 99999);

    let err = action_for(42, IoPrioValue::best_effort(6))
        .preflight_at(&proc_root, &permissive_evidence_policy())
        .unwrap_err();

    assert!(format!("{:#}", err).contains("starttime mismatch"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn identity_warnings_report_comm_mismatch_and_missing_process_pid() {
    let snapshot = IoPrioTargetSnapshot {
        tid: 42,
        process_pid: None,
        comm: Some("new-comm".to_owned()),
        starttime_ticks: Some(12345),
        exe: None,
        current_ioprio: IoPrioValue::best_effort(4).encode().unwrap(),
        current_value: IoPrioValue::best_effort(4),
    };
    let warnings = identity_warnings(
        &target_without_process_pid(42, "old-comm", 12345),
        &snapshot,
    );

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
}

#[test]
fn rollback_rejects_wrong_token_kind() {
    let action = action_for(42, IoPrioValue::best_effort(6));
    let token = RollbackToken::NiceRestore {
        records: Vec::new(),
    };

    let err = action.rollback(&token).unwrap_err().to_string();

    assert!(err.contains("rollback token is not an I/O priority restore token"));
}

#[test]
fn rollback_token_reports_affected_tasks() {
    let token = RollbackToken::IoPrioRestore {
        records: vec![
            IoPrioRestoreRecord::new(
                TaskRestoreIdentity::observed(1, None, Some("test".to_owned()), None, None),
                IoPrioValue::best_effort(4).encode().unwrap(),
            ),
            IoPrioRestoreRecord::new(
                TaskRestoreIdentity::observed(2, None, Some("test".to_owned()), None, None),
                IoPrioValue::idle().encode().unwrap(),
            ),
        ],
    };

    assert_eq!(token.affected_tasks(), 2);
    assert!(token.restore_path().is_none());
}
