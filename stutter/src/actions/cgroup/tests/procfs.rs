use super::*;

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
