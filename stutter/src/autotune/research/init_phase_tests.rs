use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, Mutex},
};

use super::*;
use crate::autotune::research::prefix_init::MAX_INIT_SECONDS;

const T0: u64 = 2_000_000;

fn study_dir() -> (tempfile::TempDir, File) {
    let root = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(root.path()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = File::open(&path).unwrap();
    (root, dir)
}

fn create_prefix(compat: &Path, settings: &Path) {
    let pfx = compat.join("pfx");
    std::fs::create_dir_all(&pfx).unwrap();
    for name in ["system.reg", "user.reg", "userdef.reg"] {
        std::fs::write(pfx.join(name), b"x").unwrap();
    }
    std::fs::write(settings, b"{}").unwrap();
    std::fs::write(compat.join("version"), b"11.0-100").unwrap();
}

fn arm(root: &Path, source: ProtonSource) -> InitArm {
    let compat = root.join(format!("compat-{}", source.dir_name()));
    std::fs::create_dir_all(&compat).unwrap();
    InitArm {
        source,
        settings: compat.join("pfx/UserSettings.json"),
        compat,
    }
}

#[derive(Default)]
struct Log {
    launched: Vec<ProtonSource>,
    terminated: u32,
    fences: u32,
}

/// A host whose launch creates the prefix after `polls` polls, unless `creates` is false.
struct Synthetic {
    log: Arc<Mutex<Log>>,
    arms: Vec<InitArm>,
    current: Option<ProtonSource>,
    polls: u32,
    creates: bool,
    terminate_fails: bool,
}

impl InitHost for Synthetic {
    fn launch(&mut self, arm: ProtonSource) -> Result<(), &'static str> {
        self.log.lock().unwrap().launched.push(arm);
        self.current = Some(arm);
        self.polls = 0;
        Ok(())
    }
    fn process(&mut self) -> Result<InitProcess, &'static str> {
        self.polls += 1;
        if self.polls < 2 {
            return Ok(InitProcess::Running);
        }
        if self.creates
            && let Some(source) = self.current
            && let Some(arm) = self.arms.iter().find(|a| a.source == source)
        {
            create_prefix(&arm.compat, &arm.settings);
        }
        Ok(InitProcess::Settled)
    }
    fn terminate(&mut self) -> Result<(), &'static str> {
        self.log.lock().unwrap().terminated += 1;
        if self.terminate_fails {
            Err("SettlementUnproven")
        } else {
            Ok(())
        }
    }
}

fn drive(phase: &mut InitPhase, log: &Arc<Mutex<Log>>, mut now: u64) -> PhaseStep {
    for _ in 0..40 {
        now += 10;
        let log = log.clone();
        match phase.tick(now, &mut || {
            log.lock().unwrap().fences += 1;
            Ok(())
        }) {
            PhaseStep::Pending => {}
            done => return done,
        }
    }
    panic!("the phase never finished");
}

#[test]
fn only_cold_arms_are_initialized_one_launch_each_and_warm_arms_are_left_alone() {
    let (root, dir) = study_dir();
    let cold = arm(root.path(), ProtonSource::Ge);
    let warm = arm(root.path(), ProtonSource::ValveStable);
    create_prefix(&warm.compat, &warm.settings);
    let other = arm(root.path(), ProtonSource::CachyOs);
    let arms = vec![cold.clone(), warm, other.clone()];
    let log = Arc::new(Mutex::new(Log::default()));
    let host = Synthetic {
        log: log.clone(),
        arms: arms.clone(),
        current: None,
        polls: 0,
        creates: true,
        terminate_fails: false,
    };
    let mut phase = InitPhase::new(
        dir,
        crate::syscall::geteuid(),
        arms,
        Box::new(host),
        2 * MAX_INIT_SECONDS,
    );
    assert_eq!(drive(&mut phase, &log, T0), PhaseStep::Finished);
    let log = log.lock().unwrap();
    assert_eq!(log.launched, vec![ProtonSource::Ge, ProtonSource::CachyOs]);
    assert_eq!(log.fences, 2, "each launch passes the controller's fence");
    assert!(phase.outcomes().iter().all(|o| o.verified));
    assert_eq!(phase.outcomes().len(), 2);
    assert!(phase.used_seconds() > 0 && phase.used_seconds() < MAX_INIT_SECONDS);
}

