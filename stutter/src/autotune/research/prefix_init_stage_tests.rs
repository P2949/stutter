use std::os::unix::fs::PermissionsExt;

use super::*;

const ARM: ProtonSource = ProtonSource::Ge;
const T0: u64 = 1_000_000;
const AMPLE: u64 = MAX_INIT_SECONDS;

struct Fixture {
    _root: tempfile::TempDir,
    journal: File,
    compat: std::path::PathBuf,
    settings: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let study = std::fs::canonicalize(root.path()).unwrap();
    std::fs::set_permissions(&study, std::fs::Permissions::from_mode(0o700)).unwrap();
    let compat = study.join("compatdata/ge");
    std::fs::create_dir_all(&compat).unwrap();
    Fixture {
        journal: File::open(&study).unwrap(),
        settings: compat.join("pfx/UserSettings.json"),
        compat,
        _root: root,
    }
}

/// What a real supervised launch would have left in the prefix.
fn create_prefix(f: &Fixture) {
    let pfx = f.compat.join("pfx");
    std::fs::create_dir_all(&pfx).unwrap();
    for name in ["system.reg", "user.reg", "userdef.reg"] {
        std::fs::write(pfx.join(name), b"x").unwrap();
    }
    std::fs::write(&f.settings, b"{}").unwrap();
    std::fs::write(f.compat.join("version"), b"11.0-100").unwrap();
}

#[derive(Default)]
struct Host {
    launches: u32,
    refuse: Option<&'static str>,
    state: Option<InitProcess>,
    process_error: Option<&'static str>,
    terminate_fails: bool,
    terminated: u32,
}

impl InitHost for Host {
    fn launch(&mut self, _: ProtonSource) -> Result<(), &'static str> {
        if let Some(reason) = self.refuse {
            return Err(reason);
        }
        self.launches += 1;
        self.state = Some(InitProcess::Running);
        Ok(())
    }
    fn process(&mut self) -> Result<InitProcess, &'static str> {
        if let Some(reason) = self.process_error {
            return Err(reason);
        }
        Ok(self.state.unwrap_or(InitProcess::Running))
    }
    fn terminate(&mut self) -> Result<(), &'static str> {
        self.terminated += 1;
        if self.terminate_fails {
            Err("SettlementUnproven")
        } else {
            self.state = Some(InitProcess::Settled);
            Ok(())
        }
    }
}

fn runner<'a>(f: &'a Fixture, seconds: u64) -> InitStageRunner<'a> {
    InitStageRunner::open(
        &f.journal,
        ARM,
        &f.compat,
        &f.settings,
        crate::syscall::geteuid(),
        seconds,
    )
    .unwrap()
}

#[test]
fn a_cold_arm_is_planned_launched_settled_and_verified_with_no_second_launch() {
    let f = fixture();
    let mut host = Host::default();
    let mut stage = runner(&f, AMPLE);
    assert_eq!(stage.tick(&mut host, T0), InitStep::Waiting); // planned
    assert_eq!(stage.tick(&mut host, T0 + 1), InitStep::Waiting); // launched
    assert_eq!(stage.tick(&mut host, T0 + 30), InitStep::Waiting); // still running
    create_prefix(&f);
    host.state = Some(InitProcess::Settled);
    assert_eq!(stage.tick(&mut host, T0 + 90), InitStep::Verified);
    assert_eq!(host.launches, 1);
    assert_eq!(stage.tick(&mut host, T0 + 91), InitStep::Verified);
    assert_eq!(host.launches, 1, "a verified arm is never launched again");
    // The journal on disk says the same to a fresh reader.
    assert!(
        super::super::prefix_init::initialized(
            &f.journal,
            ARM,
            &f.compat,
            crate::syscall::geteuid()
        )
        .unwrap()
    );
}

#[test]
fn a_launch_that_leaves_no_prefix_is_abandoned_and_the_arm_stays_blocked() {
    let f = fixture();
    let mut host = Host::default();
    let mut stage = runner(&f, AMPLE);
    stage.tick(&mut host, T0);
    stage.tick(&mut host, T0 + 1);
    host.state = Some(InitProcess::Settled); // exited, but created nothing
    assert_eq!(
        stage.tick(&mut host, T0 + 20),
        InitStep::Abandoned("PrefixNotInitialized")
    );
    assert_eq!(
        stage.tick(&mut host, T0 + 21),
        InitStep::Abandoned("InitAbandoned")
    );
    assert!(
        !super::super::prefix_init::initialized(
            &f.journal,
            ARM,
            &f.compat,
            crate::syscall::geteuid()
        )
        .unwrap()
    );
    assert_eq!(host.launches, 1, "an abandoned arm is not retried");
}

