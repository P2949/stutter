use super::*;

#[test]
fn a_prelaunch_health_halt_survives_restart_in_the_lease() {
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
    let script = Arc::new(Mutex::new(Script {
        safe: true,
        health_blocked: true,
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
    let reply = send(
        &mut first,
        ResearchRequest::Submit {
            intent: Box::new(intent.clone()),
        },
        100,
    );
    assert!(reply.contains("await_safe_window"), "{reply}");
    first.drive_attempts(101);
    assert_eq!(first.terminal.as_ref().unwrap().code, "BlockedGpuStability");
    assert_eq!(script.lock().unwrap().launches, 0);
    drop(first);
    let inspection =
        crate::autotune::research::lease_inspect::inspect(campaign.path(), intent.uid).unwrap();
    assert_eq!(
        inspection.block_reason,
        Some(CampaignBlockReason::GpuHealthIncident)
    );
    assert!(inspection.chain_valid && inspection.transitions_valid);
    let mut restarted =
        ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
    restarted.set_study_dir(Some(study.path().to_path_buf()));
    let job = restarted.prepare_recovery(102).unwrap();
    restarted.finish_recovery(job.run());
    assert_eq!(
        restarted.terminal.as_ref().unwrap().code,
        "BlockedGpuStability"
    );
}

#[test]
fn a_sealed_urgent_failure_survives_interruption_before_the_next_tick() {
    use crate::autotune::research::driver::{FailureClass, Progress};
    for (class, interruption, report_conflict) in [FailureClass::Integrity, FailureClass::Health]
        .into_iter()
        .flat_map(|class| {
            [
                (0, false),
                (1, false),
                (2, false),
                (0, true),
                (1, true),
                (2, true),
            ]
            .map(move |(interruption, conflict)| (class, interruption, conflict))
        })
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
        facts.study_root = Some(root);
        intent.study_root = Some(root);
        intent.tools.push(ResearchTool {
            source: ProtonSource::ValveStable,
            identity_sha256: "b".repeat(64),
        });
        facts.tools = intent.tools.clone();
        intent.max_attempts = 4;
        let script = Arc::new(Mutex::new(Script {
            safe: true,
            ..Default::default()
        }));
        script.lock().unwrap().polls.push_back(Progress::Failed {
            class,
            code: "SameDiagnostic".into(),
        });
        let mut coordinator = ResearchCoordinator::new(
            Some(campaign.path().to_path_buf()),
            Some(facts),
            "BlockedHostIdentity",
        );
        coordinator.set_study_dir(Some(study.path().to_path_buf()));
        let handle = script.clone();
        coordinator.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
        let reply = send(
            &mut coordinator,
            ResearchRequest::Submit {
                intent: Box::new(intent),
            },
            100,
        );
        assert!(reply.contains("await_safe_window"), "{reply}");
        coordinator.drive_attempts(101);
        coordinator.drive_attempts(102);
        if report_conflict {
            let dir = coordinator.open_campaign_study_dir().unwrap();
            crate::autotune::research::report_store::write_new(&dir, "report.json", b"conflict")
                .unwrap();
        }
        match interruption {
            0 => {
                send(&mut coordinator, ResearchRequest::Cancel {}, 103);
            }
            1 => coordinator.on_deadline(3610),
            _ => coordinator.shutdown_driver(103).unwrap(),
        }
        let terminal = coordinator.terminal.as_ref().unwrap_or_else(|| {
            panic!(
                "interruption={interruption} status={} launches={} blocker={:?}",
                coordinator.status(),
                script.lock().unwrap().launches,
                coordinator.storage_blocker
            )
        });
        assert_eq!(
            terminal.phase,
            if class == FailureClass::Integrity {
                ResearchPhase::Aborted
            } else {
                ResearchPhase::Blocked
            }
        );
        assert_eq!(terminal.code, class.terminal_code());
        assert_eq!(terminal.report_persisted, Some(!report_conflict));
        assert_eq!(script.lock().unwrap().launches, 1);
    }
}
