use super::*;

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
                FailureClass::Health => assert_eq!(next, Step::Halted("BlockedGpuStability")),
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
            Step::Halted("BlockedGpuStability")
        } else {
            Step::Stopped(class)
        };
        assert_eq!(recovered.tick(&mut host, now + 2).unwrap(), expected);
        assert_eq!(host.launches, 1);
    }
}
