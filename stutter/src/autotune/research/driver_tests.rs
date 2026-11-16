use std::{collections::VecDeque, os::unix::fs::PermissionsExt};

use super::*;
use crate::autotune::research::report::AttemptEvidenceV1;

struct Fake {
    safe: bool,
    polls: VecDeque<Progress>,
    launch_fails: bool,
    launch_unresolved: bool,
    launches: u32,
    aborts: u32,
    abort_fails: bool,
    current: u32,
    auto_finish: bool,
    arm_delay: std::time::Duration,
    safe_delay: std::time::Duration,
    rename_on_launch: Option<(std::path::PathBuf, std::path::PathBuf)>,
    health_stop: bool,
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
        std::thread::sleep(self.safe_delay);
        self.safe
    }
    fn arm_spec(&mut self, _: &AttemptSlot) -> anyhow::Result<ArmSpec> {
        std::thread::sleep(self.arm_delay);
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
        if let Some((source, destination)) = &self.rename_on_launch {
            std::fs::rename(source, destination)?;
        }
        if self.launch_unresolved {
            return Err(LaunchUnresolved("lost acknowledgement").into());
        }
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
    fn health_blocked(&mut self) -> bool {
        self.health_stop
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
        launch_unresolved: false,
        launches: 0,
        aborts: 0,
        abort_fails: false,
        current: 0,
        auto_finish: false,
        arm_delay: std::time::Duration::ZERO,
        safe_delay: std::time::Duration::ZERO,
        rename_on_launch: None,
        health_stop: false,
    }
}

#[test]
fn deadline_expiring_during_preparation_never_starts_steam() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.arm_delay = std::time::Duration::from_millis(1100);
    let step = driver
        .tick(&mut host, intent.deadline_unix_seconds - 2)
        .unwrap();
    let Step::Sealed(record) = step else {
        panic!("expired authorization must be settled")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "AuthorizationExpiredBeforeLaunch".into()
        }
    );
    assert_eq!(host.launches, 0);
    assert_eq!(host.aborts, 1);
}

#[test]
fn deadline_expiring_during_safe_window_check_never_starts_steam() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.safe_delay = std::time::Duration::from_millis(1100);
    let step = driver
        .tick(&mut host, intent.deadline_unix_seconds - 2)
        .unwrap();
    let Step::Sealed(record) = step else {
        panic!("expired authorization must be settled")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "AuthorizationExpiredBeforeLaunch".into()
        }
    );
    assert_eq!(host.launches, 0);
    assert_eq!(host.aborts, 1);
}

#[test]
fn launch_ticket_lifetime_is_bounded_by_remaining_authorization() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    assert_eq!(
        driver
            .tick(&mut host, intent.deadline_unix_seconds - 60)
            .unwrap(),
        Step::Running(0)
    );
    assert_eq!(host.launches, 1);
}

fn setup(max_attempts: u32) -> (tempfile::TempDir, Driver, ResearchIntentV1) {
    setup_with_budget(max_attempts, None)
}

