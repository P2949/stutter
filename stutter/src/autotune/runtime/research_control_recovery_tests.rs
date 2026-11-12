use super::*;

#[test]
fn restart_recovers_only_verified_prelaunch_boundaries_and_reserves_the_slot() {
    for paused in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let (intent, facts) = fixture();
        let mut first = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            Some(facts.clone()),
            "BlockedHostIdentity",
        );
        send(
            &mut first,
            ResearchRequest::Submit {
                intent: Box::new(intent),
            },
            100,
        );
        if paused {
            send(&mut first, ResearchRequest::Pause {}, 101);
        }
        drop(first);

        let mut restarted = ResearchCoordinator::new(
            Some(dir.path().to_path_buf()),
            Some(facts),
            "BlockedHostIdentity",
        );
        assert!(restarted.owns_action_slot());
        let job = restarted.prepare_recovery(102).unwrap();
        assert!(!restarted.host_refresh_complete());
        assert!(restarted.prepare_recovery(102).is_none());
        restarted.finish_recovery(job.run());
        assert!(restarted.host_refresh_complete());
        let status = send(&mut restarted, ResearchRequest::Status {}, 102);
        assert!(status.contains(if paused {
            "paused"
        } else {
            "await_safe_window"
        }));
        assert!(restarted.owns_action_slot());
        assert!(send(&mut restarted, ResearchRequest::Cancel {}, 103).contains("aborted"));
        assert!(!restarted.owns_action_slot());
    }
}

#[test]
fn restart_refuses_changed_identity_and_any_unverified_launch_history() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let mut first = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts.clone()),
        "BlockedHostIdentity",
    );
    send(
        &mut first,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    drop(first);
    let mut changed = facts.clone();
    changed.source_sha256 = "9".repeat(64);
    let mut restarted = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(changed),
        "BlockedHostIdentity",
    );
    let job = restarted.prepare_recovery(101).unwrap();
    restarted.finish_recovery(job.run());
    assert!(send(&mut restarted, ResearchRequest::Status {}, 101).contains("UnsafeToResume"));
    assert!(restarted.owns_action_slot());
    assert!(restarted.prepare_recovery(102).is_none());
    drop(restarted);

    let mut lease = CampaignLease::recover(
        crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
        &facts,
        102,
    )
    .unwrap();
    for phase in [
        ResearchPhase::Launching,
        ResearchPhase::Attesting,
        ResearchPhase::Warmup,
        ResearchPhase::Measuring,
        ResearchPhase::Sealing,
        ResearchPhase::Analysing,
        ResearchPhase::NextArm,
        ResearchPhase::AwaitSafeWindow,
    ] {
        lease.advance(phase, 102).unwrap();
    }
    drop(lease);
    let count = std::fs::read_dir(dir.path()).unwrap().count();
    let mut restarted = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );
    let job = restarted.prepare_recovery(103).unwrap();
    restarted.finish_recovery(job.run());
    assert!(send(&mut restarted, ResearchRequest::Status {}, 103).contains("UnsafeToResume"));
    assert!(restarted.owns_action_slot());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), count);
}

#[test]
fn restart_waits_for_existing_actions_and_expires_a_safe_boundary() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let mut first = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts.clone()),
        "BlockedHostIdentity",
    );
    send(
        &mut first,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    drop(first);
    let mut restarted = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );
    restarted.set_mutable_action_present(true);
    assert!(restarted.prepare_recovery(3_610).is_none());
    restarted.set_mutable_action_present(false);
    let job = restarted.prepare_recovery(3_610).unwrap();
    restarted.finish_recovery(job.run());
    restarted.on_deadline(3_610);
    assert!(send(&mut restarted, ResearchRequest::Status {}, 3_610).contains("DeadlineExpired"));
    assert!(!restarted.owns_action_slot());
}

#[tokio::test]
async fn resident_schedules_recovery_without_driving_it_from_the_client() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let mut lease = CampaignLease::create(
        crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
        intent,
        facts.clone(),
        100,
        std::process::id(),
        1,
    )
    .unwrap();
    for phase in [
        ResearchPhase::Preflight,
        ResearchPhase::Prepared,
        ResearchPhase::AwaitSafeWindow,
    ] {
        lease.advance(phase, 100).unwrap();
    }
    drop(lease);
    let mut coordinator = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );
    let runtime = crate::autotune::runtime::AutotuneRuntime::new(
        crate::autotune::runtime::AutotuneRuntimeConfig::observe(None, None, None),
    );
    let (sender, mut receiver) = mpsc::channel(1);
    crate::autotune::runtime::session::schedule_research_recovery(
        &mut coordinator,
        &runtime,
        101,
        &sender,
    );
    assert!(coordinator.owns_action_slot());
    let result = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    coordinator.finish_recovery(result);
    assert_eq!(
        coordinator.active.as_ref().unwrap().lease.phase(),
        ResearchPhase::AwaitSafeWindow
    );
    assert!(!runtime.has_active_experiment());
}

#[test]
fn changed_context_discards_recovery_without_mutating_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let mut first = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts.clone()),
        "BlockedHostIdentity",
    );
    send(
        &mut first,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    drop(first);
    let count = std::fs::read_dir(dir.path()).unwrap().count();
    let mut restarted = ResearchCoordinator::new(
        Some(dir.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );
    let result = restarted.prepare_recovery(101).unwrap().run();
    restarted.set_host_context(Err(crate::autotune::research::host::HostFactsError {
        code: "BlockedSteamClientIdentity",
    }));
    restarted.finish_recovery(result);
    assert!(restarted.owns_action_slot());
    assert!(send(&mut restarted, ResearchRequest::Status {}, 101).contains("UnsafeToResume"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), count);
}
