use super::*;

#[test]
fn urgent_observations_survive_simultaneous_cancellation_and_deadline() {
    for class in [
        FailureClass::Integrity,
        FailureClass::Health,
        FailureClass::OwnershipUnresolved,
    ] {
        for cancel in [false, true] {
            let (_dir, mut driver, intent) = setup(4);
            let mut host = fake();
            let now = intent.issued_unix_seconds + 1;
            assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
            host.polls.push_back(Progress::Failed {
                class,
                code: "Urgent".into(),
            });
            if cancel {
                driver.cancel();
            }
            let step = driver
                .tick(&mut host, intent.deadline_unix_seconds)
                .unwrap();
            if class == FailureClass::OwnershipUnresolved {
                assert!(matches!(step, Step::OwnershipUnresolved(_)));
            } else {
                assert!(
                    matches!(step, Step::Sealed(ref record) if matches!(record.outcome, AttemptOutcome::ClassifiedFailure { class: found, .. } if found == class))
                );
                assert_eq!(
                    driver
                        .tick(&mut host, intent.deadline_unix_seconds + 1)
                        .unwrap(),
                    if class == FailureClass::Health {
                        Step::Stopped(FailureClass::Health)
                    } else {
                        Step::Stopped(class)
                    }
                );
            }
        }
    }
}

#[test]
fn typed_attempt_failures_stop_or_continue_without_interpreting_their_message() {
    for class in [
        FailureClass::Incompatible,
        FailureClass::Environment,
        FailureClass::Resource,
        FailureClass::Integrity,
        FailureClass::Health,
        FailureClass::OwnershipUnresolved,
    ] {
        let (_dir, mut driver, intent) = setup(4);
        let mut host = fake();
        let now = intent.issued_unix_seconds + 1;
        assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
        host.polls.push_back(Progress::Failed {
            class,
            code: "SameDiagnostic".into(),
        });
        let step = driver.tick(&mut host, now + 1).unwrap();
        if class == FailureClass::OwnershipUnresolved {
            assert!(matches!(step, Step::OwnershipUnresolved(_)));
            assert!(
                driver.records().is_err(),
                "started attempt must stay unsealed"
            );
            assert!(driver.tick(&mut host, now + 2).is_err());
        } else {
            assert!(matches!(step, Step::Sealed(_)));
            let next = driver.tick(&mut host, now + 2).unwrap();
            match class {
                FailureClass::Incompatible => assert_eq!(next, Step::Running(1)),
                FailureClass::Health => assert_eq!(next, Step::Stopped(FailureClass::Health)),
                _ => assert_eq!(next, Step::Stopped(class)),
            }
        }
        assert_eq!(
            host.launches,
            if class == FailureClass::Incompatible {
                2
            } else {
                1
            }
        );
    }
}

#[test]
fn rejected_evidence_is_sealed_then_latches_an_integrity_stop() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    host.polls
        .push_back(Progress::Finished { evidence: ev(999) });
    assert!(matches!(
        driver.tick(&mut host, now + 1).unwrap(),
        Step::Sealed(_)
    ));
    assert_eq!(
        driver.tick(&mut host, now + 2).unwrap(),
        Step::Stopped(FailureClass::Integrity)
    );
    assert_eq!(host.launches, 1);
}

#[test]
fn rejected_evidence_is_not_hidden_by_cancellation() {
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    host.polls
        .push_back(Progress::Finished { evidence: ev(999) });
    driver.cancel();
    assert!(
        matches!(driver.tick(&mut host, now + 1).unwrap(), Step::Sealed(ref record) if matches!(record.outcome, AttemptOutcome::ClassifiedFailure { class: FailureClass::Integrity, .. }))
    );
    assert_eq!(
        driver.tick(&mut host, now + 2).unwrap(),
        Step::Stopped(FailureClass::Integrity)
    );
}