#[test]
fn an_overdue_launch_is_terminated_proven_gone_and_abandoned() {
    let f = fixture();
    let mut host = Host::default();
    let mut stage = runner(&f, AMPLE);
    stage.tick(&mut host, T0);
    stage.tick(&mut host, T0 + 1);
    assert_eq!(
        stage.tick(&mut host, T0 + 1 + MAX_INIT_SECONDS + 1),
        InitStep::Abandoned("InitTimedOut")
    );
    assert_eq!(host.terminated, 1);
}

#[test]
fn an_overdue_launch_that_cannot_be_proven_gone_leaves_ownership_unresolved() {
    let f = fixture();
    let mut host = Host {
        terminate_fails: true,
        ..Host::default()
    };
    let mut stage = runner(&f, AMPLE);
    stage.tick(&mut host, T0);
    stage.tick(&mut host, T0 + 1);
    assert_eq!(
        stage.tick(&mut host, T0 + 2 + MAX_INIT_SECONDS),
        InitStep::Unresolved("SettlementUnproven")
    );
    assert!(!stage.journal.verified());
}

#[test]
fn insufficient_authorized_time_or_a_refused_launch_abandons_without_launching() {
    let f = fixture();
    let mut host = Host::default();
    let mut short = runner(&f, MAX_INIT_SECONDS - 1);
    short.tick(&mut host, T0);
    assert_eq!(
        short.tick(&mut host, T0 + 1),
        InitStep::Abandoned("InitBudgetInsufficient")
    );
    assert_eq!(host.launches, 0);

    let g = fixture();
    let mut refusing = Host {
        refuse: Some("BlockedGpuStability"),
        ..Host::default()
    };
    let mut stage = runner(&g, AMPLE);
    stage.tick(&mut refusing, T0);
    assert_eq!(
        stage.tick(&mut refusing, T0 + 1),
        InitStep::Abandoned("BlockedGpuStability")
    );
}

#[test]
fn a_restart_finds_an_unfinished_launch_settles_it_and_never_replays_it() {
    let f = fixture();
    let mut host = Host::default();
    {
        let mut stage = runner(&f, AMPLE);
        stage.tick(&mut host, T0);
        stage.tick(&mut host, T0 + 1);
    }
    // The controller restarted: a new runner reads the journal and sees a launch in flight.
    let mut resumed = runner(&f, AMPLE);
    assert_eq!(
        resumed.recover(&mut host, T0 + 50),
        InitStep::Abandoned("InitInterrupted")
    );
    assert_eq!((host.launches, host.terminated), (1, 1));

    // If the old tree cannot be proven gone, nothing proceeds.
    let g = fixture();
    let mut stuck = Host {
        terminate_fails: true,
        ..Host::default()
    };
    {
        let mut stage = runner(&g, AMPLE);
        stage.tick(&mut stuck, T0);
        stage.tick(&mut stuck, T0 + 1);
    }
    assert_eq!(
        runner(&g, AMPLE).recover(&mut stuck, T0 + 5),
        InitStep::Unresolved("SettlementUnproven")
    );
}

#[test]
fn cancellation_settles_a_running_launch_and_an_unobservable_process_is_unresolved() {
    let f = fixture();
    let mut host = Host::default();
    let mut stage = runner(&f, AMPLE);
    stage.tick(&mut host, T0);
    stage.tick(&mut host, T0 + 1);
    assert_eq!(
        stage.cancel(&mut host, T0 + 2),
        InitStep::Abandoned("InitCancelled")
    );
    assert_eq!(host.terminated, 1);

    let g = fixture();
    let mut blind = Host {
        process_error: Some("SettlementUnproven"),
        ..Host::default()
    };
    let mut stage = runner(&g, AMPLE);
    stage.tick(&mut blind, T0);
    stage.tick(&mut blind, T0 + 1);
    assert_eq!(
        stage.tick(&mut blind, T0 + 2),
        InitStep::Unresolved("SettlementUnproven")
    );
    // Cancelling before anything was launched records the arm as given up, with no launch to settle.
    let h = fixture();
    let mut idle = Host::default();
    assert_eq!(
        runner(&h, AMPLE).cancel(&mut idle, T0),
        InitStep::Abandoned("InitCancelled")
    );
    assert_eq!(idle.terminated, 0);
}
