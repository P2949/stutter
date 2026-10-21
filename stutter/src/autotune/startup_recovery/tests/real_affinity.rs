//! Regression contracts use only an owned disposable child; no host workload
//! or user restore state is modified.
use stutter_core::ids::{Pid, Tid};

use super::*;
struct Owned(std::process::Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn child() -> Owned {
    Owned(
        std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    )
}
fn applied(config: &StartupRecoveryConfig, path: PathBuf) {
    let record = ControllerJournalRecord::applied(
        experiment_id("real-affinity-recovery"),
        action_id("cpu-affinity-profile:recovery-child"),
        RollbackToken::CpuAffinityRestoreFile {
            path,
            affected_tasks: 1,
        },
    );
    write_controller_journal_record(&config.journal_path, &record).unwrap();
}
#[test]
fn real_startup_recovery_refuses_unverified_legacy_affinity_before_mutation() {
    let target = child();
    let pid = target.0.id();
    let tid = Tid::new(pid);
    let before = crate::affinity::read_allowed_mask(tid).unwrap();
    let restore_mask =
        crate::affinity::CpuMask::parse(before.to_range_string().split([',', '-']).next().unwrap())
            .unwrap();
    let dir = temp_dir("legacy-unverified-real");
    let config = config_for_dir(&dir, true);
    let path = dir.join("restore.json");
    let state = crate::profile_restore::ProfileRestoreState {
        schema_version: 1,
        affinity_records: vec![crate::affinity::AffinityRecord {
            tid,
            process_pid: None,
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            original_mask: restore_mask,
            applied_mask: before.clone(),
        }],
        ..Default::default()
    };
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    applied(&config, path.clone());
    let outcome = recover_controller_journal_on_startup(config.clone()).unwrap();
    assert!(
        matches!(outcome, StartupRecoveryOutcome::Faulted { .. }),
        "{outcome:?}"
    );
    assert_eq!(crate::affinity::read_allowed_mask(tid).unwrap(), before);
    assert!(path.exists());
    assert!(
        !crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
            .unwrap()
            .is_clean()
    );
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn real_startup_recovery_restores_identified_historical_affinity_format() {
    let target = child();
    let pid = target.0.id();
    let tid = Tid::new(pid);
    let before = crate::affinity::read_allowed_mask(tid).unwrap();
    let changed =
        crate::affinity::CpuMask::parse(before.to_range_string().split([',', '-']).next().unwrap())
            .unwrap();
    let ticks = crate::process_tree::process_starttime_at(Path::new("/proc"), pid).unwrap();
    crate::affinity::set_affinity(tid, &changed).unwrap();
    let dir = temp_dir("historical-identified-real");
    let config = config_for_dir(&dir, true);
    let path = dir.join("restore.json");
    let state = crate::affinity::RestoreState {
        schema_version: 3,
        records: vec![crate::affinity::AffinityRecord {
            tid,
            process_pid: Some(Pid::new(pid)),
            process_starttime_ticks: Some(ticks),
            task_starttime_ticks: Some(ticks),
            original_mask: before.clone(),
            applied_mask: changed,
        }],
    };
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    applied(&config, path);
    let outcome = recover_controller_journal_on_startup(config.clone()).unwrap();
    assert!(
        matches!(
            outcome,
            StartupRecoveryOutcome::Recovered {
                affected_tasks: 1,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(crate::affinity::read_allowed_mask(tid).unwrap(), before);
    assert!(
        crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
            .unwrap()
            .is_clean()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn real_startup_recovery_validates_the_entire_affinity_batch_before_mutation() {
    let target = child();
    let pid = target.0.id();
    let tid = Tid::new(pid);
    let before = crate::affinity::read_allowed_mask(tid).unwrap();
    let ticks = crate::process_tree::process_starttime_at(Path::new("/proc"), pid).unwrap();
    let narrowed =
        crate::affinity::CpuMask::parse(before.to_range_string().split([',', '-']).next().unwrap())
            .unwrap();
    for case in [
        "mixed_legacy",
        "missing_process_ticks",
        "missing_task_ticks",
        "zero_process",
        "empty",
        "future_version",
        "ambiguous",
    ] {
        let dir = temp_dir(case);
        let config = config_for_dir(&dir, true);
        let path = dir.join("restore.json");
        let good = crate::affinity::AffinityRecord {
            tid,
            process_pid: Some(Pid::new(pid)),
            process_starttime_ticks: Some(ticks),
            task_starttime_ticks: Some(ticks),
            original_mask: narrowed.clone(),
            applied_mask: before.clone(),
        };
        let mut bad = good.clone();
        match case {
            "mixed_legacy" => {
                bad.process_pid = None;
                bad.process_starttime_ticks = None;
                bad.task_starttime_ticks = None;
            }
            "missing_process_ticks" => bad.process_starttime_ticks = None,
            "missing_task_ticks" => bad.task_starttime_ticks = None,
            "zero_process" => bad.process_pid = Some(Pid::new(0)),
            _ => {}
        }
        let state = crate::profile_restore::ProfileRestoreState {
            schema_version: if case == "future_version" { 999 } else { 1 },
            affinity_records: if case == "empty" {
                vec![]
            } else {
                vec![good, bad]
            },
            ..Default::default()
        };
        let mut value = serde_json::to_value(state).unwrap();
        if case == "ambiguous" {
            value["records"] = serde_json::json!([]);
        }
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        applied(&config, path.clone());
        let outcome = recover_controller_journal_on_startup(config.clone()).unwrap();
        assert!(
            matches!(outcome, StartupRecoveryOutcome::Faulted { .. }),
            "{case}: {outcome:?}"
        );
        assert_eq!(
            crate::affinity::read_allowed_mask(tid).unwrap(),
            before,
            "{case}"
        );
        assert!(path.exists(), "{case}");
        assert!(
            !crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
                .unwrap()
                .is_clean()
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn real_startup_recovery_skips_a_replacement_identity_without_changing_its_mask() {
    let target = child();
    let pid = target.0.id();
    let tid = Tid::new(pid);
    let before = crate::affinity::read_allowed_mask(tid).unwrap();
    let ticks = crate::process_tree::process_starttime_at(Path::new("/proc"), pid).unwrap();
    let dir = temp_dir("replacement-identity");
    let config = config_for_dir(&dir, true);
    let path = dir.join("restore.json");
    let narrowed =
        crate::affinity::CpuMask::parse(before.to_range_string().split([',', '-']).next().unwrap())
            .unwrap();
    let state = crate::profile_restore::ProfileRestoreState {
        schema_version: 1,
        affinity_records: vec![crate::affinity::AffinityRecord {
            tid,
            process_pid: Some(Pid::new(pid)),
            process_starttime_ticks: Some(ticks + 1),
            task_starttime_ticks: Some(ticks + 1),
            original_mask: narrowed,
            applied_mask: before.clone(),
        }],
        ..Default::default()
    };
    fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    applied(&config, path);
    let outcome = recover_controller_journal_on_startup(config.clone()).unwrap();
    assert!(
        matches!(
            outcome,
            StartupRecoveryOutcome::Recovered {
                affected_tasks: 0,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(crate::affinity::read_allowed_mask(tid).unwrap(), before);
    assert!(
        fs::read_to_string(&config.audit_path)
            .unwrap()
            .contains("skipped_identity_mismatch=1")
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn explicit_manual_legacy_restore_retains_supported_format_compatibility() {
    let target = child();
    let pid = target.0.id();
    let tid = Tid::new(pid);
    let before = crate::affinity::read_allowed_mask(tid).unwrap();
    let narrowed =
        crate::affinity::CpuMask::parse(before.to_range_string().split([',', '-']).next().unwrap())
            .unwrap();
    for version in 0..=3 {
        let dir = temp_dir("manual-legacy");
        let path = dir.join("restore.json");
        let record = crate::affinity::AffinityRecord {
            tid,
            process_pid: None,
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            original_mask: narrowed.clone(),
            applied_mask: before.clone(),
        };
        let (restored, legacy) = if version == 0 {
            let state = crate::profile_restore::ProfileRestoreState {
                schema_version: 1,
                affinity_records: vec![record],
                ..Default::default()
            };
            fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
            let summary = crate::profile_restore::restore_saved(&path).unwrap();
            (summary.affinity, summary.legacy_unverified)
        } else {
            let state = crate::affinity::RestoreState {
                schema_version: version,
                records: vec![record],
            };
            fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
            let summary = crate::affinity::restore_saved(&path).unwrap();
            (summary.restored, summary.legacy_unverified)
        };
        assert_eq!((restored, legacy), (1, 1));
        assert_eq!(crate::affinity::read_allowed_mask(tid).unwrap(), narrowed);
        crate::affinity::set_affinity(tid, &before).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
