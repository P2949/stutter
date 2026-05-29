use super::*;

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
