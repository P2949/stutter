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

/// A probe whose observer attributes (or no longer attributes) the game on command.
struct SwitchProbe(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl ResultProbe for SwitchProbe {
    fn begin(&mut self, _: crate::autotune::research::ProtonSource) -> anyhow::Result<()> {
        Ok(())
    }
    fn poll(&mut self) -> GameState {
        if self.0.load(std::sync::atomic::Ordering::SeqCst) {
            GameState::Running
        } else {
            GameState::Starting
        }
    }
}

struct Probe;
impl ResultProbe for Probe {
    fn begin(&mut self, _: crate::autotune::research::ProtonSource) -> anyhow::Result<()> {
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
        [
            "steam",
            "-applaunch",
            "1091500",
            "--launcher-skip",
            "-benchmark"
        ]
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

    let game_attributable = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
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
    super::super::dispatch_context::mark_launch_requested(&attempt).unwrap();
    let env = [
        ("SteamAppId", "1091500"),
        ("STEAM_COMPAT_APP_ID", "1091500"),
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
        let (_, ticket) = authorize_dispatch(
            &attempt,
            &argv,
            &env,
            intent.uid,
            now + 1,
            &|_, _| Ok(()),
            &|_| Ok(u64::MAX),
        )
        .unwrap();
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
        SwitchProbe(game_attributable.clone()),
    );
    port.proc_root = proc.path().to_path_buf();
    port.observe_desktop_idle(Some(Instant::now()));
    port.observe_user_input_idle(Some(Instant::now()));
    port.dispatch = Some((attempt, context));
    assert_eq!(port.state(), GameState::Running);
    // The live dispatcher exits while the game keeps running: the game stays visible.
    std::fs::remove_dir_all(proc.path().join("123")).unwrap();
    assert_eq!(port.state(), GameState::Running);
    // Once the independent observer no longer attributes a game, nothing is Running.
    game_attributable.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(port.state(), GameState::Starting);
}

mod transient {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use super::*;
    use crate::autotune::research::{
        launch_txn::{SendError, UnitState},
        user_launcher::UserLaunchRequest,
    };

    struct Manager {
        send: Option<Result<(), SendError>>,
        states: Vec<UnitState>,
        sends: Arc<AtomicU32>,
    }

    impl UserManagerPort for Manager {
        fn send_start(&mut self, _: &UserLaunchRequest) -> Result<(), SendError> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            self.send.take().unwrap_or(Err(SendError::AckLost))
        }
        fn unit_state(&mut self, _: &str) -> anyhow::Result<UnitState> {
            Ok(if self.states.is_empty() {
                UnitState::Absent
            } else {
                self.states.remove(0)
            })
        }
        fn proves_never_executable(&mut self, _: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    fn fixture(
        send: Result<(), SendError>,
        states: Vec<UnitState>,
        prepare_ok: bool,
    ) -> (
        ProductionSteamPort<Probe>,
        File,
        AttemptSlot,
        DispatchContextV1,
        Arc<AtomicU32>,
        tempfile::TempDir,
    ) {
        let sends = Arc::new(AtomicU32::new(0));
        let launcher = TransientLauncher {
            manager: Box::new(Manager {
                send: Some(send),
                states,
                sends: sends.clone(),
            }),
            prepare: Box::new(move |_, _| {
                anyhow::ensure!(prepare_ok, "cannot prepare the request");
                Ok(UserLaunchRequest {
                    unit_name: "stutter-research-0123456789abcdef-0000.service".into(),
                    body: Vec::new(),
                })
            }),
        };
        let port = ProductionSteamPort::new(
            "/s".into(),
            "/study".into(),
            "/a".into(),
            "/m".into(),
            super::super::super::tests::intent().steam_client,
            vec![],
            Probe,
        )
        .with_transient_launcher(launcher);
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(
            tmp.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let dir = File::from(crate::syscall::open_dir_no_symlinks(tmp.path()).unwrap());
        let slot = AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index: 0,
            arm: super::super::super::ProtonSource::Ge,
        };
        let context = DispatchContextV1 {
            schema_version: 1,
            intent: super::super::super::tests::intent(),
            slot: slot.clone(),
            tickets: Default::default(),
            spec: super::super::super::dispatch_context::ArmSpec {
                tool_dir: "/t/ge".into(),
                study_compat_data: "/s".into(),
                arm_env: Default::default(),
                steamapps: "/a".into(),
                proton_manifest: "/m".into(),
            },
        };
        (port, dir, slot, context, sends, tmp)
    }

    #[test]
    fn a_running_unit_leaves_the_port_unsettled_and_never_completes_by_itself() {
        let (mut port, dir, slot, context, sends, _tmp) =
            fixture(Ok(()), vec![UnitState::Active], true);
        port.start_unit(dir, &slot, &context).unwrap();
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        assert!(port.unsettled && port.dispatch.is_some());
        // Detached game-tree settlement is unverified, so stop refuses.
        assert!(port.stop().is_err());
    }

    #[test]
    fn a_verified_refusal_clears_the_port_but_ambiguity_never_does() {
        let (mut port, dir, slot, context, _, _tmp) =
            fixture(Err(SendError::Rejected), vec![], true);
        let error = port.start_unit(dir, &slot, &context).unwrap_err();
        assert!(error.downcast_ref::<LaunchUnresolved>().is_none());
        assert!(!port.unsettled && port.dispatch.is_none());

        let (mut port, dir, slot, context, sends, _tmp) =
            fixture(Err(SendError::AckLost), vec![], true);
        let error = port.start_unit(dir, &slot, &context).unwrap_err();
        assert!(error.downcast_ref::<LaunchUnresolved>().is_some());
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        assert!(port.unsettled && port.dispatch.is_some());
    }

    #[test]
    fn a_preparation_failure_sends_nothing_and_leaves_no_trace() {
        let (mut port, dir, slot, context, sends, _tmp) = fixture(Ok(()), vec![], false);
        assert!(port.start_unit(dir, &slot, &context).is_err());
        assert_eq!(sends.load(Ordering::SeqCst), 0);
        assert!(!port.unsettled && port.dispatch.is_none());
    }
}

#[test]
fn a_port_without_a_transient_launcher_blocks_instead_of_spawning_steam() {
    let mut port = ProductionSteamPort::new(
        "/s".into(),
        "/study".into(),
        "/a".into(),
        "/m".into(),
        super::super::tests::intent().steam_client,
        vec![],
        Probe,
    );
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
    let error = port.launch(&slot, &context).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<super::super::driver_failure::LaunchFailure>()
            .unwrap()
            .class,
        super::super::driver_failure::FailureClass::Environment
    );
    assert!(
        error
            .to_string()
            .contains("BlockedTransientLauncherRequired")
    );
    assert!(!port.unsettled && port.dispatch.is_none());
}

mod settlement {
    use super::*;

    struct Witness(Option<bool>);
    impl SettlementWitness for Witness {
        fn settled(&mut self, _: &File, _: &DispatchContextV1) -> anyhow::Result<bool> {
            self.0.ok_or_else(|| anyhow::anyhow!("witness unavailable"))
        }
    }

    fn launched_port(witness: Option<Option<bool>>) -> ProductionSteamPort<Probe> {
        let mut port = ProductionSteamPort::new(
            "/s".into(),
            "/study".into(),
            "/a".into(),
            "/m".into(),
            crate::autotune::research::tests::intent().steam_client,
            vec![],
            Probe,
        );
        let tmp = tempfile::tempdir().unwrap();
        let dir = File::from(crate::syscall::open_dir_no_symlinks(tmp.path()).unwrap());
        let intent = crate::autotune::research::tests::intent();
        let slot = AttemptSlot {
            study_sha256: "e".repeat(64),
            context_sha256: "f".repeat(64),
            index: 0,
            arm: crate::autotune::research::ProtonSource::Ge,
        };
        let context = DispatchContextV1 {
            schema_version: 1,
            intent,
            slot,
            tickets: Default::default(),
            spec: crate::autotune::research::dispatch_context::ArmSpec {
                tool_dir: "/t".into(),
                study_compat_data: "/s".into(),
                arm_env: Default::default(),
                steamapps: "/a".into(),
                proton_manifest: "/m".into(),
            },
        };
        port.unsettled = true;
        port.dispatch = Some((dir, context));
        if let Some(answer) = witness {
            port = port.with_settlement_witness(Box::new(Witness(answer)));
        }
        port
    }

    /// Settles on the third look: a launch still in flight when the attempt was aborted.
    struct LateWitness(std::sync::atomic::AtomicU32);
    impl SettlementWitness for LateWitness {
        fn settled(&mut self, _: &File, _: &DispatchContextV1) -> anyhow::Result<bool> {
            let looks = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            Ok(looks >= 3)
        }
    }
    #[test]
    fn a_settle_window_waits_for_a_launch_that_was_still_in_flight() {
        let mut port = launched_port(None)
            .with_settlement_witness(Box::new(LateWitness(Default::default())))
            .with_settle_window(Duration::from_secs(10));
        let started = std::time::Instant::now();
        assert!(port.stop().is_ok());
        assert!(started.elapsed() < Duration::from_secs(8));
        // With no window the first refusal is final.
        let mut strict =
            launched_port(None).with_settlement_witness(Box::new(LateWitness(Default::default())));
        assert!(strict.stop().is_err());
    }

    #[test]
    fn the_stop_step_returns_at_once_and_settles_over_polls_inside_its_window() {
        use crate::autotune::research::steam_host::StopStep;
        let mut port = launched_port(None)
            .with_settlement_witness(Box::new(LateWitness(Default::default())))
            .with_settle_window(Duration::from_secs(10));
        let started = std::time::Instant::now();
        assert_eq!(port.stop_step(), StopStep::Pending);
        assert_eq!(port.stop_step(), StopStep::Pending);
        assert_eq!(port.stop_step(), StopStep::Settled);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "no waiting inside a step"
        );
        // With no window the first refusal is final.
        let mut strict =
            launched_port(None).with_settlement_witness(Box::new(LateWitness(Default::default())));
        assert_eq!(strict.stop_step(), StopStep::Unsettled);
    }

    #[test]
    fn only_an_independent_witness_clears_the_unsettled_state() {
        let mut port = launched_port(Some(Some(true)));
        assert!(port.stop().is_ok());
        assert!(!port.unsettled);
        // Idempotent once settled.
        assert!(port.stop().is_ok());
    }

    #[test]
    fn a_missing_negative_or_failing_witness_never_settles() {
        for witness in [None, Some(Some(false)), Some(None)] {
            let mut port = launched_port(witness);
            assert!(port.stop().is_err());
            assert!(port.unsettled);
        }
        // Without a dispatch record there is nothing to witness.
        let mut port = launched_port(Some(Some(true)));
        port.dispatch = None;
        assert!(port.stop().is_err());
        assert!(port.unsettled);
    }

    struct Terminator(std::sync::Arc<std::sync::atomic::AtomicU32>, bool);
    impl OwnedTerminator for Terminator {
        fn terminate_owned(&mut self, _: &File, _: &DispatchContextV1) -> anyhow::Result<()> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            anyhow::ensure!(self.1, "ownership could not be established");
            Ok(())
        }
    }

    #[test]
    fn stop_runs_exact_owner_cleanup_but_only_a_witness_settles() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        // Cleanup succeeding does not by itself settle the port.
        let mut port =
            launched_port(None).with_owned_terminator(Box::new(Terminator(calls.clone(), true)));
        assert!(port.stop().is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(port.unsettled);
        // A witness still has to agree afterwards.
        let mut port = launched_port(Some(Some(true)))
            .with_owned_terminator(Box::new(Terminator(calls.clone(), true)));
        assert!(port.stop().is_ok());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        // A refused cleanup is logged and leaves the port unsettled when no witness settles it.
        let mut port = launched_port(Some(Some(false)))
            .with_owned_terminator(Box::new(Terminator(calls.clone(), false)));
        assert!(port.stop().is_err());
        assert!(port.unsettled);
        // Without a terminator nothing is ever signalled.
        let mut port = launched_port(None);
        assert!(port.stop().is_err());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }
}
#[path = "steam_port_stop_tests.rs"]
mod stop;
