use std::{collections::VecDeque, os::unix::fs::PermissionsExt};

use super::*;
use crate::autotune::research::report::AttemptEvidenceV1;

struct Fake {
    safe: bool,
    polls: VecDeque<Progress>,
    launch_fails: bool,
    launches: u32,
    aborts: u32,
    abort_fails: bool,
    current: u32,
    auto_finish: bool,
}

fn ev(slot_index: u32) -> AttemptEvidenceV1 {
    AttemptEvidenceV1 {
        schema_version: 1,
        slot_index,
        average_fps: 60.0 + f64::from(slot_index),
        native_completion: None,
    }
}

impl AttemptHost for Fake {
    fn safe_window(&mut self) -> bool {
        self.safe
    }
    fn arm_spec(&mut self, _: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        Ok(ArmSpec {
            tool_dir: "/t".into(),
            study_compat_data: "/s".into(),
            arm_env: Default::default(),
            steamapps: "/a".into(),
            proton_manifest: "/m".into(),
        })
    }
    fn launch(&mut self, slot: &AttemptSlot, _: &DispatchContextV1) -> anyhow::Result<()> {
        self.current = slot.index;
        self.launches += 1;
        anyhow::ensure!(!self.launch_fails, "steam refused");
        Ok(())
    }
    fn poll(&mut self) -> Progress {
        if self.auto_finish {
            return Progress::Finished {
                evidence: ev(self.current),
            };
        }
        self.polls.pop_front().unwrap_or(Progress::Running)
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        self.aborts += 1;
        anyhow::ensure!(!self.abort_fails, "settlement unverified");
        Ok(())
    }
}

fn fake() -> Fake {
    Fake {
        safe: true,
        polls: VecDeque::new(),
        launch_fails: false,
        launches: 0,
        aborts: 0,
        abort_fails: false,
        current: 0,
        auto_finish: false,
    }
}

fn setup(max_attempts: u32) -> (tempfile::TempDir, Driver, ResearchIntentV1) {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["journal", "dispatch", "evidence"] {
        std::fs::create_dir(dir.path().join(sub)).unwrap();
        std::fs::set_permissions(dir.path().join(sub), std::fs::Permissions::from_mode(0o700))
            .unwrap();
    }
    let mut intent = super::super::tests::intent();
    intent.uid = crate::syscall::geteuid();
    intent.tools.push(super::super::ResearchTool {
        source: super::super::ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    intent.max_attempts = max_attempts;
    intent.max_attempt_seconds = 100;
    let schedule = build_schedule(&intent).unwrap();
    let len = schedule.len();
    let journal = ResearchJournal::open(
        crate::syscall::open_dir_no_symlinks(&dir.path().join("journal")).unwrap(),
        intent.uid,
        schedule,
    )
    .unwrap();
    let root =
        File::from(crate::syscall::open_dir_no_symlinks(&dir.path().join("dispatch")).unwrap());
    let evidence =
        File::from(crate::syscall::open_dir_no_symlinks(&dir.path().join("evidence")).unwrap());
    (
        dir,
        Driver::new(journal, intent.clone(), root, evidence, len),
        intent,
    )
}

#[test]
fn schedule_is_deterministic_complete_and_blocks_are_balanced() {
    let (_dir, _driver, intent) = setup(4);
    let a = build_schedule(&intent).unwrap();
    assert_eq!(a, build_schedule(&intent).unwrap());
    assert_eq!(a.len(), 2 * (4 / intent.tools.len()));
}

#[test]
fn waits_for_a_safe_window_then_runs_each_attempt_once_to_done() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.safe = false;
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Waiting);
    assert_eq!(host.launches, 0);
    host.safe = true;
    host.auto_finish = true;
    let mut sealed = 0;
    for t in 0..40 {
        match driver.tick(&mut host, now + t).unwrap() {
            Step::Sealed(record) => {
                assert!(matches!(record.outcome, AttemptOutcome::Completed { .. }));
                sealed += 1;
            }
            Step::Done => break,
            _ => {}
        }
    }
    assert_eq!(sealed, build_schedule(&intent).unwrap().len());
    assert_eq!(host.launches as usize, sealed);
}

#[test]
fn launch_refusal_timeout_and_cancel_seal_failed_without_replay() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.launch_fails = true;
    let now = intent.issued_unix_seconds + 1;
    let Step::Sealed(record) = driver.tick(&mut host, now).unwrap() else {
        panic!("launch refusal must seal")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "LaunchRefused".into()
        }
    );
    assert_eq!(host.aborts, 1);
    host.launch_fails = false;
    assert_eq!(driver.tick(&mut host, now + 1).unwrap(), Step::Running(1));
    let Step::Sealed(record) = driver.tick(&mut host, now + 1 + 100).unwrap() else {
        panic!("timeout must seal")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "AttemptTimeout".into()
        }
    );
    assert_eq!(driver.tick(&mut host, now + 200).unwrap(), Step::Running(2));
    driver.cancel();
    let Step::Sealed(record) = driver.tick(&mut host, now + 201).unwrap() else {
        panic!("cancel must seal")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "Cancelled".into()
        }
    );
    assert_eq!(driver.tick(&mut host, now + 202).unwrap(), Step::Cancelled);
    assert_eq!(host.launches, 3);
}

#[test]
fn deadline_expires_an_idle_driver_and_aborts_an_active_attempt() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    let Step::Sealed(record) = driver
        .tick(&mut host, intent.deadline_unix_seconds)
        .unwrap()
    else {
        panic!("deadline must seal the active attempt")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "CampaignDeadline".into()
        }
    );
    assert_eq!(
        driver
            .tick(&mut host, intent.deadline_unix_seconds)
            .unwrap(),
        Step::Expired
    );
}

#[test]
fn a_restart_with_an_unsealed_started_attempt_refuses_to_resume() {
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    driver
        .tick(&mut host, intent.issued_unix_seconds + 1)
        .unwrap();
    drop(driver);
    let reopened = ResearchJournal::open(
        crate::syscall::open_dir_no_symlinks(&dir.path().join("journal")).unwrap(),
        intent.uid,
        build_schedule(&intent).unwrap(),
    );
    assert!(format!("{:#}", reopened.err().unwrap()).contains("UnsafeToResume"));
}

#[test]
fn failed_process_settlement_never_seals_the_attempt() {
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    host.abort_fails = true;
    driver.cancel();
    assert!(driver.tick(&mut host, now + 1).is_err());
    assert!(dir.path().join("journal/0000.started").exists());
    assert!(!dir.path().join("journal/0000.json").exists());
    drop(driver);
    let reopened = ResearchJournal::open(
        crate::syscall::open_dir_no_symlinks(&dir.path().join("journal")).unwrap(),
        intent.uid,
        build_schedule(&intent).unwrap(),
    );
    assert!(reopened.is_err());
}

#[test]
fn a_host_reported_failure_aborts_and_seals_with_its_code() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    host.polls.push_back(Progress::Failed {
        code: "GameCrashed".into(),
    });
    let Step::Sealed(record) = driver.tick(&mut host, now + 1).unwrap() else {
        panic!("host failure must seal")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "GameCrashed".into()
        }
    );
    assert_eq!(host.aborts, 1);
}