#[test]
fn a_restarted_driver_reconstructs_a_terminal_class_from_the_sealed_journal() {
    for class in [
        FailureClass::Integrity,
        FailureClass::Environment,
        FailureClass::Resource,
        FailureClass::Health,
    ] {
        let (dir, mut driver, intent) = setup(4);
        let mut host = fake();
        let now = intent.issued_unix_seconds + 1;
        assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
        host.polls.push_back(Progress::Failed {
            class,
            code: "SameDiagnostic".into(),
        });
        assert!(matches!(
            driver.tick(&mut host, now + 1).unwrap(),
            Step::Sealed(_)
        ));
        drop(driver);
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
        let mut recovered = Driver::new(journal, intent, root, evidence, len);
        let expected = if class == FailureClass::Health {
            Step::Stopped(FailureClass::Health)
        } else {
            Step::Stopped(class)
        };
        assert_eq!(recovered.tick(&mut host, now + 2).unwrap(), expected);
        assert_eq!(host.launches, 1);
    }
}
#[test]
fn typed_launch_failures_stop_after_settlement_and_unresolved_ownership_is_retained() {
    for class in [
        FailureClass::Environment,
        FailureClass::Resource,
        FailureClass::Integrity,
        FailureClass::Health,
        FailureClass::OwnershipUnresolved,
    ] {
        let (_dir, mut driver, intent) = setup(4);
        let mut host = fake();
        host.launch_class = Some(class);
        let now = intent.issued_unix_seconds + 1;
        let first = driver.tick(&mut host, now).unwrap();
        if class == FailureClass::OwnershipUnresolved {
            assert_eq!(first, Step::OwnershipUnresolved("SettlementUnproven"));
            assert_eq!(host.aborts, 0);
            assert!(driver.tick(&mut host, now + 1).is_err());
        } else {
            assert!(
                matches!(first, Step::Sealed(ref record) if matches!(record.outcome, AttemptOutcome::ClassifiedFailure { class: found, .. } if found == class))
            );
            let next = driver.tick(&mut host, now + 1).unwrap();
            assert_eq!(
                next,
                if class == FailureClass::Health {
                    Step::Stopped(FailureClass::Health)
                } else {
                    Step::Stopped(class)
                }
            );
            assert_eq!(host.aborts, 1);
        }
        assert_eq!(host.launches, 1);
    }
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
        AttemptOutcome::ClassifiedFailure {
            class: FailureClass::Environment,
            code: "LaunchRefused".into()
        }
    );
    assert_eq!(host.aborts, 1);
    host.launch_fails = false;
    assert_eq!(
        driver.tick(&mut host, now + 1).unwrap(),
        Step::Stopped(FailureClass::Environment)
    );
    let (_other_dir, mut driver, _) = setup(4);
    assert_eq!(driver.tick(&mut host, now + 1).unwrap(), Step::Running(0));
    let Step::Sealed(record) = driver.tick(&mut host, now + 1 + 100).unwrap() else {
        panic!("timeout must seal")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "AttemptTimeout".into()
        }
    );
    assert_eq!(driver.tick(&mut host, now + 200).unwrap(), Step::Running(1));
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

/// A timeout, budget stop or cancellation no longer blocks the tick for the settlement window: the host is asked one
/// bounded step per tick, the attempt stays active and the tick reports it running, and the outcome is sealed only
/// when settlement is proven. An unproven settlement is an error, never a seal over a possibly running game.
#[test]
fn a_stop_waits_for_proven_settlement_one_step_per_tick_and_never_seals_over_an_unsettled_game() {
    use crate::autotune::research::steam_host::StopStep::{Pending, Settled, Unsettled};
    let (_dir, mut driver, intent) = setup(4);
    let mut host = fake();
    host.abort_steps = [Pending, Pending, Settled].into();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    // The attempt timed out: the stop begins and is still settling; nothing is sealed yet.
    assert_eq!(driver.tick(&mut host, now + 101).unwrap(), Step::Running(0));
    assert!(driver.abort_pending());
    assert_eq!(driver.tick(&mut host, now + 102).unwrap(), Step::Running(0));
    assert!(driver.abort_pending());
    // Proven settled: the outcome the stop began with is sealed.
    let Step::Sealed(record) = driver.tick(&mut host, now + 103).unwrap() else {
        panic!("a settled stop seals")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "AttemptTimeout".into()
        }
    );
    assert!(!driver.abort_pending());
    assert_eq!(
        host.launches, 1,
        "a stop in progress never starts another launch"
    );
    // Never proven: an error on that tick, nothing sealed, and the stop is still owed.
    let (_other, mut driver, intent) = setup(4);
    let mut host = fake();
    host.abort_steps = [Pending, Unsettled].into();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    assert_eq!(driver.tick(&mut host, now + 101).unwrap(), Step::Running(0));
    assert!(driver.tick(&mut host, now + 102).is_err());
    assert!(
        driver.abort_pending(),
        "an unsettled game keeps the stop pending"
    );
    // A cancellation that waits for settlement (an explicit shutdown) ticks through the window to the seal.
    let (_third, mut driver, intent) = setup(4);
    let mut host = fake();
    host.abort_steps = [Pending, Settled].into();
    let now = intent.issued_unix_seconds + 1;
    assert_eq!(driver.tick(&mut host, now).unwrap(), Step::Running(0));
    driver.cancel();
    let Step::Sealed(record) = driver
        .tick_cancelled_to_settlement(&mut host, now + 1)
        .unwrap()
    else {
        panic!("a cancelled attempt seals once settled")
    };
    assert_eq!(
        record.outcome,
        AttemptOutcome::Failed {
            code: "Cancelled".into()
        }
    );
}