fn setup_with_budget(
    max_attempts: u32,
    active_budget: Option<u32>,
) -> (tempfile::TempDir, Driver, ResearchIntentV1) {
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
    if let Some(budget) = active_budget {
        intent.max_active_seconds = budget;
        intent.max_attempt_seconds = budget;
        intent.max_owned_pressure_seconds_per_study = budget;
    }
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
fn failed_launch_fence_leaves_started_journal_without_tickets_or_launch() {
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let result = driver.tick_with_launch_fence(&mut host, intent.issued_unix_seconds + 1, || {
        anyhow::bail!("lease persistence failed")
    });
    assert!(result.is_err());
    assert_eq!(host.launches, 0);
    assert!(dir.path().join("journal/0000.started").is_file());
    assert!(!dir.path().join("journal/0000.json").exists());
    assert_eq!(
        std::fs::read_dir(dir.path().join("dispatch"))
            .unwrap()
            .count(),
        0
    );
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
fn an_unresolved_launch_is_never_sealed_aborted_or_retried() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.launch_unresolved = true;
    let now = intent.issued_unix_seconds + 1;
    let error = driver.tick(&mut host, now).unwrap_err();
    assert!(error.downcast_ref::<LaunchUnresolved>().is_some());
    // A no-op abort would not show the request cannot start later, so none is attempted.
    assert_eq!(host.aborts, 0);
    assert_eq!(host.launches, 1);
    // Nothing was sealed, and no later tick may launch, abort or seal anything.
    host.launch_unresolved = false;
    let again = driver.tick(&mut host, now + 1).unwrap_err();
    assert!(again.to_string().contains("UnsafeToResume"));
    assert_eq!(host.launches, 1);
    assert_eq!(host.aborts, 0);
    driver.cancel();
    assert!(driver.tick(&mut host, now + 2).is_err());
    assert_eq!(host.launches, 1);
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
fn refused_launch_retires_every_issued_ticket_before_sealing() {
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.launch_fails = true;
    assert!(matches!(
        driver
            .tick(&mut host, intent.issued_unix_seconds + 1)
            .unwrap(),
        Step::Sealed(_)
    ));
    let attempt = dir.path().join("dispatch/0000");
    let ticket_names: Vec<_> = std::fs::read_dir(&attempt)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("ticket-") && name.ends_with(".json"))
        .collect();
    assert_eq!(ticket_names.len(), 2);
    for name in ticket_names {
        let id = name
            .strip_prefix("ticket-")
            .unwrap()
            .strip_suffix(".json")
            .unwrap();
        assert_eq!(
            std::fs::read(attempt.join(format!("used-{id}"))).unwrap(),
            b"revoked-v1"
        );
    }
}

#[test]
fn disappeared_attempt_directory_cannot_be_treated_as_no_tickets() {
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.launch_fails = true;
    host.rename_on_launch = Some((
        dir.path().join("dispatch/0000"),
        dir.path().join("moved-attempt"),
    ));
    assert!(
        driver
            .tick(&mut host, intent.issued_unix_seconds + 1)
            .is_err()
    );
    assert!(dir.path().join("journal/0000.started").is_file());
    assert!(!dir.path().join("journal/0000.json").exists());
}

#[test]
fn preexisting_attempt_directory_is_not_modified_after_arming_failure() {
    let (dir, mut driver, intent) = setup(4);
    let attempt = dir.path().join("dispatch/0000");
    std::fs::create_dir(&attempt).unwrap();
    let id = "a".repeat(64);
    std::fs::write(attempt.join(format!("ticket-{id}.json")), b"existing").unwrap();
    let mut host = fake();
    assert!(
        driver
            .tick(&mut host, intent.issued_unix_seconds + 1)
            .is_err()
    );
    assert_eq!(host.launches, 0);
    assert!(!attempt.join(format!("used-{id}")).exists());
    assert!(!dir.path().join("journal/0000.json").exists());
}

#[test]
fn claimed_ticket_keeps_the_attempt_unsealed() {
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.auto_finish = true;
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    let attempt = dir.path().join("dispatch/0000");
    let name = std::fs::read_dir(&attempt)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .find(|name| name.starts_with("ticket-") && name.ends_with(".json"))
        .unwrap();
    let id = name
        .strip_prefix("ticket-")
        .unwrap()
        .strip_suffix(".json")
        .unwrap();
    std::fs::write(attempt.join(format!("used-{id}")), b"claimed").unwrap();
    assert!(driver.tick(&mut host, now + 1).is_err());
    assert!(dir.path().join("journal/0000.started").is_file());
    assert!(!dir.path().join("journal/0000.json").exists());
}

#[path = "driver_claim_tests.rs"]
mod claim_tests;

#[test]
fn missing_issued_ticket_keeps_the_attempt_unsealed() {
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.auto_finish = true;
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    let attempt = dir.path().join("dispatch/0000");
    let ticket = std::fs::read_dir(&attempt)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("ticket-")
        })
        .unwrap();
    std::fs::remove_file(ticket).unwrap();
    assert!(driver.tick(&mut host, now + 1).is_err());
    assert!(dir.path().join("journal/0000.started").is_file());
    assert!(!dir.path().join("journal/0000.json").exists());
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
fn active_budget_accumulates_across_attempts_and_refuses_another_launch() {
    let (_dir, mut driver, intent) = setup_with_budget(4, Some(5));
    let mut host = fake();
    host.auto_finish = true;
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    assert!(matches!(
        driver.tick(&mut host, now + 2).unwrap(),
        Step::Sealed(_)
    ));
    assert_eq!(driver.tick(&mut host, now + 3).unwrap(), Step::Running(1));
    let Step::Sealed(record) = driver.tick(&mut host, now + 4).unwrap() else {
        panic!("active budget must settle the active attempt")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "ActiveBudgetExceeded".into()
        }
    );
    assert_eq!(host.aborts, 1);
    assert_eq!(
        driver.tick(&mut host, now + 5).unwrap(),
        Step::ActiveBudgetExhausted
    );
    assert_eq!(host.launches, 2);
}

#[test]
fn backward_clock_never_extends_the_active_budget() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let now = intent.issued_unix_seconds + 2;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    assert!(driver.tick(&mut host, now - 1).is_err());
}

