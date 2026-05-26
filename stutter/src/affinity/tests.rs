use std::{
    fs,
    path::{Path, PathBuf},
};

use stutter_core::ids::{Pid, Tid};

use super::{
    restore_record::{RESTORE_SCHEMA_VERSION, restore_record_status_at},
    *,
};

#[test]
fn parses_cpu_mask_ranges_and_lists() {
    assert_eq!(CpuMask::parse("0-2,5").unwrap().to_range_string(), "0-2,5");
}

#[test]
fn rejects_empty_cpu_mask() {
    assert!(CpuMask::parse("").is_err());
}

#[test]
fn parses_cpu_ids_above_63() {
    let mask = CpuMask::parse("0,64").unwrap();

    assert_eq!(mask.to_range_string(), "0,64");
}

#[test]
fn serializes_ranges_and_deserializes_legacy_numeric_masks() {
    let mask = CpuMask::parse("0-2,5").unwrap();
    assert_eq!(serde_json::to_string(&mask).unwrap(), r#""0-2,5""#);

    let legacy: CpuMask = serde_json::from_str("39").unwrap();
    assert_eq!(legacy.to_range_string(), "0-2,5");
}

#[test]
fn affinity_record_typed_ids_preserve_numeric_json_shape() {
    let record = AffinityRecord {
        tid: 7.into(),
        process_pid: Some(42.into()),
        process_starttime_ticks: Some(100),
        task_starttime_ticks: Some(200),
        original_mask: CpuMask::parse("0-3").unwrap(),
        applied_mask: CpuMask::parse("0-1").unwrap(),
    };

    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["tid"], 7);
    assert_eq!(json["process_pid"], 42);

    let decoded: AffinityRecord = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.tid, Tid::new(7));
    assert_eq!(decoded.process_pid, Some(Pid::new(42)));
}

#[test]
fn merged_restore_state_preserves_earliest_original_mask() {
    let dir = temp_dir("affinity-merge");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("restore.json");
    save_restore_state(&path, &[affinity_record(7, "0-3", "0-1")]).unwrap();

    save_merged_restore_state(&path, &[affinity_record(7, "0-1", "0")], false).unwrap();

    let state = load_restore_state(&path).unwrap();
    assert_eq!(state.schema_version, RESTORE_SCHEMA_VERSION);
    assert_eq!(state.records.len(), 1);
    assert_eq!(state.records[0].original_mask.to_range_string(), "0-3");
    assert_eq!(state.records[0].applied_mask.to_range_string(), "0");

    fs::remove_dir_all(dir).ok();
}

#[test]
fn restore_all_skips_dead_tids() {
    let (summary, errors) = restore_all(&[affinity_record(i32::MAX as u32, "0", "0")]);

    assert_eq!(summary.restored, 0);
    assert_eq!(summary.skipped_dead, 1);
    assert_eq!(summary.errors, 0);
    assert!(errors.is_empty());
}

