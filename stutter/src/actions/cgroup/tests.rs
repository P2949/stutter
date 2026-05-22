use std::{
    fs, io,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context as _;

use super::*;
use crate::{actions::TaskIdentity, process_tree::TaskClass};

fn target(tid: u32, comm: &str, starttime_ticks: u64) -> TaskIdentity {
    TaskIdentity {
        tid,
        process_pid: Some(tid),
        comm: Some(comm.to_owned()),
        starttime_ticks: Some(starttime_ticks),
    }
}

fn placement_target(tid: u32, comm: &str, class: TaskClass) -> CgroupPlacementTarget {
    CgroupPlacementTarget {
        identity: target(tid, comm, 12345),
        class,
    }
}

fn action_for(root: &Path, tid: u32) -> CgroupPlacementAction {
    CgroupPlacementAction {
        cgroup_root: root.to_path_buf(),
        target_cgroup: PathBuf::from("/stutter/game.slice"),
        targets: vec![placement_target(tid, "game-thread", TaskClass::Game)],
        cpuset_cpus: Some("2-3".to_owned()),
        cpuset_mems: Some("0".to_owned()),
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-cgroup-action-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_fake_task(proc_root: &Path, tid: u32, comm: &str, starttime_ticks: u64, cgroup: &str) {
    let task_dir = proc_root.join(tid.to_string());
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
    fs::write(
        task_dir.join("status"),
        format!("Name:\t{comm}\nTgid:\t{tid}\nPid:\t{tid}\n"),
    )
    .unwrap();
    fs::write(
        task_dir.join("stat"),
        fake_stat_line(tid, comm, starttime_ticks),
    )
    .unwrap();
    fs::write(task_dir.join("cgroup"), format!("0::{cgroup}\n")).unwrap();
}

fn fake_stat_line(tid: u32, comm: &str, starttime_ticks: u64) -> String {
    let mut fields = vec!["0".to_owned(); 20];
    fields[0] = "S".to_owned();
    fields[19] = starttime_ticks.to_string();

    format!("{tid} ({comm}) {}", fields.join(" "))
}

fn write_fake_cgroup(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative.trim_start_matches('/'));
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("cgroup.procs"), "").unwrap();
    fs::write(path.join("cpuset.cpus"), "0-7\n").unwrap();
    fs::write(path.join("cpuset.mems"), "0\n").unwrap();
    path
}

struct FailingCgroupWriter {
    writes: usize,
    fail_on_write: usize,
}

impl FailingCgroupWriter {
    fn fail_on_write(fail_on_write: usize) -> Self {
        Self {
            writes: 0,
            fail_on_write,
        }
    }
}

impl CgroupFileWriter for FailingCgroupWriter {
    fn write_trimmed(&mut self, path: &Path, value: &str) -> anyhow::Result<()> {
        self.writes += 1;
        if self.writes == self.fail_on_write {
            anyhow::bail!("injected cgroup write failure for {}", path.display());
        }

        super::write_trimmed(path, value)
    }
}

#[test]
fn cgroup_partial_token_keeps_cpuset_restore_state() {
    let cpuset = CgroupCpusetRestoreRecord {
        cgroup_path: PathBuf::from("/stutter/game.slice"),
        original_cpuset_cpus: Some("0-7".to_owned()),
        original_cpuset_mems: Some("0".to_owned()),
    };

    let token = cgroup_partial_token(Vec::new(), true, &Some(cpuset.clone()))
        .expect("cpuset mutation should produce a rollback token");

    let RollbackToken::CgroupRestore {
        records,
        cpuset: restored_cpuset,
    } = token
    else {
        panic!("expected cgroup rollback token");
    };
    assert!(records.is_empty());
    assert_eq!(restored_cpuset, Some(cpuset));
}

#[test]
fn rollback_restores_cpuset_files_from_token() {
    let proc_root = temp_dir("proc-cpuset-rollback");
    let cgroup_root = temp_dir("cgroup-cpuset-rollback");
    let target = write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    fs::write(target.join("cpuset.cpus"), "2-3\n").unwrap();
    fs::write(target.join("cpuset.mems"), "1\n").unwrap();
    let action = action_for(&cgroup_root, 42);
    let token = RollbackToken::CgroupRestore {
        records: Vec::new(),
        cpuset: Some(CgroupCpusetRestoreRecord {
            cgroup_path: PathBuf::from("/stutter/game.slice"),
            original_cpuset_cpus: Some("0-7".to_owned()),
            original_cpuset_mems: Some("0".to_owned()),
        }),
    };

    action.rollback_at(&proc_root, &token).unwrap();

    assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
    assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn detects_dead_task_io_errors_through_anyhow_context() {
    let missing = Err::<(), _>(io::Error::new(io::ErrorKind::NotFound, "task gone"))
        .context("failed to write cgroup.procs")
        .unwrap_err();
    assert!(is_dead_task_io_error(&missing));

    let esrch = Err::<(), _>(io::Error::from_raw_os_error(libc::ESRCH))
        .context("failed to move task")
        .unwrap_err();
    assert!(is_dead_task_io_error(&esrch));

    let permission_denied = Err::<(), _>(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "cgroup.procs not writable",
    ))
    .context("failed to write cgroup.procs")
    .unwrap_err();
    assert!(!is_dead_task_io_error(&permission_denied));
}

#[test]
fn safety_class_is_reversible_medium_risk() {
    let root = temp_dir("safety");
    write_fake_cgroup(&root, "/stutter/game.slice");

    assert_eq!(
        action_for(&root, 42).safety_class(),
        SafetyClass::ReversibleMediumRisk
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn action_id_and_description_include_target_cgroup() {
    let root = temp_dir("id");
    write_fake_cgroup(&root, "/stutter/game.slice");
    let action = action_for(&root, 42);

    assert_eq!(
        action.id(),
        ActionId::new("cgroup:place:/stutter/game.slice:targets:1".to_owned())
    );
    assert_eq!(
        action.describe(),
        "move task(s) [42] to cgroup /stutter/game.slice"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn preflight_accepts_fake_nested_cgroup_files() {
    let proc_root = temp_dir("proc-nested");
    let cgroup_root = temp_dir("cgroup-nested");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

    let warnings = action_for(&cgroup_root, 42)
        .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
        .unwrap();

    assert!(warnings.is_empty());
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn preflight_rejects_missing_target_cgroup() {
    let proc_root = temp_dir("proc-missing-cgroup");
    let cgroup_root = temp_dir("cgroup-missing-cgroup");
    write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

    let err = action_for(&cgroup_root, 42)
        .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("target cgroup does not exist"));
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn preflight_rejects_nested_cgroup_when_policy_disallows_it() {
    let proc_root = temp_dir("proc-no-nested");
    let cgroup_root = temp_dir("cgroup-no-nested");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

    let policy = CgroupPlacementPolicy {
        allow_nested_cgroups: false,
        ..CgroupPlacementPolicy::default()
    };

    let err = action_for(&cgroup_root, 42)
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("policy does not allow nested cgroups"));
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn preflight_rejects_cpuset_when_policy_disallows_it() {
    let proc_root = temp_dir("proc-no-cpuset");
    let cgroup_root = temp_dir("cgroup-no-cpuset");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

    let policy = CgroupPlacementPolicy {
        allow_cpuset_changes: false,
        ..CgroupPlacementPolicy::default()
    };

    let err = action_for(&cgroup_root, 42)
        .preflight_at(&proc_root, &policy)
        .unwrap_err()
        .to_string();

    assert!(err.contains("policy does not allow cpuset changes"));
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn preflight_rejects_missing_cgroup_procs_permission_file() {
    let proc_root = temp_dir("proc-no-procs");
    let cgroup_root = temp_dir("cgroup-no-procs");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    let target = cgroup_root.join("stutter/game.slice");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("cpuset.cpus"), "0-7\n").unwrap();
    fs::write(target.join("cpuset.mems"), "0\n").unwrap();
    write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

    let err = action_for(&cgroup_root, 42)
        .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("required cgroup file does not exist"));
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn preflight_rejects_system_critical_task_classes() {
    for class in [
        TaskClass::AudioRealtime,
        TaskClass::Input,
        TaskClass::KernelThread,
        TaskClass::IrqThread,
        TaskClass::Service,
        TaskClass::NetworkDaemon,
        TaskClass::StorageDaemon,
        TaskClass::Unknown,
    ] {
        let proc_root = temp_dir("proc-critical");
        let cgroup_root = temp_dir("cgroup-critical");
        write_fake_cgroup(&cgroup_root, "/old.slice");
        write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
        write_fake_task(&proc_root, 42, "critical", 12345, "/old.slice");

        let mut action = action_for(&cgroup_root, 42);
        action.targets = vec![placement_target(42, "critical", class)];

        let err = action
            .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
            .unwrap_err()
            .to_string();

        assert!(err.contains("refusing to move system/critical task class"));
        fs::remove_dir_all(proc_root).ok();
        fs::remove_dir_all(cgroup_root).ok();
    }
}

#[test]
fn preflight_rejects_starttime_mismatch() {
    let proc_root = temp_dir("proc-starttime");
    let cgroup_root = temp_dir("cgroup-starttime");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    write_fake_task(&proc_root, 42, "game-thread", 99999, "/old.slice");

    let err = action_for(&cgroup_root, 42)
        .preflight_at(&proc_root, &CgroupPlacementPolicy::default())
        .unwrap_err();

    assert!(format!("{:#}", err).contains("starttime mismatch"));
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn dry_run_counts_pending_move_and_cpuset_changes_without_mutating() {
    let proc_root = temp_dir("proc-dry");
    let cgroup_root = temp_dir("cgroup-dry");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    let target = write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");

    let state = action_for(&cgroup_root, 42)
        .dry_run_at(&proc_root, &CgroupPlacementPolicy::default())
        .unwrap();

    assert!(!state.applied);
    assert_eq!(state.checked_tasks, 1);
    assert_eq!(state.affected_tasks, 2);
    assert_eq!(state.pending_changes, 2);
    assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
    assert_eq!(read_trimmed(&target.join("cgroup.procs")).unwrap(), "");
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn apply_restores_cpuset_cpus_when_cpuset_mems_write_fails() {
    let proc_root = temp_dir("proc-apply-mems-fail");
    let cgroup_root = temp_dir("cgroup-apply-mems-fail");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    let target = write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    write_fake_task(&proc_root, 42, "game-thread", 12345, "/old.slice");
    let action = action_for(&cgroup_root, 42);
    let mut writer = FailingCgroupWriter::fail_on_write(2);

    let err = action
        .apply_at_with_writer(&proc_root, &CgroupPlacementPolicy::default(), &mut writer)
        .unwrap_err();

    assert!(format!("{:#}", err.source).contains("cpuset.mems"));
    assert!(err.rollback.is_some());
    assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
    assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
    assert_eq!(read_trimmed(&target.join("cgroup.procs")).unwrap(), "");
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn apply_restores_moved_tasks_and_cpuset_when_second_task_move_fails() {
    let proc_root = temp_dir("proc-apply-second-move-fail");
    let cgroup_root = temp_dir("cgroup-apply-second-move-fail");
    write_fake_cgroup(&cgroup_root, "/old-first.slice");
    write_fake_cgroup(&cgroup_root, "/old-second.slice");
    let target = write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    write_fake_task(&proc_root, 41, "first", 12345, "/old-first.slice");
    write_fake_task(&proc_root, 42, "second", 12345, "/old-second.slice");

    let mut action = action_for(&cgroup_root, 41);
    action.targets = vec![
        placement_target(41, "first", TaskClass::Game),
        placement_target(42, "second", TaskClass::Game),
    ];
    let mut writer = FailingCgroupWriter::fail_on_write(4);

    let err = action
        .apply_at_with_writer(&proc_root, &CgroupPlacementPolicy::default(), &mut writer)
        .unwrap_err();

    assert!(format!("{:#}", err.source).contains("failed to move tid=42"));
    assert!(err.rollback.is_some());
    assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
    assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
    assert_eq!(
        read_trimmed(&cgroup_root.join("old-first.slice/cgroup.procs")).unwrap(),
        "41"
    );
    assert_eq!(
        read_trimmed(&cgroup_root.join("old-second.slice/cgroup.procs")).unwrap(),
        ""
    );
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn verify_reports_exited_task_as_warning_not_crash() {
    let proc_root = temp_dir("proc-exited-verify");
    let cgroup_root = temp_dir("cgroup-exited-verify");
    write_fake_cgroup(&cgroup_root, "/stutter/game.slice");

    let state = action_for(&cgroup_root, 42)
        .verify_at(&proc_root, &CgroupPlacementPolicy::default())
        .unwrap();

    assert!(!state.applied);
    assert_eq!(state.checked_tasks, 0);
    assert_eq!(state.affected_tasks, 0);
    assert!(state.warnings.iter().any(|warning| {
        warning
            .message
            .contains("exited before cgroup placement verify")
    }));

    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn rollback_skips_exited_tasks_cleanly() {
    let proc_root = temp_dir("proc-rollback-exited");
    let cgroup_root = temp_dir("cgroup-rollback-exited");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    let action = action_for(&cgroup_root, 42);

    let token = RollbackToken::CgroupRestore {
        records: vec![CgroupRestoreRecord::new(
            crate::actions::TaskRestoreIdentity::observed(
                42,
                None,
                Some("test".to_owned()),
                None,
                None,
            ),
            PathBuf::from("/old.slice"),
        )],
        cpuset: None,
    };

    action.rollback_at(&proc_root, &token).unwrap();

    assert_eq!(
        read_trimmed(&cgroup_root.join("old.slice/cgroup.procs")).unwrap(),
        ""
    );
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn rollback_skips_identity_mismatch_without_restoring_reused_tid() {
    let proc_root = temp_dir("proc-rollback-reused");
    let cgroup_root = temp_dir("cgroup-rollback-reused");
    write_fake_task(&proc_root, 42, "other-thread", 99999, "/stutter/game.slice");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    let action = action_for(&cgroup_root, 42);

    let token = RollbackToken::CgroupRestore {
        records: vec![CgroupRestoreRecord::new(
            crate::actions::TaskRestoreIdentity::observed(
                42,
                None,
                Some("game-thread".to_owned()),
                Some(12345),
                None,
            ),
            PathBuf::from("/old.slice"),
        )],
        cpuset: None,
    };

    action.rollback_at(&proc_root, &token).unwrap();

    assert_eq!(
        read_trimmed(&cgroup_root.join("old.slice/cgroup.procs")).unwrap(),
        ""
    );
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn rollback_skips_dead_middle_task_and_restores_remaining_records() {
    let proc_root = temp_dir("proc-rollback-dead-middle");
    let cgroup_root = temp_dir("cgroup-rollback-dead-middle");
    write_fake_task(&proc_root, 41, "first", 4100, "/stutter/game.slice");
    write_fake_task(&proc_root, 43, "third", 4300, "/stutter/game.slice");
    write_fake_cgroup(&cgroup_root, "/old-first.slice");
    write_fake_cgroup(&cgroup_root, "/old-second.slice");
    write_fake_cgroup(&cgroup_root, "/old-third.slice");
    let action = action_for(&cgroup_root, 41);

    let token = RollbackToken::CgroupRestore {
        records: vec![
            CgroupRestoreRecord::new(
                crate::actions::TaskRestoreIdentity::observed(
                    41,
                    None,
                    Some("first".to_owned()),
                    Some(4100),
                    None,
                ),
                PathBuf::from("/old-first.slice"),
            ),
            CgroupRestoreRecord::new(
                crate::actions::TaskRestoreIdentity::observed(
                    42,
                    None,
                    Some("second".to_owned()),
                    Some(4200),
                    None,
                ),
                PathBuf::from("/old-second.slice"),
            ),
            CgroupRestoreRecord::new(
                crate::actions::TaskRestoreIdentity::observed(
                    43,
                    None,
                    Some("third".to_owned()),
                    Some(4300),
                    None,
                ),
                PathBuf::from("/old-third.slice"),
            ),
        ],
        cpuset: None,
    };

    action.rollback_at(&proc_root, &token).unwrap();

    assert_eq!(
        read_trimmed(&cgroup_root.join("old-first.slice/cgroup.procs")).unwrap(),
        "41"
    );
    assert_eq!(
        read_trimmed(&cgroup_root.join("old-second.slice/cgroup.procs")).unwrap(),
        ""
    );
    assert_eq!(
        read_trimmed(&cgroup_root.join("old-third.slice/cgroup.procs")).unwrap(),
        "43"
    );
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn rollback_reports_real_write_failure_after_attempting_remaining_records() {
    let proc_root = temp_dir("proc-rollback-write-failure");
    let cgroup_root = temp_dir("cgroup-rollback-write-failure");
    write_fake_task(&proc_root, 41, "first", 4100, "/stutter/game.slice");
    write_fake_task(&proc_root, 43, "third", 4300, "/stutter/game.slice");
    write_fake_cgroup(&cgroup_root, "/old-third.slice");
    let action = action_for(&cgroup_root, 41);

    let token = RollbackToken::CgroupRestore {
        records: vec![
            CgroupRestoreRecord::new(
                crate::actions::TaskRestoreIdentity::observed(
                    41,
                    None,
                    Some("first".to_owned()),
                    Some(4100),
                    None,
                ),
                PathBuf::from("/missing-old.slice"),
            ),
            CgroupRestoreRecord::new(
                crate::actions::TaskRestoreIdentity::observed(
                    43,
                    None,
                    Some("third".to_owned()),
                    Some(4300),
                    None,
                ),
                PathBuf::from("/old-third.slice"),
            ),
        ],
        cpuset: None,
    };

    let err = action.rollback_at(&proc_root, &token).unwrap_err();

    assert!(format!("{err:#}").contains("after attempting all records"));
    assert_eq!(
        read_trimmed(&cgroup_root.join("old-third.slice/cgroup.procs")).unwrap(),
        "43"
    );
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn rollback_rejects_wrong_token_kind() {
    let cgroup_root = temp_dir("cgroup-wrong-token");
    write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    let action = action_for(&cgroup_root, 42);

    let err = action
        .rollback(&RollbackToken::NiceRestore {
            records: Vec::new(),
        })
        .unwrap_err()
        .to_string();

    assert!(err.contains("rollback token is not a cgroup restore token"));
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn cgroup_restore_token_reports_affected_tasks_and_no_restore_path() {
    let token = RollbackToken::CgroupRestore {
        records: vec![
            CgroupRestoreRecord::new(
                crate::actions::TaskRestoreIdentity::observed(
                    1,
                    None,
                    Some("test".to_owned()),
                    None,
                    None,
                ),
                PathBuf::from("/a.slice"),
            ),
            CgroupRestoreRecord::new(
                crate::actions::TaskRestoreIdentity::observed(
                    2,
                    None,
                    Some("test".to_owned()),
                    None,
                    None,
                ),
                PathBuf::from("/b.slice"),
            ),
        ],
        cpuset: None,
    };

    assert_eq!(token.affected_tasks(), 2);
    assert!(token.restore_path().is_none());
}

#[test]
fn rejects_parent_traversal_in_target_cgroup() {
    let cgroup_root = temp_dir("cgroup-traversal");
    let mut action = action_for(&cgroup_root, 42);
    action.target_cgroup = PathBuf::from("../bad");

    let err = validate_action_request(&action, &CgroupPlacementPolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("must not contain parent traversal"));
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn rejects_invalid_cpuset_values() {
    let cgroup_root = temp_dir("cgroup-invalid-cpuset");
    let mut action = action_for(&cgroup_root, 42);
    action.cpuset_cpus = Some("0;rm -rf".to_owned());

    let err = validate_action_request(&action, &CgroupPlacementPolicy::default())
        .unwrap_err()
        .to_string();

    assert!(err.contains("cpuset.cpus contains invalid character"));
    fs::remove_dir_all(cgroup_root).ok();
}