#[test]
fn an_arm_that_fails_to_initialize_stays_cold_and_does_not_stop_the_others() {
    let (root, dir) = study_dir();
    let first = arm(root.path(), ProtonSource::Ge);
    let arms = vec![first.clone()];
    let log = Arc::new(Mutex::new(Log::default()));
    let host = Synthetic {
        log: log.clone(),
        arms: arms.clone(),
        current: None,
        polls: 0,
        creates: false,
        terminate_fails: false,
    };
    let mut phase = InitPhase::new(
        dir,
        crate::syscall::geteuid(),
        arms,
        Box::new(host),
        MAX_INIT_SECONDS,
    );
    assert_eq!(drive(&mut phase, &log, T0), PhaseStep::Finished);
    assert_eq!(
        phase.outcomes(),
        [ArmOutcome {
            source: ProtonSource::Ge,
            verified: false,
            reason: Some("PrefixNotInitialized"),
        }]
    );
    assert!(
        !first.compat.join("pfx").exists(),
        "the prefix is still cold"
    );
}

#[test]
fn the_authorized_time_is_shared_across_arms_and_never_stretched() {
    let (root, dir) = study_dir();
    let arms = vec![
        arm(root.path(), ProtonSource::Ge),
        arm(root.path(), ProtonSource::CachyOs),
    ];
    let log = Arc::new(Mutex::new(Log::default()));
    let host = Synthetic {
        log: log.clone(),
        arms: arms.clone(),
        current: None,
        polls: 0,
        creates: true,
        terminate_fails: false,
    };
    // Room for one full-bound launch only: the first arm runs, the second is abandoned unlaunched.
    let mut phase = InitPhase::new(
        dir,
        crate::syscall::geteuid(),
        arms,
        Box::new(host),
        MAX_INIT_SECONDS + 5,
    );
    assert_eq!(drive(&mut phase, &log, T0), PhaseStep::Finished);
    let log = log.lock().unwrap();
    assert_eq!(log.launched, vec![ProtonSource::Ge]);
    assert_eq!(phase.outcomes()[1].reason, Some("InitBudgetInsufficient"));
}

#[test]
fn a_restart_during_a_launch_settles_it_and_an_unprovable_settlement_ends_the_phase() {
    let (root, dir) = study_dir();
    let arms = vec![arm(root.path(), ProtonSource::Ge)];
    let log = Arc::new(Mutex::new(Log::default()));
    let uid = crate::syscall::geteuid();
    {
        let host = Synthetic {
            log: log.clone(),
            arms: arms.clone(),
            current: None,
            polls: 0,
            creates: false,
            terminate_fails: false,
        };
        let mut phase = InitPhase::new(
            dir.try_clone().unwrap(),
            uid,
            arms.clone(),
            Box::new(host),
            MAX_INIT_SECONDS,
        );
        // Two ticks: planned, then launched; the controller is "killed" here.
        for step in 1..=2 {
            assert_eq!(phase.tick(T0 + step, &mut || Ok(())), PhaseStep::Pending);
        }
    }
    let stuck = Synthetic {
        log: log.clone(),
        arms: arms.clone(),
        current: None,
        polls: 0,
        creates: false,
        terminate_fails: true,
    };
    let mut resumed = InitPhase::new(
        dir.try_clone().unwrap(),
        uid,
        arms.clone(),
        Box::new(stuck),
        MAX_INIT_SECONDS,
    );
    assert_eq!(
        resumed.tick(T0 + 60, &mut || Ok(())),
        PhaseStep::Unresolved("SettlementUnproven")
    );
    // A proven settlement abandons the interrupted launch rather than replaying it.
    let fine = Synthetic {
        log: log.clone(),
        arms: arms.clone(),
        current: None,
        polls: 0,
        creates: false,
        terminate_fails: false,
    };
    let mut again = InitPhase::new(dir, uid, arms, Box::new(fine), MAX_INIT_SECONDS);
    assert_eq!(drive(&mut again, &log, T0 + 100), PhaseStep::Finished);
    assert_eq!(log.lock().unwrap().launched.len(), 1, "never replayed");
    assert_eq!(again.outcomes()[0].reason, Some("InitInterrupted"));
}
