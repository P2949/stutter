//! Prefilter tests against realistic Steam and kernel process layouts.
use super::{super::gpu_health::ResetScan, *};

#[test]
fn steams_outer_launcher_is_exempt_only_as_an_ancestor_of_the_pinned_client() {
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
    // Real layout: outer steam -> srt-bwrap -> pinned inner steam -> web helper.
    add(proc.path(), 200, 1, 20, "steam");
    add(proc.path(), 201, 200, 21, "srt-bwrap");
    add(proc.path(), 202, 201, 22, "steam");
    add(proc.path(), 203, 202, 23, "steamwebhelper");
    let client = SteamProcessIdentity {
        steam_pid: 202,
        steam_start_ticks: 22,
        webhelper_pid: 203,
        webhelper_start_ticks: 23,
        logged_in: true,
    };
    let uid = crate::syscall::geteuid();
    assert!(!host_has_game_or_foreign_steam(proc.path(), &client, uid));
    // An unrelated steam process is still foreign.
    add(proc.path(), 300, 1, 30, "steam");
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
}

#[test]
fn long_kernel_thread_names_are_not_mistaken_for_unknown_or_busy_processes() {
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
    let client = SteamProcessIdentity {
        steam_pid: 100,
        steam_start_ticks: 11,
        webhelper_pid: 101,
        webhelper_start_ticks: 12,
        logged_in: true,
    };
    let uid = crate::syscall::geteuid();
    add(proc.path(), 2, 0, 1, "kworker/0:0H-events_highpri");
    assert!(!host_has_game_or_foreign_steam(proc.path(), &client, uid));
    // An unbounded name is still unknown, and an empty one is never evidence of a free host.
    add(proc.path(), 3, 0, 1, &"k".repeat(200));
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
    std::fs::remove_dir_all(proc.path().join("3")).unwrap();
    std::fs::write(proc.path().join("2/comm"), "").unwrap();
    assert!(host_has_game_or_foreign_steam(proc.path(), &client, uid));
}

#[test]
fn a_foreign_game_outside_the_name_list_still_blocks_admission() {
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
    fn evidence(proc: &Path, pid: u32, cmdline: &[u8], environ: &[u8]) {
        std::fs::write(proc.join(pid.to_string()).join("cmdline"), cmdline).unwrap();
        std::fs::write(proc.join(pid.to_string()).join("environ"), environ).unwrap();
    }
    let proc = tempfile::tempdir().unwrap();
    add(proc.path(), 100, 1, 11, "steam");
    add(proc.path(), 101, 100, 12, "steamwebhelper");
    let client = SteamProcessIdentity {
        steam_pid: 100,
        steam_start_ticks: 11,
        webhelper_pid: 101,
        webhelper_start_ticks: 12,
        logged_in: true,
    };
    let uid = crate::syscall::geteuid();
    let busy = |proc: &Path| host_has_game_or_foreign_steam(proc, &client, uid);
    assert!(!busy(proc.path()));

    // Ordinary desktop processes, and processes with no readable evidence yet, do not block.
    add(proc.path(), 400, 1, 40, "firefox");
    evidence(proc.path(), 400, b"firefox\0--new-tab\0", b"HOME=/h\0");
    add(proc.path(), 401, 1, 41, "bash");
    assert!(!busy(proc.path()));

    // A Windows game started outside Steam under an unlisted name is recognised by its command line.
    add(proc.path(), 410, 1, 42, "launcher");
    evidence(proc.path(), 410, b"wine\0/games/Other Game/Game.EXE\0", b"");
    assert!(busy(proc.path()));
    std::fs::remove_dir_all(proc.path().join("410")).unwrap();
    add(proc.path(), 411, 1, 43, "runner");
    evidence(proc.path(), 411, b"runner\0Z:\\games\\x\0", b"");
    assert!(busy(proc.path()));
    std::fs::remove_dir_all(proc.path().join("411")).unwrap();

    // A native game is recognised by Steam's game identity in its environment, not by its name.
    add(proc.path(), 420, 1, 44, "hl2_linux");
    evidence(
        proc.path(),
        420,
        b"/g/hl2_linux\0",
        b"SteamGameId=220\0HOME=/h\0",
    );
    assert!(busy(proc.path()));
    evidence(
        proc.path(),
        420,
        b"/g/hl2_linux\0",
        b"SteamGameId=0\0SteamAppId=\0",
    );
    assert!(!busy(proc.path()));
    // An unreadable command line is never evidence that the host is free.
    std::fs::remove_file(proc.path().join("420/cmdline")).unwrap();
    std::fs::create_dir(proc.path().join("420/cmdline")).unwrap();
    assert!(busy(proc.path()));
}

struct QuietProbe;

impl ResultProbe for QuietProbe {
    fn begin(&mut self, _: super::super::ProtonSource) -> anyhow::Result<()> {
        Ok(())
    }
    fn poll(&mut self) -> GameState {
        GameState::Running
    }
}

#[test]
fn a_running_games_own_idle_inhibitor_does_not_interrupt_its_attempt_but_user_input_does() {
    let client = crate::autotune::research::tests::intent().steam_client;
    let mut port = ProductionSteamPort::new(
        "/study/compatdata".into(),
        "/study".into(),
        "/a".into(),
        "/m".into(),
        client,
        vec![],
        QuietProbe,
    );
    // No dispatch context yet, so the state is an identity problem, not an interruption, as long as
    // the user has not touched the desktop recently, even though the combined idle signal is
    // absent because the game itself holds an idle inhibitor.
    port.observe_desktop_idle(None);
    port.observe_user_input_idle(Some(Instant::now()));
    assert_eq!(port.state(), GameState::IdentityMismatch);
    // Admission still needs the combined signal.
    assert!(!port.safe_window());
    // Recent user input, or an expired or missing observation, interrupts the attempt.
    port.observe_user_input_idle(Some(Instant::now() - Duration::from_secs(4)));
    assert_eq!(port.state(), GameState::DesktopInterrupted);
    port.observe_user_input_idle(None);
    assert_eq!(port.state(), GameState::DesktopInterrupted);
}

