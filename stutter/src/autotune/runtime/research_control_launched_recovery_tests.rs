use super::*;

#[test]
fn a_restarted_coordinator_retires_only_a_settled_interrupted_attempt_and_blocks_the_campaign() {
    use std::os::fd::AsFd;

    use crate::autotune::research::{
        ResearchJournal,
        driver::{ClaimSettlementGuard, ClaimSettlementVerifier, build_schedule},
        launch_record::{record_outcome_with_invocation, record_phase},
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
                    unit_invocation: Box::new(|_| {
                        Ok(Some("0123456789abcdef0123456789abcdef".into()))
                    }),
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
    record_outcome_with_invocation(
        &attempt,
        &unit,
        &LaunchOutcome::Running,
        Some("0123456789abcdef0123456789abcdef"),
    )
    .unwrap();

    // Leave a durable profile snapshot as if the controller died after editing the settings.
    let study_root = std::fs::canonicalize(study.path()).unwrap();
    let settings =
        crate::autotune::research::session_host::arm_prefix_paths(&study_root, ProtonSource::Ge).2;
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let original = br#"{"version":1,"data":[{"group_name":"/graphics/raytracing","options":[{"name":"RayTracing","type":"bool","value":true},{"name":"RayTracedReflections","type":"bool","value":true},{"name":"RayTracedSunShadows","type":"bool","value":true},{"name":"RayTracedLocalShadows","type":"bool","value":true},{"name":"RayTracedPathTracing","type":"bool","value":false}]}]}"#;
    std::fs::write(&settings, original).unwrap();
    let file = crate::autotune::research::profile_applier::StudySettingsFile::new(
        &study_root,
        settings.clone(),
        crate::syscall::geteuid(),
    )
    .unwrap();
    crate::autotune::research::profile_applier::apply(
        &file,
        &crate::autotune::research::profile_applier::ray_tracing_off(),
        false,
    )
    .unwrap();
    let edited = std::fs::read(&settings).unwrap();
    assert_ne!(edited, original);

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
    assert_eq!(
        std::fs::read(&settings).unwrap(),
        edited,
        "unsettled parent must prevent profile restoration"
    );
    assert!(crate::autotune::research::profile_applier::pending_snapshot(&settings));
    unsafe_restart.finish_recovery(result);
    assert!(unsafe_restart.owns_action_slot());
    drop(unsafe_restart);

    // Every witness agreeing retires the attempt and ends the campaign blocked, never relaunched.
    let mut settled_restart = restart(witnesses(true));
    let result = settled_restart.prepare_recovery(103).unwrap().run();
    assert!(result.is_ok(), "{:?}", result.as_ref().err());
    assert_eq!(std::fs::read(&settings).unwrap(), original);
    assert!(!crate::autotune::research::profile_applier::pending_snapshot(&settings));
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

mod funnel_campaign {
    use std::os::fd::AsFd;

    use super::*;
    use crate::autotune::research::{
        ResearchIntentV1,
        driver::{ClaimSettlementGuard, ClaimSettlementVerifier},
        launch_record::{record_outcome_with_invocation, record_phase},
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

    fn witnesses(process: bool) -> crate::autotune::runtime::research_recovery::RecoveryFactory {
        std::sync::Arc::new(move |_| {
            Ok(RecoveryWitnesses {
                unit_state: Box::new(|_| Ok(UnitState::Finished)),
                unit_invocation: Box::new(|_| Ok(Some("0123456789abcdef0123456789abcdef".into()))),
                process_tree_settled: Box::new(move |_, _| Ok(process)),
                outstanding_worker_tokens: Box::new(|| Ok(0)),
                transactions_restored: Box::new(|| Ok(true)),
                claim_verifier: Box::new(NoClaims),
            })
        })
    }

    #[test]
    fn a_restarted_controller_settles_the_childs_interrupted_attempt_before_it_retires_the_campaign()
     {
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
        intent.max_attempts = 40;
        let boot = crate::telemetry::identity::read_boot_id().unwrap();
        intent.boot_id = boot.clone();
        facts.boot_id = boot;
        let digest = intent.digest().unwrap();

        // The funnel campaign launches the first child's first attempt; the controller then dies.
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
        first.set_funnel_builder(Some(super::super::funnel_mode::builder()));
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

        // The child's attempt directory (not the flat one) holds the launch records.
        let studies = std::fs::read_dir(study.path().join(&digest[..16]).join("studies"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(studies.len(), 1, "one child study has begun");
        let child_intent: ResearchIntentV1 =
            serde_json::from_slice(&std::fs::read(studies[0].join("intent.json")).unwrap())
                .unwrap();
        let unit = launch_unit_name(&child_intent.digest().unwrap(), 0).unwrap();
        let attempt = File::from(
            crate::syscall::open_dir_no_symlinks(&studies[0].join("dispatch").join("0000"))
                .unwrap(),
        );
        let _ = attempt.as_fd();
        record_phase(&attempt, LaunchPhase::Prepared, &unit).unwrap();
        record_phase(&attempt, LaunchPhase::Sent, &unit).unwrap();
        record_outcome_with_invocation(
            &attempt,
            &unit,
            &LaunchOutcome::Running,
            Some("0123456789abcdef0123456789abcdef"),
        )
        .unwrap();

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
        // The child's process tree is not provably settled: the campaign must stay unsafe.
        let mut unsafe_restart = restart(witnesses(false));
        let result = unsafe_restart.prepare_recovery(102).unwrap().run();
        assert!(
            result.is_err(),
            "a campaign whose child attempt is unsettled must not be retired"
        );
        unsafe_restart.finish_recovery(result);
        assert!(unsafe_restart.owns_action_slot());
        drop(unsafe_restart);

        // Every witness agreeing retires the child's attempt and ends the campaign blocked.
        let mut settled = restart(witnesses(true));
        let result = settled.prepare_recovery(103).unwrap().run();
        assert!(result.is_ok(), "{:?}", result.as_ref().err());
        settled.finish_recovery(result);
        assert!(!settled.owns_action_slot());
        assert_eq!(
            settled.terminal.as_ref().unwrap().phase,
            ResearchPhase::Blocked
        );
        let child_report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(studies[0].join("report.json")).unwrap())
                .unwrap();
        assert_eq!(child_report["attempts_sealed"].as_u64(), Some(1));
        let campaign_study = File::open(study.path().join(&digest[..16])).unwrap();
        let counts = crate::autotune::research::child_ledger::read_attempt_counts(
            &campaign_study,
            intent.uid,
            &intent,
        )
        .unwrap();
        assert_eq!(counts.values().copied().collect::<Vec<_>>(), vec![1]);
        let replay = crate::autotune::research::child_verify::verify_children(
            &campaign_study,
            intent.uid,
            &intent,
            &|_| std::path::PathBuf::from("/nonexistent"),
        )
        .unwrap();
        assert_eq!(replay.children.len(), 1);
        assert!(
            replay.children[0].reconstruction,
            "{:?}",
            replay.children[0].problem
        );
        assert!(
            !studies[0].join("budget.json").exists(),
            "unknown crash spend must not be recorded as measured"
        );
        let spent = crate::autotune::research::campaign_budget::spent(
            &File::open(study.path().join(&digest[..16]).join("studies")).unwrap(),
            intent.uid,
            u64::from(intent.max_active_seconds),
        )
        .unwrap();
        assert!(spent.active_seconds >= u64::from(intent.max_active_seconds));
        drop(settled);
        let mut again = restart(witnesses(true));
        let result = again.prepare_recovery(104).unwrap().run();
        assert!(
            result.is_ok(),
            "recovery must replay idempotently: {:?}",
            result.as_ref().err()
        );
        drop(result);
        drop(again);
        std::fs::write(studies[0].join("report.json"), b"{}").unwrap();
        let mut corrupted = restart(witnesses(true));
        assert!(corrupted.prepare_recovery(105).unwrap().run().is_err());
    }
    #[test]
    fn a_child_integrity_failure_is_reconstructed_after_the_controller_crashes() {
        use crate::autotune::research::driver::FailureClass;
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
        intent.max_attempts = 40;
        let boot = crate::telemetry::identity::read_boot_id().unwrap();
        intent.boot_id = boot.clone();
        facts.boot_id = boot;
        let mut script = Script {
            safe: true,
            ..Default::default()
        };
        script.polls.push_back(Progress::Failed {
            class: FailureClass::Integrity,
            code: "SameDiagnostic".into(),
        });
        let script = Arc::new(Mutex::new(script));
        let mut first = ResearchCoordinator::new(
            Some(campaign.path().to_path_buf()),
            Some(facts.clone()),
            "Ready",
        );
        first.set_study_dir(Some(study.path().to_path_buf()));
        let handle = script.clone();
        first.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
        first.set_funnel_builder(Some(super::super::funnel_mode::builder()));
        assert!(
            send(
                &mut first,
                ResearchRequest::Submit {
                    intent: Box::new(intent)
                },
                100
            )
            .contains("await_safe_window")
        );
        first.drive_attempts(101);
        first.drive_attempts(102);
        assert!(first.terminal.is_none());
        drop(first);
        let mut restarted =
            ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
        restarted.set_study_dir(Some(study.path().to_path_buf()));
        restarted.set_recovery_factory(Some(witnesses(true)));
        let result = restarted.prepare_recovery(103).unwrap().run();
        assert!(result.is_ok(), "{:?}", result.as_ref().err());
        restarted.finish_recovery(result);
        let terminal = restarted.terminal.as_ref().unwrap();
        assert_eq!(terminal.phase, ResearchPhase::Aborted);
        assert_eq!(terminal.code, "AbortedIntegrity");
        assert_eq!(script.lock().unwrap().launches, 1);
    }
}

mod funnel_waiting {
    use super::*;

    #[test]
    fn a_funnel_campaign_that_only_waited_for_a_safe_window_is_still_recoverable() {
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
        intent.max_attempts = 40;
        let boot = crate::telemetry::identity::read_boot_id().unwrap();
        intent.boot_id = boot.clone();
        facts.boot_id = boot;
        // No safe window: the first child begins (its directories exist) but never launches.
        let script = Arc::new(Mutex::new(Script::default()));
        let mut first = ResearchCoordinator::new(
            Some(campaign.path().to_path_buf()),
            Some(facts.clone()),
            "Ready",
        );
        first.set_study_dir(Some(study.path().to_path_buf()));
        let handle = script.clone();
        first.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
        first.set_funnel_builder(Some(super::super::funnel_mode::builder()));
        send(
            &mut first,
            ResearchRequest::Submit {
                intent: Box::new(intent.clone()),
            },
            100,
        );
        first.drive_attempts(101);
        assert_eq!(script.lock().unwrap().launches, 0);
        drop(first);
        let digest = intent.digest().unwrap();
        assert!(study.path().join(&digest[..16]).join("studies").exists());
        // A restarted controller finds no launch and records the interrupted child with zero attempts.
        let mut restarted =
            ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
        restarted.set_study_dir(Some(study.path().to_path_buf()));
        let result = restarted.prepare_recovery(102).unwrap().run();
        assert!(result.is_ok(), "{:?}", result.as_ref().err());
        let campaign_study = study.path().join(&digest[..16]);
        let child = std::fs::read_dir(campaign_study.join("studies"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(child.join("report.json")).unwrap()).unwrap();
        assert_eq!(report["attempts_sealed"].as_u64(), Some(0));
        let counts = crate::autotune::research::child_ledger::read_attempt_counts(
            &File::open(campaign_study).unwrap(),
            intent.uid,
            &intent,
        )
        .unwrap();
        assert_eq!(counts.values().copied().collect::<Vec<_>>(), vec![0]);
    }
}

#[test]
fn a_sealed_integrity_failure_reconstructs_its_abort_after_restart() {
    use crate::autotune::research::{
        driver::{ClaimSettlementGuard, ClaimSettlementVerifier, FailureClass},
        launch_ticket::ResearchLaunchTicketV1,
        launch_txn::UnitState,
        recovery_gather::RecoveryWitnesses,
    };
    struct NoClaims;
    impl ClaimSettlementVerifier for NoClaims {
        fn verify_process(
            &mut self,
            _: &File,
            _: &DispatchContextV1,
            _: &ResearchLaunchTicketV1,
        ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
            anyhow::bail!("no claim expected")
        }
        fn verify_worker(
            &mut self,
            _: &DispatchContextV1,
        ) -> anyhow::Result<Box<dyn ClaimSettlementGuard>> {
            anyhow::bail!("no claim expected")
        }
    }
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
    let boot = crate::telemetry::identity::read_boot_id().unwrap();
    intent.boot_id = boot.clone();
    facts.boot_id = boot;
    let mut script = Script {
        safe: true,
        ..Default::default()
    };
    script.polls.push_back(Progress::Failed {
        class: FailureClass::Integrity,
        code: "SameDiagnostic".into(),
    });
    let script = Arc::new(Mutex::new(script));
    let mut first = ResearchCoordinator::new(
        Some(campaign.path().to_path_buf()),
        Some(facts.clone()),
        "Ready",
    );
    first.set_study_dir(Some(study.path().to_path_buf()));
    let handle = script.clone();
    first.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
    assert!(
        send(
            &mut first,
            ResearchRequest::Submit {
                intent: Box::new(intent)
            },
            100
        )
        .contains("await_safe_window")
    );
    first.drive_attempts(101);
    first.drive_attempts(102);
    assert!(first.terminal.is_none());
    drop(first);
    let factory: crate::autotune::runtime::research_recovery::RecoveryFactory = Arc::new(|_| {
        Ok(RecoveryWitnesses {
            unit_state: Box::new(|_| Ok(UnitState::Finished)),
            unit_invocation: Box::new(|_| Ok(None)),
            process_tree_settled: Box::new(|_, _| Ok(true)),
            outstanding_worker_tokens: Box::new(|| Ok(0)),
            transactions_restored: Box::new(|| Ok(true)),
            claim_verifier: Box::new(NoClaims),
        })
    });
    let mut restarted =
        ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
    restarted.set_study_dir(Some(study.path().to_path_buf()));
    restarted.set_recovery_factory(Some(factory));
    let result = restarted.prepare_recovery(103).unwrap().run();
    assert!(result.is_ok(), "{:?}", result.as_ref().err());
    restarted.finish_recovery(result);
    let terminal = restarted.terminal.as_ref().unwrap();
    assert_eq!(terminal.phase, ResearchPhase::Aborted);
    assert_eq!(terminal.code, "AbortedIntegrity");
    assert_eq!(script.lock().unwrap().launches, 1);
}
