//! Tests extracted from the parent module to keep production files below the architecture size gate.

use std::{
    fs,
    path::{Path, PathBuf},
};

use stutter_core::ids::{Pid, Tid};

use super::{
    model::{PROFILE_RESTORE_SCHEMA_VERSION, ProfileRestoreState},
    *,
};

#[test]
fn restore_nice_with_matching_identity() {
    let dir = temp_dir("profile-restore-nice-match");
    write_fake_task_stat(&dir, 10, 11, 100, 111);
    let state = ProfileRestoreState {
        schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
        nice_records: vec![nice_record(11, 10, 100, 111, 5, 10)],
        ..ProfileRestoreState::default()
    };
    let mut restored = Vec::new();

    let (summary, errors) = restore_all_at_with_ops(
        &dir,
        &state,
        |_, _| Ok(()),
        |tid, nice| {
            restored.push((tid, nice));
            Ok(())
        },
        |_, _| Ok(()),
    );

    assert!(errors.is_empty());
    assert_eq!(summary.nice, 1);
    assert_eq!(restored, vec![(Tid::new(11), 5)]);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn skip_nice_restore_on_tid_reuse() {
    let dir = temp_dir("profile-restore-nice-reuse");
    write_fake_task_stat(&dir, 10, 11, 100, 222);
    let state = ProfileRestoreState {
        schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
        nice_records: vec![nice_record(11, 10, 100, 111, 5, 10)],
        ..ProfileRestoreState::default()
    };
    let mut restored = Vec::new();

    let (summary, errors) = restore_all_at_with_ops(
        &dir,
        &state,
        |_, _| Ok(()),
        |tid, nice| {
            restored.push((tid, nice));
            Ok(())
        },
        |_, _| Ok(()),
    );

    assert!(errors.is_empty());
    assert_eq!(summary.nice, 0);
    assert_eq!(summary.skipped_identity_mismatch, 1);
    assert!(restored.is_empty());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn restore_ionice_with_matching_identity() {
    let dir = temp_dir("profile-restore-ionice-match");
    write_fake_task_stat(&dir, 10, 11, 100, 111);
    let state = ProfileRestoreState {
        schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
        ionice_records: vec![ionice_record(11, 10, 100, 111, 0, 16388)],
        ..ProfileRestoreState::default()
    };
    let mut restored = Vec::new();

    let (summary, errors) = restore_all_at_with_ops(
        &dir,
        &state,
        |_, _| Ok(()),
        |_, _| Ok(()),
        |tid, ioprio| {
            restored.push((tid, ioprio));
            Ok(())
        },
    );

    assert!(errors.is_empty());
    assert_eq!(summary.ionice, 1);
    assert_eq!(restored, vec![(Tid::new(11), 0)]);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn profile_restore_v2_typed_ids_preserve_numeric_json_shape() {
    let nice = NiceRestoreRecordV2 {
        tid: Tid::new(11),
        process_pid: Some(Pid::new(10)),
        process_starttime_ticks: Some(100),
        task_starttime_ticks: Some(111),
        comm: Some("task".to_owned()),
        original_nice: 5,
        applied_nice: 10,
    };

    let json = serde_json::to_value(&nice).unwrap();
    assert_eq!(json["tid"], 11);
    assert_eq!(json["process_pid"], 10);

    let decoded: NiceRestoreRecordV2 = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.tid, Tid::new(11));
    assert_eq!(decoded.process_pid, Some(Pid::new(10)));

    let ionice = IoPrioRestoreRecordV2 {
        tid: Tid::new(12),
        process_pid: Some(Pid::new(10)),
        process_starttime_ticks: Some(100),
        task_starttime_ticks: Some(112),
        comm: Some("io-task".to_owned()),
        original_ioprio: 0,
        applied_ioprio: 16_388,
    };

    let json = serde_json::to_value(&ionice).unwrap();
    assert_eq!(json["tid"], 12);
    assert_eq!(json["process_pid"], 10);

    let decoded: IoPrioRestoreRecordV2 = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.tid, Tid::new(12));
    assert_eq!(decoded.process_pid, Some(Pid::new(10)));
}

#[test]
fn skip_ionice_restore_on_tid_reuse() {
    let dir = temp_dir("profile-restore-ionice-reuse");
    write_fake_task_stat(&dir, 10, 11, 999, 111);
    let state = ProfileRestoreState {
        schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
        ionice_records: vec![ionice_record(11, 10, 100, 111, 0, 16388)],
        ..ProfileRestoreState::default()
    };

    let (summary, errors) =
        restore_all_at_with_ops(&dir, &state, |_, _| Ok(()), |_, _| Ok(()), |_, _| Ok(()));

    assert!(errors.is_empty());
    assert_eq!(summary.ionice, 0);
    assert_eq!(summary.skipped_identity_mismatch, 1);
    fs::remove_dir_all(dir).ok();
}

fn nice_record(
    tid: u32,
    process_pid: u32,
    process_starttime_ticks: u64,
    task_starttime_ticks: u64,
    original_nice: i32,
    applied_nice: i32,
) -> NiceRestoreRecordV2 {
    NiceRestoreRecordV2 {
        tid: tid.into(),
        process_pid: Some(process_pid.into()),
        process_starttime_ticks: Some(process_starttime_ticks),
        task_starttime_ticks: Some(task_starttime_ticks),
        comm: Some("task".to_owned()),
        original_nice,
        applied_nice,
    }
}

fn ionice_record(
    tid: u32,
    process_pid: u32,
    process_starttime_ticks: u64,
    task_starttime_ticks: u64,
    original_ioprio: i32,
    applied_ioprio: i32,
) -> IoPrioRestoreRecordV2 {
    IoPrioRestoreRecordV2 {
        tid: tid.into(),
        process_pid: Some(process_pid.into()),
        process_starttime_ticks: Some(process_starttime_ticks),
        task_starttime_ticks: Some(task_starttime_ticks),
        comm: Some("task".to_owned()),
        original_ioprio,
        applied_ioprio,
    }
}

fn write_fake_task_stat(
    proc_root: &Path,
    process_pid: u32,
    tid: u32,
    process_starttime: u64,
    task_starttime: u64,
) {
    let process_dir = proc_root.join(process_pid.to_string());
    fs::create_dir_all(process_dir.join("task").join(tid.to_string())).unwrap();
    fs::write(
        process_dir.join("stat"),
        fake_stat("process", process_starttime),
    )
    .unwrap();
    fs::write(
        process_dir.join("task").join(tid.to_string()).join("stat"),
        fake_stat("task", task_starttime),
    )
    .unwrap();
}

fn fake_stat(comm: &str, starttime: u64) -> String {
    let mut fields = vec!["0".to_owned(); 18];
    fields.push(starttime.to_string());
    format!("1 ({comm}) S {}\n", fields.join(" "))
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}
