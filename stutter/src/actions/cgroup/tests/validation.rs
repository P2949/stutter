use super::*;

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

    assert!(err.contains("action_missing_path"));
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

    assert!(err.contains("action_invalid_policy"));
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

    assert!(err.contains("action_policy_denied"));
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

    assert!(err.contains("action_missing_path"));
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

        assert!(err.contains("action_invalid_request"));
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

    assert!(format!("{:#}", err).contains("action_target_identity_mismatch"));
    fs::remove_dir_all(proc_root).ok();
    fs::remove_dir_all(cgroup_root).ok();
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