#[test]
fn restore_saved_deletes_file_when_only_dead_tids_are_skipped() {
    let dir = temp_dir("affinity-restore-dead");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("restore.json");
    save_restore_state(&path, &[affinity_record(i32::MAX as u32, "0", "0")]).unwrap();

    let summary = restore_saved(&path).unwrap();

    assert_eq!(summary.skipped_dead, 1);
    assert!(!path.exists());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn restore_record_status_verifies_saved_identity() {
    let dir = temp_dir("affinity-identity");
    write_fake_task_stat(&dir, 10, 11, 100, 111);

    let mut record = affinity_record(11, "0", "1");
    record.process_pid = Some(10.into());
    record.process_starttime_ticks = Some(100);
    record.task_starttime_ticks = Some(111);
    assert_eq!(
        restore_record_status_at(&dir, &record).unwrap(),
        RestoreRecordStatus::Verified
    );

    record.task_starttime_ticks = Some(222);
    assert_eq!(
        restore_record_status_at(&dir, &record).unwrap(),
        RestoreRecordStatus::IdentityMismatch
    );

    fs::remove_dir_all(dir).ok();
}

#[test]
fn merged_restore_state_keys_by_task_identity() {
    let dir = temp_dir("affinity-merge-identity");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("restore.json");

    let mut original = affinity_record(7, "0-3", "0-1");
    original.process_pid = Some(7.into());
    original.process_starttime_ticks = Some(70);
    original.task_starttime_ticks = Some(70);
    save_restore_state(&path, &[original]).unwrap();

    let mut same_identity = affinity_record(7, "0-1", "0");
    same_identity.process_pid = Some(7.into());
    same_identity.process_starttime_ticks = Some(70);
    same_identity.task_starttime_ticks = Some(70);
    save_merged_restore_state(&path, &[same_identity], false).unwrap();

    let state = load_restore_state(&path).unwrap();
    assert_eq!(state.records.len(), 1);
    assert_eq!(state.records[0].original_mask.to_range_string(), "0-3");
    assert_eq!(state.records[0].applied_mask.to_range_string(), "0");

    let mut new_identity = affinity_record(7, "1-3", "1");
    new_identity.process_pid = Some(7.into());
    new_identity.process_starttime_ticks = Some(70);
    new_identity.task_starttime_ticks = Some(71);
    save_merged_restore_state(&path, &[new_identity], false).unwrap();

    let state = load_restore_state(&path).unwrap();
    assert_eq!(state.records.len(), 2);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn merged_restore_state_replaces_legacy_same_tid_record() {
    let dir = temp_dir("affinity-merge-legacy");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("restore.json");

    save_restore_state(&path, &[affinity_record(7, "0-3", "0-1")]).unwrap();

    let mut identity_record = affinity_record(7, "1-3", "1");
    identity_record.process_pid = Some(7.into());
    identity_record.process_starttime_ticks = Some(70);
    identity_record.task_starttime_ticks = Some(70);
    save_merged_restore_state(&path, &[identity_record], false).unwrap();

    let state = load_restore_state(&path).unwrap();
    assert_eq!(state.records.len(), 1);
    assert_eq!(state.records[0].original_mask.to_range_string(), "1-3");
    assert_eq!(state.records[0].task_starttime_ticks, Some(70));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn can_round_trip_current_thread_affinity_when_allowed() {
    let Ok(current) = read_allowed_mask_raw(0) else {
        return;
    };
    if current.is_empty() {
        return;
    }

    let Ok(()) = set_affinity_raw(0, &current) else {
        return;
    };

    let reread = read_allowed_mask_raw(0).unwrap();
    assert_eq!(reread, current);
}

#[test]
fn merged_restore_state_preserves_earliest_original_mask_even_if_merge_order_is_swapped() {
    let dir = temp_dir("affinity-merge-swapped");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("restore.json");

    save_restore_state(&path, &[affinity_record(7, "0-1", "0")]).unwrap();

    save_merged_restore_state(&path, &[affinity_record(7, "0-3", "0-1")], false).unwrap();

    let state = load_restore_state(&path).unwrap();
    assert_eq!(state.records.len(), 1);
    assert_eq!(state.records[0].original_mask.to_range_string(), "0-3");
    assert_eq!(state.records[0].applied_mask.to_range_string(), "0");

    fs::remove_dir_all(dir).ok();
}

#[test]
fn merged_restore_state_preserves_earliest_original_mask_during_legacy_upgrade() {
    let dir = temp_dir("affinity-legacy-upgrade");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("restore.json");

    save_restore_state(&path, &[affinity_record(7, "0-3", "0-1")]).unwrap();

    let mut identity = affinity_record(7, "0-1", "0");
    identity.process_pid = Some(7.into());
    identity.process_starttime_ticks = Some(70);
    identity.task_starttime_ticks = Some(70);

    save_merged_restore_state(&path, &[identity], false).unwrap();

    let state = load_restore_state(&path).unwrap();
    assert_eq!(state.records.len(), 1);
    assert_eq!(state.records[0].original_mask.to_range_string(), "0-3");
    assert_eq!(state.records[0].applied_mask.to_range_string(), "0");

    fs::remove_dir_all(dir).ok();
}

fn affinity_record(tid: u32, original_mask: &str, applied_mask: &str) -> AffinityRecord {
    AffinityRecord {
        tid: tid.into(),
        process_pid: None,
        process_starttime_ticks: None,
        task_starttime_ticks: None,
        original_mask: CpuMask::parse(original_mask).unwrap(),
        applied_mask: CpuMask::parse(applied_mask).unwrap(),
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
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    dir
}
