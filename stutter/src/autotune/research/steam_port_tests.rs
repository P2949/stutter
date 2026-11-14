use super::{super::session_env::session_environment_matches, *};

fn unprivileged_launcher_status(status: &str, uid: u32) -> bool {
    super::super::session_env::capability_status(status, uid, true)
}

#[test]
fn launcher_rejects_expired_and_not_yet_valid_authorization() {
    let intent = super::super::tests::intent();
    let at = |seconds| std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    assert!(!launch_window_open(
        &intent,
        at(intent.issued_unix_seconds - 1)
    ));
    assert!(launch_window_open(&intent, at(intent.issued_unix_seconds)));
    assert!(launch_window_open(
        &intent,
        at(intent.deadline_unix_seconds - 1)
    ));
    assert!(!launch_window_open(
        &intent,
        at(intent.deadline_unix_seconds)
    ));
    assert!(!launch_window_open(
        &intent,
        std::time::UNIX_EPOCH - std::time::Duration::from_secs(1)
    ));
}

fn proc_with(comms: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (pid, comm) in comms.iter().enumerate() {
        let p = dir.path().join((pid + 100).to_string());
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("comm"), format!("{comm}\n")).unwrap();
    }
    std::fs::create_dir(dir.path().join("self")).unwrap();
    dir
}

#[test]
fn only_the_frozen_client_tree_is_exempt_from_the_prefilter() {
    fn add(proc: &Path, pid: u32, parent: u32, ticks: u64, comm: &str) {
        let path = proc.join(pid.to_string());
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("comm"), format!("{comm}\n")).unwrap();
        std::fs::write(
            path.join("stat"),
            format!("{pid} ({comm}) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0"),
        )
        .unwrap();
    }
    let proc = tempfile::tempdir().unwrap();
    add(proc.path(), 100, 1, 11, "steam");
    add(proc.path(), 101, 100, 12, "steamwebhelper");
    add(proc.path(), 102, 101, 13, "steamwebhelper");
    let client = SteamProcessIdentity {
        steam_pid: 100,
        steam_start_ticks: 11,
        webhelper_pid: 101,
        webhelper_start_ticks: 12,
        logged_in: true,
    };
    let uid = crate::syscall::geteuid();
    assert!(!host_has_game_or_foreign_steam(proc.path(), &client, uid));
    assert!(host_has_game_or_foreign_steam_bounded(
        proc.path(),
        &client,
        uid,
        2
    ));
    assert!(host_has_game_or_foreign_steam(
        proc.path(),
        &client,
        uid + 1
    ));
    add(proc.path(), 103, 1, 14, "steam");
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
    std::fs::remove_dir_all(proc.path().join("103")).unwrap();
    add(proc.path(), 103, 100, 14, "steam");
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
    std::fs::remove_dir_all(proc.path().join("103")).unwrap();
    add(proc.path(), 103, 100, 14, "wineserver");
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
    std::fs::remove_dir_all(proc.path().join("103")).unwrap();
    let mut reused = client.clone();
    reused.steam_start_ticks += 1;
    assert!(host_has_game_or_foreign_steam(proc.path(), &reused, uid));
    std::fs::remove_file(proc.path().join("102/comm")).unwrap();
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
    std::fs::write(proc.path().join("102/comm"), "steamwebhelper\n").unwrap();
    std::fs::remove_file(proc.path().join("102/stat")).unwrap();
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
    assert!(host_has_game_or_foreign_steam(
        Path::new("/definitely/not/proc"),
        &client,
        uid
    ));
}

