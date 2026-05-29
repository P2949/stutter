use super::*;

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
fn apply_returns_unapplied_cpuset_rollback_when_cpuset_mems_write_fails() {
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
    let token = err
        .rollback
        .expect("cpuset mutation should return rollback token");
    assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "2-3");
    assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
    assert_eq!(read_trimmed(&target.join("cgroup.procs")).unwrap(), "");

    action.rollback_at(&proc_root, &token).unwrap();

    assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
    assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn apply_returns_unapplied_task_and_cpuset_rollback_when_second_task_move_fails() {
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
    let token = err
        .rollback
        .expect("partial task move should return rollback token");
    assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "2-3");
    assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
    assert_eq!(
        read_trimmed(&cgroup_root.join("old-first.slice/cgroup.procs")).unwrap(),
        ""
    );
    assert_eq!(
        read_trimmed(&cgroup_root.join("old-second.slice/cgroup.procs")).unwrap(),
        ""
    );

    action.rollback_at(&proc_root, &token).unwrap();

    assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
    assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
    assert_eq!(
        read_trimmed(&cgroup_root.join("old-first.slice/cgroup.procs")).unwrap(),
        "41"
    );
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}

#[test]
fn audited_runner_rolls_back_cgroup_partial_failure_exactly_once() {
    let proc_root = temp_dir("proc-runner-partial");
    let cgroup_root = temp_dir("cgroup-runner-partial");
    let audit_path = temp_dir("audit-runner-partial").join("audit.jsonl");
    write_fake_cgroup(&cgroup_root, "/old-first.slice");
    write_fake_cgroup(&cgroup_root, "/old-second.slice");
    let target = write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    write_fake_task(&proc_root, 41, "first", 12345, "/old-first.slice");
    write_fake_task(&proc_root, 42, "second", 12345, "/old-second.slice");

    let mut inner = action_for(&cgroup_root, 41);
    inner.targets = vec![
        placement_target(41, "first", TaskClass::Game),
        placement_target(42, "second", TaskClass::Game),
    ];
    let rollback_calls = Cell::new(0);
    let action = ProcRootCgroupAction {
        inner: &inner,
        proc_root: &proc_root,
        rollback_calls: &rollback_calls,
        fail_on_apply_write: 4,
    };

    let mut run_policy = ActionRunPolicy::for_action(&action, false, ActionSource::Test);
    run_policy.policy = DaemonPolicy::apply_medium_risk(ActionSource::Test);

    let result =
        run_audited_action_with_audit_path("cgroup-runner-test", &action, run_policy, &audit_path);

    assert!(result.is_err());
    let err = format!("{:#}", result.as_ref().unwrap_err());
    assert!(
        err.contains("partial rollback attempted"),
        "expected audited runner to reach the partial-apply rollback path, got: {err}"
    );
    assert_eq!(rollback_calls.get(), 1);
    assert_eq!(read_trimmed(&target.join("cpuset.cpus")).unwrap(), "0-7");
    assert_eq!(read_trimmed(&target.join("cpuset.mems")).unwrap(), "0");
    assert_eq!(
        read_trimmed(&cgroup_root.join("old-first.slice/cgroup.procs")).unwrap(),
        "41"
    );

    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
    if let Some(audit_dir) = audit_path.parent() {
        fs::remove_dir_all(audit_dir).ok();
    }
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

    assert!(err.contains("invalid rollback token"));
    assert!(err.contains("expected cgroup-restore"));
    assert!(err.contains("actual nice-restore"));
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
fn rollback_token_handler_restores_namespace_absolute_cgroup_path_under_root() {
    let proc_root = temp_dir("proc-token-rollback-namespace-absolute");
    let cgroup_root = temp_dir("cgroup-token-rollback-namespace-absolute");
    write_fake_task(&proc_root, 42, "game-thread", 12345, "/stutter/game.slice");
    write_fake_cgroup(&cgroup_root, "/old.slice");

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

    let result = CgroupRollbackHandler
        .restore_token_at(&proc_root, &cgroup_root, &token)
        .unwrap();

    assert_eq!(result.restored, 1);
    assert_eq!(
        read_trimmed(&cgroup_root.join("old.slice/cgroup.procs")).unwrap(),
        "42"
    );
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
}
