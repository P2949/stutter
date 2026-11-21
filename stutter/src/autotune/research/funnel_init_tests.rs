//! The funnel initializes cold arms before any study, through the lease-visible fence, and a launch
//! that cannot be proven gone ends the campaign instead of letting a study start.

use std::sync::{Arc, Mutex};

use super::{funnel::funnel, *};
use crate::autotune::research::{
    init_phase::{InitArm, InitPhase},
    prefix_init::MAX_INIT_SECONDS,
    prefix_init_stage::{InitHost, InitProcess},
};

#[derive(Default)]
struct Seen {
    launches: u32,
}

struct Synthetic {
    seen: Arc<Mutex<Seen>>,
    arm: InitArm,
    terminate_fails: bool,
    polls: u32,
}

impl InitHost for Synthetic {
    fn launch(&mut self, _: ProtonSource) -> Result<(), &'static str> {
        self.seen.lock().unwrap().launches += 1;
        self.polls = 0;
        Ok(())
    }
    fn process(&mut self) -> Result<InitProcess, &'static str> {
        self.polls += 1;
        if self.polls < 2 {
            return Ok(InitProcess::Running);
        }
        let pfx = self.arm.compat.join("pfx");
        std::fs::create_dir_all(&pfx).unwrap();
        for name in ["system.reg", "user.reg", "userdef.reg"] {
            std::fs::write(pfx.join(name), b"x").unwrap();
        }
        std::fs::write(&self.arm.settings, b"{}").unwrap();
        Ok(InitProcess::Settled)
    }
    fn terminate(&mut self) -> Result<(), &'static str> {
        if self.terminate_fails {
            Err("SettlementUnproven")
        } else {
            Ok(())
        }
    }
}

fn phase(root: &tempfile::TempDir, seen: &Arc<Mutex<Seen>>, terminate_fails: bool) -> InitPhase {
    let compat = root.path().join("compat-ge");
    std::fs::create_dir_all(&compat).unwrap();
    let arm = InitArm {
        source: ProtonSource::Ge,
        settings: compat.join("pfx/UserSettings.json"),
        compat,
    };
    let dir = File::open(root.path()).unwrap();
    InitPhase::new(
        dir,
        crate::syscall::geteuid(),
        vec![arm.clone()],
        Box::new(Synthetic {
            seen: seen.clone(),
            arm,
            terminate_fails,
            polls: 0,
        }),
        MAX_INIT_SECONDS,
    )
}

#[test]
fn no_study_launches_until_every_cold_arm_is_initialized() {
    let (root, driver, parent) = funnel(Host::finishing(), 1);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut driver = driver.with_init_phase(phase(&root, &seen, false));
    let mut fences = 0;
    let mut steps = Vec::new();
    for tick in 0..400 {
        let step = driver
            .tick_at(parent.issued_unix_seconds + 1 + tick, &mut || {
                fences += 1;
                Ok(())
            })
            .unwrap();
        let done = step == Step::Done;
        steps.push(step);
        if done {
            break;
        }
    }
    assert_eq!(steps.last(), Some(&Step::Done));
    assert_eq!(seen.lock().unwrap().launches, 1);
    // The initialization launch passed the fence once, then the two screens' four attempts.
    assert_eq!(fences, 5);
    // While the phase ran, only waiting steps were shown: nothing from a study.
    let first_run = steps
        .iter()
        .position(|s| matches!(s, Step::Running(_)))
        .unwrap();
    assert!(
        first_run >= 3,
        "the phase took ticks before the first study attempt"
    );
    assert!(steps[..first_run].iter().all(|s| *s == Step::Waiting));
    assert_eq!(driver.init_outcomes().len(), 1);
    assert!(driver.init_outcomes()[0].verified);
}

#[test]
fn an_initialization_that_cannot_be_settled_ends_the_campaign_before_any_study() {
    let (root, driver, parent) = funnel(Host::finishing(), 1);
    let seen = Arc::new(Mutex::new(Seen::default()));
    let mut driver = driver.with_init_phase(phase(&root, &seen, true));
    // Planned, launched (a "restart" is simulated by cancelling while the launch is in flight).
    for tick in 0..2 {
        assert_eq!(
            driver
                .tick_at(parent.issued_unix_seconds + 1 + tick, &mut || Ok(()))
                .unwrap(),
            Step::Waiting
        );
    }
    driver.cancel();
    let step = driver
        .tick_at(parent.issued_unix_seconds + 10, &mut || Ok(()))
        .unwrap();
    assert_eq!(step, Step::OwnershipUnresolved("SettlementUnproven"));
    assert!(driver.dispositions().iter().all(|d| d.result.is_none()));
    assert_eq!(driver.shutdown(), Err("SettlementUnproven"));
}
