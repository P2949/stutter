use super::*;

fn setup(name: &str) -> (PathBuf, PathBuf, CgroupPlacementAction) {
    let root = temp_dir(name);
    let proc_root = root.join("proc");
    let cgroup_root = root.join("cgroup");
    write_fake_cgroup(&cgroup_root, "/old.slice");
    write_fake_cgroup(&cgroup_root, "/stutter/game.slice");
    write_fake_task(&proc_root, 4242, "game-thread", 12345, "/old.slice");
    let mut action = action_for(&cgroup_root, 4242);
    action.cpuset_cpus = None;
    action.cpuset_mems = None;
    (proc_root, cgroup_root, action)
}

#[test]
fn the_hook_sees_the_original_cgroup_and_its_identity_before_any_file_is_written() {
    let (proc_root, cgroup_root, action) = setup("prepared-hook");
    let procs = cgroup_root.join("stutter/game.slice/cgroup.procs");
    let mut seen = None;
    let policy = CgroupPlacementPolicy::default();
    let err = action
        .apply_prepared_at(&proc_root, &policy, &mut |token| {
            // Nothing has been written when the journal sees the token.
            assert_eq!(fs::read_to_string(&procs).unwrap(), "");
            seen = Some(token.clone());
            anyhow::bail!("journal refused")
        })
        .unwrap_err();
    assert!(format!("{:#}", err.source).contains("journal refused"));
    assert!(err.rollback.is_none());
    assert_eq!(fs::read_to_string(&procs).unwrap(), "");
    let token = seen.expect("hook invoked");
    let (records, _) = token.as_cgroup_restore().expect("cgroup token");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].original_cgroup, PathBuf::from("/old.slice"));
    assert_eq!(
        records[0].original_cgroup_id,
        Some(cgroup_id(&cgroup_root.join("old.slice")).unwrap())
    );
    assert_eq!(records[0].restore_identity().starttime_ticks, Some(12345));
}

#[test]
fn a_task_that_drifted_after_preparation_aborts_the_apply_without_writing() {
    let (proc_root, cgroup_root, action) = setup("prepared-drift");
    let procs = cgroup_root.join("stutter/game.slice/cgroup.procs");
    let proc_for_hook = proc_root.clone();
    let err = action
        .apply_prepared_at(&proc_root, &CgroupPlacementPolicy::default(), &mut |_| {
            // Another actor moves the task before the commit.
            write_fake_task(
                &proc_for_hook,
                4242,
                "game-thread",
                12345,
                "/stutter/game.slice",
            );
            Ok(())
        })
        .unwrap_err();
    assert!(
        format!("{:#}", err.source).contains("drifted"),
        "{:#}",
        err.source
    );
    assert_eq!(fs::read_to_string(&procs).unwrap(), "");
}

#[test]
fn a_recreated_original_cgroup_is_never_restored_into() {
    let (proc_root, cgroup_root, action) = setup("prepared-recreated");
    let record = CgroupRestoreRecord::new(
        TaskRestoreIdentity::observed(4242, Some(4242), None, Some(12345), None),
        PathBuf::from("/old.slice"),
    )
    .with_cgroup_id(cgroup_id(&cgroup_root.join("old.slice")).unwrap());
    let token = RollbackToken::CgroupRestore {
        records: vec![record],
        cpuset: None,
    };
    // The original cgroup is destroyed and recreated at the same path: a different resource.
    fs::remove_dir_all(cgroup_root.join("old.slice")).unwrap();
    // Keep a few other directories alive so a reused inode number cannot make the recreated
    // directory look like the original.
    for index in 0..4 {
        write_fake_cgroup(&cgroup_root, &format!("/padding-{index}.slice"));
    }
    write_fake_cgroup(&cgroup_root, "/old.slice");
    let procs = cgroup_root.join("old.slice/cgroup.procs");
    action.rollback_at(&proc_root, &token).unwrap();
    assert_eq!(
        fs::read_to_string(&procs).unwrap(),
        "",
        "nothing was restored into it"
    );
}

#[test]
fn the_startup_handler_refuses_records_without_a_cgroup_identity() {
    let (proc_root, cgroup_root, _) = setup("prepared-strict");
    let record = CgroupRestoreRecord::new(
        TaskRestoreIdentity::observed(4242, Some(4242), None, Some(12345), None),
        PathBuf::from("/old.slice"),
    );
    let token = RollbackToken::CgroupRestore {
        records: vec![record],
        cpuset: None,
    };
    let result = CgroupRollbackHandler {
        require_verified_identity: true,
    }
    .restore_token_at(&proc_root, &cgroup_root, &token)
    .unwrap();
    assert_eq!(result.restored, 0);
    assert_eq!(result.skipped_identity_mismatch, 1);
    assert_eq!(
        fs::read_to_string(cgroup_root.join("old.slice/cgroup.procs")).unwrap(),
        ""
    );
}
