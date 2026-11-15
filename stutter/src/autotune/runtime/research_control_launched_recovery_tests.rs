use super::*;

#[test]
fn a_restarted_coordinator_retires_only_a_settled_interrupted_attempt_and_blocks_the_campaign() {
    use std::os::fd::AsFd;

    use crate::autotune::research::{
        ResearchJournal,
        driver::{ClaimSettlementGuard, ClaimSettlementVerifier, build_schedule},
        launch_record::{record_outcome, record_phase},
        launch_txn::{LaunchOutcome, LaunchPhase, UnitState},
        recovery_gather::RecoveryWitnesses,
        user_launcher::launch_unit_name,
    };
    struct NoClaims;
    impl ClaimSettlementVerifier for NoClaims {
        fn verify_process(
            &mut self,
            _: &File,
            _: &DispatchContextV1,
            _: &crate::autotune::research::launch_ticket::ResearchLaunchTicketV1,
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
    let witnesses =
        |process: bool| -> crate::autotune::runtime::research_recovery::RecoveryFactory {
            std::sync::Arc::new(move |_| {
                Ok(RecoveryWitnesses {
                    unit_state: Box::new(|_| Ok(UnitState::Finished)),
                    process_tree_settled: Box::new(move |_, _| Ok(process)),
                    outstanding_worker_tokens: Box::new(|| Ok(0)),
                    transactions_restored: Box::new(|| Ok(true)),
                    claim_verifier: Box::new(NoClaims),
                })
            })
        };

    let campaign = tempfile::tempdir().unwrap();
    let study = tempfile::tempdir().unwrap();
    for dir in [&campaign, &study] {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let meta = std::fs::metadata(study.path()).unwrap();
    let root = crate::autotune::research::StudyRootIdentity {
        device: std::os::unix::fs::MetadataExt::dev(&meta),
        inode: std::os::unix::fs::MetadataExt::ino(&meta),
    };
    let (mut intent, mut facts) = fixture();
    intent.study_root = Some(root);
    facts.study_root = Some(root);
    intent.tools.push(ResearchTool {
        source: ProtonSource::ValveStable,
        identity_sha256: "b".repeat(64),
    });
    facts.tools = intent.tools.clone();
    intent.max_attempts = 4;
    // Recovery compares the intent's boot with the running boot, so use the real one here.
    let boot = crate::telemetry::identity::read_boot_id().unwrap();
    intent.boot_id = boot.clone();
    facts.boot_id = boot;
    let digest = intent.digest().unwrap();

    // The resident campaign launches attempt 0 and the controller dies before it is sealed.
    let script = Arc::new(Mutex::new(Script {
        safe: true,
        ..Default::default()
    }));
    let mut first = ResearchCoordinator::new(
        Some(campaign.path().to_path_buf()),
        Some(facts.clone()),
        "Ready",
    );
    first.set_study_dir(Some(study.path().to_path_buf()));
    let handle = script.clone();
    first.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
    send(
        &mut first,
        ResearchRequest::Submit {
            intent: Box::new(intent.clone()),
        },
        100,
    );
    first.drive_attempts(101);
    assert_eq!(script.lock().unwrap().launches, 1);
    drop(first);

    // The transient launch durably recorded a sent and reconciled request.
    let unit = launch_unit_name(&digest, 0).unwrap();
    let dispatch = File::from(
        crate::syscall::open_dir_no_symlinks(&study.path().join(&digest[..16]).join("dispatch"))
            .unwrap(),
    );
    let attempt =
        File::from(crate::syscall::openat_dir_nofollow(dispatch.as_fd(), "0000").unwrap());
    record_phase(&attempt, LaunchPhase::Prepared, &unit).unwrap();
    record_phase(&attempt, LaunchPhase::Sent, &unit).unwrap();
    record_outcome(&attempt, &unit, &LaunchOutcome::Running).unwrap();

    let restart = |factory: crate::autotune::runtime::research_recovery::RecoveryFactory| {
        let mut coordinator = ResearchCoordinator::new(
            Some(campaign.path().to_path_buf()),
            Some(facts.clone()),
            "Ready",
        );
        coordinator.set_study_dir(Some(study.path().to_path_buf()));
        coordinator.set_recovery_factory(Some(factory));
        coordinator
    };
    // A process tree that is not provably settled keeps the campaign unsafe and unsealed.
    let mut unsafe_restart = restart(witnesses(false));
    let result = unsafe_restart.prepare_recovery(102).unwrap().run();
    assert!(result.is_err());
    unsafe_restart.finish_recovery(result);
    assert!(unsafe_restart.owns_action_slot());
    drop(unsafe_restart);

    // Every witness agreeing retires the attempt and ends the campaign blocked, never relaunched.
    let mut settled_restart = restart(witnesses(true));
    let result = settled_restart.prepare_recovery(103).unwrap().run();
    assert!(result.is_ok(), "{:?}", result.as_ref().err());
    settled_restart.finish_recovery(result);
    assert!(!settled_restart.owns_action_slot());
    assert_eq!(
        settled_restart.terminal.as_ref().unwrap().phase,
        ResearchPhase::Blocked
    );
    assert_eq!(script.lock().unwrap().launches, 1, "nothing was relaunched");
    drop(settled_restart);

    // The interrupted attempt is sealed failed in the journal, and no other attempt was started.
    let journal = ResearchJournal::open(
        crate::syscall::open_dir_no_symlinks(&study.path().join(&digest[..16]).join("journal"))
            .unwrap(),
        crate::syscall::geteuid(),
        build_schedule(&intent).unwrap(),
    )
    .unwrap();
    let sealed = journal.verify().unwrap();
    assert_eq!(sealed.len(), 1);
    assert!(matches!(
        &sealed[0].outcome,
        crate::autotune::research::AttemptOutcome::Failed { code } if code == "InterruptedBeforeDispatch"
    ));
}