struct FakeGpu(std::sync::Arc<std::sync::Mutex<anyhow::Result<ResetScan>>>);

impl super::super::gpu_health::GpuResetProbe for FakeGpu {
    fn resets_since(&mut self, _: u64) -> anyhow::Result<ResetScan> {
        match &*self.0.lock().unwrap() {
            Ok(scan) => Ok(*scan),
            Err(error) => Err(anyhow::anyhow!("{error}")),
        }
    }
}

type Answer = std::sync::Arc<std::sync::Mutex<anyhow::Result<ResetScan>>>;

fn scan(count: u32) -> anyhow::Result<ResetScan> {
    Ok(ResetScan {
        count,
        latest_unix: None,
    })
}

fn gpu_port(
    answer: anyhow::Result<ResetScan>,
) -> (ProductionSteamPort<QuietProbe>, Answer, tempfile::TempDir) {
    let state_dir = tempfile::tempdir().unwrap();
    let shared = std::sync::Arc::new(std::sync::Mutex::new(answer));
    let client = crate::autotune::research::tests::intent().steam_client;
    let port = ProductionSteamPort::new(
        "/study/compatdata".into(),
        "/study".into(),
        "/a".into(),
        "/m".into(),
        client,
        vec![],
        QuietProbe,
    )
    .with_gpu_reset_probe(
        Box::new(FakeGpu(shared.clone())),
        state_dir.path().to_path_buf(),
    );
    (port, shared, state_dir)
}

fn incident_count(dir: &tempfile::TempDir) -> usize {
    std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(super::super::gpu_health::INCIDENT_PREFIX)
        })
        .count()
}

#[test]
fn a_gpu_reset_during_an_attempt_aborts_it_stays_sticky_and_records_the_stop() {
    let (mut port, answer, dir) = gpu_port(scan(0));
    port.observe_user_input_idle(Some(Instant::now()));
    assert_eq!(port.state(), GameState::IdentityMismatch);
    port.gpu.as_mut().unwrap().attempt_started_unix = Some(100);
    assert_eq!(port.state(), GameState::IdentityMismatch);
    *answer.lock().unwrap() = scan(2);
    port.gpu.as_mut().unwrap().last_poll = None;
    assert_eq!(port.state(), GameState::GpuReset);
    assert_eq!(incident_count(&dir), 1);
    assert!(port.health_blocked());
    *answer.lock().unwrap() = scan(0);
    assert_eq!(port.state(), GameState::GpuReset);
}

#[test]
fn a_reset_that_also_destroys_the_desktop_observation_is_still_a_gpu_reset() {
    let (mut port, answer, dir) = gpu_port(scan(0));
    // The compositor died with the reset: no fresh input-idle observation exists any more.
    port.observe_user_input_idle(None);
    port.gpu.as_mut().unwrap().attempt_started_unix = Some(100);
    *answer.lock().unwrap() = scan(1);
    port.gpu.as_mut().unwrap().last_poll = None;
    assert_eq!(port.state(), GameState::GpuReset);
    assert_eq!(incident_count(&dir), 1);
}

#[test]
fn repeated_inability_to_check_during_an_attempt_is_a_fatal_unknown() {
    let (mut port, _, dir) = gpu_port(Err(anyhow::anyhow!("journal unreadable")));
    port.observe_user_input_idle(Some(Instant::now()));
    port.gpu.as_mut().unwrap().attempt_started_unix = Some(100);
    for _ in 0..2 {
        assert_eq!(port.state(), GameState::IdentityMismatch);
        port.gpu.as_mut().unwrap().last_poll = None;
    }
    assert_eq!(port.state(), GameState::GpuReset);
    assert_eq!(incident_count(&dir), 1);
}

#[test]
fn admission_is_withheld_while_an_incident_is_unacknowledged() {
    let (mut port, answer, dir) = gpu_port(scan(1));
    assert!(!port.safe_window());
    assert_eq!(incident_count(&dir), 1);
    *answer.lock().unwrap() = scan(0);
    port.gpu.as_mut().unwrap().admission = None;
    assert!(!port.safe_window());
    assert!(port.health_blocked());
}

#[test]
fn a_restarted_port_over_the_same_state_stays_stopped_until_that_incident_is_acknowledged() {
    let (mut port, answer, dir) = gpu_port(scan(1));
    assert!(!port.safe_window());
    *answer.lock().unwrap() = scan(0);
    // A fresh controller process: new port, new guard, same private directory.
    let client = crate::autotune::research::tests::intent().steam_client;
    let mut restarted = ProductionSteamPort::new(
        "/study/compatdata".into(),
        "/study".into(),
        "/a".into(),
        "/m".into(),
        client,
        vec![],
        QuietProbe,
    )
    .with_gpu_reset_probe(Box::new(FakeGpu(answer.clone())), dir.path().to_path_buf());
    assert!(restarted.health_blocked());
    assert!(!restarted.safe_window());
    // Acknowledge exactly that incident, as a human would.
    let incident = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(super::super::gpu_health::INCIDENT_PREFIX)
        })
        .unwrap();
    let hash = stutter_core::hash::sha256_hex(&std::fs::read(&incident).unwrap())[..16].to_string();
    let ack = dir.path().join(super::super::gpu_health::ACK_FILE);
    std::fs::write(
        &ack,
        format!(
            "{} {hash}\n",
            incident.file_stem().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&ack, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    assert!(!restarted.health_blocked());
}
