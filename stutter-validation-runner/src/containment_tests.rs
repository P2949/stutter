use super::*;

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "live disposable cgroup inode-swap denial; requires caller-owned delegation"]
fn live_path_replacement_does_not_remove_another_cgroup() {
    let group = OwnedCgroup::prepare().expect("BlockedResource: caller-owned cgroup required");
    let original = group.path().to_owned();
    let birth = group.birth.record.clone();
    // v2 forbids rename. Remove/recreate this empty path while its old kernel fd stays pinned.
    fs::remove_dir(&original).unwrap();
    fs::create_dir(&original).unwrap();
    assert!(group.populated().is_err());
    drop(group);
    assert!(
        original.exists(),
        "another cgroup must survive guard cleanup"
    );
    fs::remove_dir(original).unwrap();
    journal::Registry::open().unwrap().remove(&birth).unwrap();
}

#[test]
fn regular_files_cannot_impersonate_kernel_cgroup_handles() {
    let scratch = crate::tests::Scratch::new("containment-negative");
    let path = scratch.path().join("cgroup.kill");
    fs::write(&path, "unrelated").unwrap();
    assert_eq!(
        cgroup_file(&path, true, false).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(fs::read_to_string(path).unwrap(), "unrelated");
}

#[test]
fn reaper_path_requires_root_ownership_and_protected_directory_entries() {
    assert!(protected_component(0, 0o555, false));
    assert!(protected_component(0, 0o1775, true));
    assert!(!protected_component(1000, 0o1775, true));
    assert!(!protected_component(0, 0o775, true));
    assert!(!protected_component(0, 0o1775, false));
}

#[test]
fn stale_owner_names_cannot_match_other_cgroups_or_incomplete_proofs() {
    assert_eq!(
        cleanup::parse_owner("stutter-owned-validation-42-987-1"),
        Some((42, 987))
    );
    for name in [
        "other-42-987-1",
        "stutter-owned-validation-1-987-1",
        "stutter-owned-validation-42-0-1",
        "stutter-owned-validation-42-987",
        "stutter-owned-validation-42-987-0",
        "stutter-owned-validation-42-987-1-extra",
        "stutter-owned-validation-42-987-1-0",
        "stutter-owned-validation-42-987-1-1234-extra",
    ] {
        assert_eq!(cleanup::parse_owner(name), None, "{name}");
    }
}

#[test]
#[ignore = "live disposable kernel cgroup; requires caller-owned delegation"]
fn live_empty_group_and_reaper_cleanup() {
    let mut group = OwnedCgroup::prepare().expect("BlockedResource: caller-owned cgroup required");
    let path = group.path().to_owned();
    assert!(!group.populated().unwrap());
    group.finish().unwrap();
    group.finish().unwrap();
    assert!(!path.exists());
}

#[test]
#[ignore = "live owned-child setsid/environment-clear cleanup; requires cgroup delegation"]
fn live_markerless_session_child_is_killed_without_touching_sentinel() {
    let scratch = crate::tests::Scratch::new("containment-session");
    let ready = scratch.path().join("ready");
    let mut sentinel = OwnedChild(Command::new("sleep").arg("60").spawn().unwrap());
    let mut group = OwnedCgroup::prepare().expect("BlockedResource: caller-owned cgroup required");
    let mut command = detached_command(&ready);
    group.attach(&mut command).unwrap();
    let mut child = OwnedChild(command.spawn().unwrap());
    let pid = wait_for_pid(&ready);
    assert_eq!(pid, child.0.id());
    assert!(
        !fs::read(format!("/proc/{pid}/environ"))
            .unwrap()
            .windows(b"STUTTER_XTASK_OWNER".len())
            .any(|v| v == b"STUTTER_XTASK_OWNER")
    );
    let membership = fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap();
    assert_eq!(
        membership.trim(),
        format!(
            "0::/{}",
            group
                .path()
                .strip_prefix("/sys/fs/cgroup")
                .unwrap()
                .display()
        )
    );
    assert!(group.populated().unwrap());
    group.finish().unwrap();
    assert!(!child.0.wait().unwrap().success());
    assert!(sentinel.0.try_wait().unwrap().is_none());
}

fn detached_command(ready: &Path) -> Command {
    let mut command = Command::new("setsid");
    command
        .args([
            "env",
            "-i",
            "PATH=/run/current-system/sw/bin:/usr/bin:/bin",
            "/bin/sh",
            "-c",
            "printf '%s' $$ > \"$1\"; trap '' TERM; while :; do sleep 1; done",
            "owned-stage",
        ])
        .arg(ready)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn wait_for_pid(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(pid) = fs::read_to_string(path)
            .ok()
            .and_then(|value| value.parse().ok())
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "owned child readiness deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn pidfd(pid: u32, expected_ticks: u64) -> OwnedFd {
    // SAFETY: pidfd_open takes a numeric pid and zero flags and returns a fresh descriptor.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    assert!(fd >= 0);
    // SAFETY: successful pidfd_open returned a fresh fd owned by this scope.
    let handle = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    assert_eq!(start_ticks(pid), Some(expected_ticks));
    handle
}

fn exited(handle: &OwnedFd) -> bool {
    let mut poll = libc::pollfd {
        fd: handle.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll points to one initialized descriptor record and zero timeout never blocks.
    (unsafe { libc::poll(&mut poll, 1, 0) }) > 0 && poll.revents & libc::POLLIN != 0
}

#[test]
#[ignore = "live owned-parent SIGKILL and markerless descendants; requires cgroup delegation"]
fn live_parent_sigkill_kills_markerless_descendants() {
    const CHILD_MODE: &str = "STUTTER_CONTAINMENT_CRASH_CHILD";
    if let Some(ready) = std::env::var_os(CHILD_MODE) {
        let ready = PathBuf::from(ready);
        let target_ready = ready.with_extension("target");
        let group = OwnedCgroup::prepare().expect("BlockedResource: caller-owned cgroup required");
        let mut command = detached_command(&target_ready);
        group.attach(&mut command).unwrap();
        let child = OwnedChild(command.spawn().unwrap());
        assert_eq!(wait_for_pid(&target_ready), child.0.id());
        let record = serde_json::json!({"pid":child.0.id(),"ticks":start_ticks(child.0.id()).unwrap(),
            "reaper":group.reaper.id(),"reaper_ticks":start_ticks(group.reaper.id()).unwrap(),"path":group.path()});
        let partial = ready.with_extension("partial");
        fs::write(&partial, serde_json::to_vec(&record).unwrap()).unwrap();
        fs::rename(partial, ready).unwrap();
        // Keep both RAII owners live until SIGKILL, which cannot run Rust cleanup.
        loop {
            std::hint::black_box((&group, &child));
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let scratch = crate::tests::Scratch::new("containment-owner-crash");
    let ready = scratch.path().join("ready.json");
    let mut sentinel = OwnedChild(Command::new("sleep").arg("60").spawn().unwrap());
    let mut owner = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "containment::tests::live_parent_sigkill_kills_markerless_descendants",
            ])
            .env(CHILD_MODE, &ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(
            owner.0.try_wait().unwrap().is_none(),
            "child owner exited before readiness"
        );
        assert!(Instant::now() < deadline, "child owner readiness deadline");
        std::thread::sleep(Duration::from_millis(10));
    }
    let record: serde_json::Value = serde_json::from_slice(&fs::read(ready).unwrap()).unwrap();
    println!("crash_fixture {record}");
    let target = pidfd(
        record["pid"].as_u64().unwrap() as u32,
        record["ticks"].as_u64().unwrap(),
    );
    let reaper = pidfd(
        record["reaper"].as_u64().unwrap() as u32,
        record["reaper_ticks"].as_u64().unwrap(),
    );
    assert!(!exited(&target) && !exited(&reaper));
    owner.0.kill().unwrap();
    owner.0.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !exited(&target) || !exited(&reaper) {
        assert!(
            Instant::now() < deadline,
            "independent reaper did not settle owner crash"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(sentinel.0.try_wait().unwrap().is_none());
    let path = PathBuf::from(record["path"].as_str().unwrap());
    let root = delegated_root().unwrap();
    assert_eq!(path.parent(), Some(root.as_path()));
    cleanup::reconcile_owned(&journal::Registry::open().unwrap()).unwrap();
    assert!(
        !path.exists(),
        "crash-resume reconciliation must remove the empty owned group"
    );
}