struct Probe;
impl ResultProbe for Probe {
    fn begin(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn poll(&mut self) -> GameState {
        GameState::Running
    }
}

#[test]
fn session_environment_requires_exact_unique_home_and_runtime() {
    use std::ffi::OsStr;
    let home = Some(OsStr::new("/home/user"));
    let runtime = Some(OsStr::new("/run/user/1000"));
    let good = b"HOME=/home/user\0XDG_RUNTIME_DIR=/run/user/1000\0OTHER=redacted\0";
    assert!(session_environment_matches(good, home, runtime));
    assert!(!session_environment_matches(
        good,
        Some(OsStr::new("/var/lib/stutter")),
        runtime
    ));
    assert!(!session_environment_matches(good, home, None));
    assert!(!session_environment_matches(
        b"HOME=/home/user\0HOME=/home/user\0XDG_RUNTIME_DIR=/run/user/1000\0",
        home,
        runtime
    ));
    assert!(!session_environment_matches(
        b"HOME=/home/user\0XDG_RUNTIME_DIR=/run/user/1000",
        home,
        runtime
    ));
}

#[test]
fn launcher_requires_zero_capabilities_fixed_uid_and_no_new_privileges() {
    let safe = "Uid:\t1000 1000 1000 1000\nCapInh:\t0000000000000000\nCapPrm:\t0000000000000000\nCapEff:\t0000000000000000\nCapAmb:\t0000000000000000\nNoNewPrivs:\t1\n";
    assert!(unprivileged_launcher_status(safe, 1000));
    assert!(!unprivileged_launcher_status(safe, 0));
    assert!(!unprivileged_launcher_status(
        &safe.replace("CapAmb:\t0000000000000000", "CapAmb:\t0000000000000001"),
        1000
    ));
    assert!(!unprivileged_launcher_status(
        &safe.replace("NoNewPrivs:\t1", "NoNewPrivs:\t0"),
        1000
    ));
    assert!(!unprivileged_launcher_status(
        &safe.replace("Uid:\t1000 1000 1000 1000", "Uid:\t0 1000 0 1000"),
        1000
    ));
    assert!(!unprivileged_launcher_status(
        &safe.replace("CapPrm:\t0000000000000000\n", ""),
        1000
    ));
    assert!(!unprivileged_launcher_status(
        &format!("{safe}CapEff:\t0000000000000000\n"),
        1000
    ));
}

#[test]
fn launch_refuses_unless_idle_and_free_and_the_command_is_the_one_owned_app() {
    let proc = proc_with(&["zsh"]);
    let mut port = ProductionSteamPort::new(
        "/s".into(),
        "/study".into(),
        "/a".into(),
        "/m".into(),
        super::super::tests::intent().steam_client,
        vec![(super::super::ProtonSource::Ge, "/t/ge".into())],
        Probe,
    );
    port.proc_root = proc.path().to_path_buf();
    let slot = AttemptSlot {
        study_sha256: "e".repeat(64),
        context_sha256: "f".repeat(64),
        index: 0,
        arm: super::super::ProtonSource::Ge,
    };
    let context = DispatchContextV1 {
        schema_version: 1,
        intent: super::super::tests::intent(),
        slot: slot.clone(),
        tickets: Default::default(),
        spec: super::super::dispatch_context::ArmSpec {
            tool_dir: "/t/ge".into(),
            study_compat_data: "/s".into(),
            arm_env: Default::default(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
        },
    };
    assert!(!port.safe_window());
    assert!(port.launch(&slot, &context).is_err());
    port.observe_desktop_idle(Some(Instant::now()));
    // Idle alone cannot qualify the client: the fixture has no trusted Steam inventory.
    assert!(!port.safe_window());
    port.observe_desktop_idle(Some(Instant::now() - Duration::from_secs(4)));
    assert!(!port.safe_window());
    assert_eq!(port.state(), GameState::DesktopInterrupted);
    port.observe_desktop_idle(None);
    assert_eq!(port.state(), GameState::DesktopInterrupted);
    port.observe_desktop_idle(Some(Instant::now()));
    assert_eq!(port.tool_dir(&slot).unwrap(), PathBuf::from("/t/ge"));
    let mut other = slot.clone();
    other.arm = super::super::ProtonSource::Em;
    assert!(port.tool_dir(&other).is_err());
    assert_eq!(
        launch_argv(),
        ["steam", "-applaunch", "1091500", "-benchmark"]
    );
    assert!(port.stop().is_ok());
    port.unsettled = true;
    assert!(port.stop().is_err());
    assert!(!port.safe_window());
    assert!(port.launch(&slot, &context).is_err());
}

#[test]
fn exited_first_steam_verb_never_completes_an_attempt_with_a_live_second_verb() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::super::dispatch_context::{
        ArmSpec, DispatcherStartV1, arm_attempt, authorize_dispatch,
    };

    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let attempt = File::from(crate::syscall::open_dir_no_symlinks(dir.path()).unwrap());
    let mut intent = super::super::tests::intent();
    intent.uid = crate::syscall::geteuid();
    let steam = intent.steam_client.process.as_mut().unwrap();
    steam.steam_pid = 200;
    steam.steam_start_ticks = 40;
    steam.webhelper_pid = 201;
    steam.webhelper_start_ticks = 41;
    let slot = AttemptSlot {
        study_sha256: "e".repeat(64),
        context_sha256: "f".repeat(64),
        index: 0,
        arm: intent.tools[0].source,
    };
    let spec = ArmSpec {
        tool_dir: "/tools/GE".into(),
        study_compat_data: "/study/compatdata".into(),
        arm_env: Default::default(),
        steamapps: "/a".into(),
        proton_manifest: "/m".into(),
    };
    let now = intent.issued_unix_seconds + 1;
    let context = arm_attempt(&attempt, &intent, &slot, spec, (now, 60)).unwrap();
    let env = [
        ("SteamAppId", "1091500"),
        ("STEAM_COMPAT_DATA_PATH", "/study/compatdata"),
        ("STEAM_COMPAT_CLIENT_INSTALL_PATH", "/home/u/.steam/steam"),
    ]
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .into();
    let proc = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(proc.path().join("sys/kernel/random")).unwrap();
    std::fs::write(
        proc.path().join("sys/kernel/random/boot_id"),
        &intent.boot_id,
    )
    .unwrap();
    for pid in [123, 200, 201] {
        std::fs::create_dir_all(proc.path().join(pid.to_string()).join("ns")).unwrap();
    }
    std::fs::create_dir_all(proc.path().join("self/ns")).unwrap();
    let ns = proc.path().join("self/ns/pid");
    std::fs::write(&ns, b"namespace").unwrap();
    std::fs::hard_link(&ns, proc.path().join("123/ns/pid")).unwrap();
    let stat = |pid: u32, parent: u32, ticks: u64| {
        format!("{pid} (fixture) S {parent} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 0 0")
    };
    std::fs::write(proc.path().join("200/stat"), stat(200, 1, 40)).unwrap();
    std::fs::write(proc.path().join("201/stat"), stat(201, 200, 41)).unwrap();
    std::fs::write(proc.path().join("123/stat"), stat(123, 200, 80)).unwrap();
    let ns_meta = std::fs::metadata(&ns).unwrap();
    for (verb, pid, ticks) in [
        (SteamVerb::WaitForExitAndRun, 124, 79),
        (SteamVerb::Run, 123, 80),
    ] {
        let argv = ["/x/proton", verb.as_steam_argument(), "/g/game.exe"].map(str::to_owned);
        let (_, ticket) =
            authorize_dispatch(&attempt, &argv, &env, intent.uid, now + 1, &|_, _| Ok(())).unwrap();
        let record = DispatcherStartV1 {
            schema_version: 1,
            ticket_id: ticket.ticket_id,
            intent_sha256: ticket.intent_sha256,
            attempt_sha256: ticket.attempt_sha256,
            verb,
            uid: intent.uid,
            boot_id: intent.boot_id.clone(),
            observed_unix_seconds: now + 1,
            pid: stutter_core::ids::Pid::new(pid),
            start_ticks: ticks,
            pid_namespace_device: ns_meta.dev(),
            pid_namespace_inode: ns_meta.ino(),
        };
        super::super::report_store::write_new(
            &attempt,
            &format!("dispatcher-{}.json", verb.as_steam_argument()),
            &serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }
    let mut port = ProductionSteamPort::new(
        "/study/compatdata".into(),
        "/study".into(),
        "/a".into(),
        "/m".into(),
        intent.steam_client.clone(),
        vec![],
        Probe,
    );
    port.proc_root = proc.path().to_path_buf();
    port.observe_desktop_idle(Some(Instant::now()));
    port.dispatch = Some((attempt, context));
    assert_eq!(port.state(), GameState::Running);
    std::fs::remove_dir_all(proc.path().join("123")).unwrap();
    assert_eq!(port.state(), GameState::Starting);
}