#[test]
fn sealed_attempt_history_cannot_reset_the_active_budget_on_restart() {
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.auto_finish = true;
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    assert!(matches!(
        driver.tick(&mut host, now + 1).unwrap(),
        Step::Sealed(_)
    ));
    drop(driver);
    let schedule = build_schedule(&intent).unwrap();
    let journal = ResearchJournal::open(
        crate::syscall::open_dir_no_symlinks(&dir.path().join("journal")).unwrap(),
        intent.uid,
        schedule.clone(),
    )
    .unwrap();
    let root =
        File::from(crate::syscall::open_dir_no_symlinks(&dir.path().join("dispatch")).unwrap());
    let evidence =
        File::from(crate::syscall::open_dir_no_symlinks(&dir.path().join("evidence")).unwrap());
    let mut restarted = Driver::new(journal, intent, root, evidence, schedule.len());
    let error = restarted.tick(&mut host, now + 2).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("active-time budget requires independent recovery")
    );
    assert_eq!(host.launches, 1);
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

#[test]
fn a_restarted_driver_seals_only_a_provably_settled_interrupted_attempt() {
    use super::super::{
        launch_record::{record_outcome_with_invocation, record_phase},
        launch_txn::{LaunchOutcome, LaunchPhase, UnitState},
        recovery_gather::RecoveryWitnesses,
        recovery_plan::RecoveryDecision,
        user_launcher::launch_unit_name,
    };
    let (dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    // The controller crashes while attempt 0 is launched and unsealed.
    drop(driver);

    let schedule = build_schedule(&intent).unwrap();
    let len = schedule.len();
    let (journal, adopted) = ResearchJournal::open_for_recovery(
        crate::syscall::open_dir_no_symlinks(&dir.path().join("journal")).unwrap(),
        intent.uid,
        schedule,
    )
    .unwrap();
    let adopted = adopted.expect("the started attempt is adopted");
    let root =
        File::from(crate::syscall::open_dir_no_symlinks(&dir.path().join("dispatch")).unwrap());
    let evidence =
        File::from(crate::syscall::open_dir_no_symlinks(&dir.path().join("evidence")).unwrap());
    let mut recovered = Driver::new(journal, intent.clone(), root, evidence, len);

    // No ticket is claimed in this scenario, so the claim verifier must never be consulted.
    struct NoClaims;
    impl ClaimSettlementVerifier for NoClaims {
        fn verify_process(
            &mut self,
            _: &File,
            _: &DispatchContextV1,
            _: &super::super::launch_ticket::ResearchLaunchTicketV1,
        ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
            anyhow::bail!("no claim was expected")
        }
        fn verify_worker(
            &mut self,
            _: &DispatchContextV1,
        ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
            anyhow::bail!("no claim was expected")
        }
    }
    let settled = |unit: UnitState| {
        move |driver: &mut Driver, adopted: AttemptSlot, now: u64| {
            driver.recover_interrupted(
                adopted,
                true,
                RecoveryWitnesses {
                    unit_state: Box::new(move |_| Ok(unit)),
                    unit_invocation: Box::new(|_| {
                        Ok(Some("0123456789abcdef0123456789abcdef".into()))
                    }),
                    process_tree_settled: Box::new(|_, _| Ok(true)),
                    outstanding_worker_tokens: Box::new(|| Ok(0)),
                    transactions_restored: Box::new(|| Ok(true)),
                    claim_verifier: Box::new(NoClaims),
                },
                now,
            )
        }
    };
    // The launch left only a request marker, no transaction record: never assumed settled.
    let error = settled(UnitState::Finished)(&mut recovered, adopted.clone(), now + 1).unwrap_err();
    assert!(error.to_string().contains("UnsafeToResume"), "{error:#}");
    assert!(recovered.journal.verify().is_err(), "nothing was sealed");

    // With a durable sent-and-reconciled launch and every witness settled, it is retired.
    let unit = launch_unit_name(&intent.digest().unwrap(), adopted.index).unwrap();
    let attempt = File::from(
        crate::syscall::openat_dir_nofollow(
            File::from(crate::syscall::open_dir_no_symlinks(&dir.path().join("dispatch")).unwrap())
                .as_fd(),
            "0000",
        )
        .unwrap(),
    );
    record_phase(&attempt, LaunchPhase::Prepared, &unit).unwrap();
    record_phase(&attempt, LaunchPhase::Sent, &unit).unwrap();
    record_outcome_with_invocation(
        &attempt,
        &unit,
        &LaunchOutcome::Running,
        Some("0123456789abcdef0123456789abcdef"),
    )
    .unwrap();
    // A launcher still running is not settled.
    assert!(
        settled(UnitState::Active)(&mut recovered, adopted.clone(), now + 2)
            .unwrap_err()
            .to_string()
            .contains("UnsafeToResume")
    );
    let decision = settled(UnitState::Finished)(&mut recovered, adopted, now + 3).unwrap();
    assert_eq!(
        decision,
        RecoveryDecision::RetireAndSealFailed("InterruptedBeforeDispatch")
    );
    let sealed = recovered.journal.verify().unwrap();
    assert_eq!(sealed.len(), 1);
    assert!(matches!(
        &sealed[0].outcome,
        AttemptOutcome::Failed { code } if code == "InterruptedBeforeDispatch"
    ));
}

#[path = "driver_tests_health.rs"]
mod health;
