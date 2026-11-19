use super::*;

#[test]
fn recovery_checks_attempt_journal_even_when_lease_never_recorded_launching() {
    for evidence in 0..5 {
        let campaign = tempfile::tempdir().unwrap();
        let study = tempfile::tempdir().unwrap();
        for dir in [&campaign, &study] {
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let metadata = std::fs::metadata(study.path()).unwrap();
        let root = crate::autotune::research::StudyRootIdentity {
            device: std::os::unix::fs::MetadataExt::dev(&metadata),
            inode: std::os::unix::fs::MetadataExt::ino(&metadata),
        };
        let (mut intent, mut facts) = fixture();
        intent.study_root = Some(root);
        facts.study_root = Some(root);
        let digest = intent.digest().unwrap();
        let mut first = ResearchCoordinator::new(
            Some(campaign.path().to_path_buf()),
            Some(facts.clone()),
            "Ready",
        );
        first.set_study_dir(Some(study.path().to_path_buf()));
        assert!(
            send(
                &mut first,
                ResearchRequest::Submit {
                    intent: Box::new(intent.clone())
                },
                100
            )
            .contains("await_safe_window")
        );
        let child = study.path().join(&digest[..16]);
        std::fs::create_dir(&child).unwrap();
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700)).unwrap();
        let journal = child.join("journal");
        std::fs::create_dir(&journal).unwrap();
        std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(0o700)).unwrap();
        let dispatch = child.join("dispatch");
        std::fs::create_dir(&dispatch).unwrap();
        std::fs::set_permissions(&dispatch, std::fs::Permissions::from_mode(0o700)).unwrap();
        let intent_path = child.join("intent.json");
        std::fs::write(&intent_path, serde_json::to_vec(&intent).unwrap()).unwrap();
        std::fs::set_permissions(&intent_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        if evidence == 1 {
            std::fs::write(journal.join("0000.started"), b"interrupted").unwrap();
        } else if evidence == 2 {
            std::fs::create_dir(dispatch.join("0000")).unwrap();
        } else if evidence == 3 {
            std::fs::remove_dir(&dispatch).unwrap();
        }
        drop(first);
        let mut restarted =
            ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
        restarted.set_study_dir(Some(study.path().to_path_buf()));
        let result = restarted.prepare_recovery(101).unwrap().run();
        assert_eq!(result.is_err(), (1..=3).contains(&evidence));
        let replacement = tempfile::tempdir().unwrap();
        if evidence == 4 {
            restarted.set_study_dir(Some(replacement.path().to_path_buf()));
        }
        restarted.finish_recovery(result);
        assert!(restarted.owns_action_slot());
        if evidence != 0 {
            assert!(
                send(&mut restarted, ResearchRequest::Cancel {}, 102).contains("UnsafeToResume")
            );
        }
        if evidence == 4 {
            restarted.set_study_dir(Some(study.path().to_path_buf()));
            let retry = restarted.prepare_recovery(103).unwrap();
            restarted.finish_recovery(retry.run());
            assert!(
                send(&mut restarted, ResearchRequest::Status {}, 103).contains("await_safe_window")
            );
        }
    }
}

#[test]
fn settled_prelaunch_campaign_releases_slot_even_when_live_steam_facts_are_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let mut first = ResearchCoordinator::new(Some(dir.path().to_path_buf()), Some(facts), "Ready");
    send(
        &mut first,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    send(&mut first, ResearchRequest::Cancel {}, 101);
    drop(first);

    let mut restarted =
        ResearchCoordinator::new(Some(dir.path().to_path_buf()), None, "BlockedSteamLogin");
    assert!(restarted.owns_action_slot());
    restarted.set_mutable_action_present(true);
    assert!(restarted.prepare_recovery(102).is_none());
    restarted.set_mutable_action_present(false);
    let job = restarted.prepare_recovery(102).unwrap();
    let result = job.run();
    restarted.set_mutable_action_present(true);
    restarted.finish_recovery(result);
    assert!(restarted.owns_action_slot());
    restarted.set_mutable_action_present(false);
    let retry = restarted.prepare_recovery(102).unwrap();
    restarted.finish_recovery(retry.run());
    assert!(!restarted.owns_action_slot());
    assert!(send(&mut restarted, ResearchRequest::Status {}, 102).contains("aborted"));
}

#[test]
fn absent_live_facts_never_resume_a_nonterminal_campaign() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let mut first =
        ResearchCoordinator::new(Some(dir.path().to_path_buf()), Some(facts.clone()), "Ready");
    send(
        &mut first,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    drop(first);

    let mut restarted =
        ResearchCoordinator::new(Some(dir.path().to_path_buf()), None, "BlockedSteamLogin");
    let job = restarted.prepare_recovery(101).unwrap();
    restarted.finish_recovery(job.run());
    assert!(restarted.owns_action_slot());
    assert!(restarted.prepare_recovery(102).is_none());
    restarted.set_host_context(Ok(facts));
    let job = restarted.prepare_recovery(103).unwrap();
    restarted.finish_recovery(job.run());
    assert!(restarted.owns_action_slot());
    assert!(send(&mut restarted, ResearchRequest::Status {}, 103).contains("await_safe_window"));
}

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
    lease.advance(ResearchPhase::Restoring, 102).unwrap();
    lease.advance(ResearchPhase::Aborted, 102).unwrap();
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
    let mut without_facts =
        ResearchCoordinator::new(Some(dir.path().to_path_buf()), None, "BlockedSteamLogin");
    let job = without_facts.prepare_recovery(103).unwrap();
    without_facts.finish_recovery(job.run());
    assert!(without_facts.owns_action_slot());
}

#[test]
fn a_recovered_completed_lease_stays_terminal_until_artifacts_are_verified() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let (intent, facts) = fixture();
    let mut changed_facts = facts.clone();
    changed_facts.boot_id = "later-boot".into();
    let mut lease = CampaignLease::create(
        crate::syscall::open_dir_no_symlinks(dir.path()).unwrap(),
        intent,
        facts,
        100,
        std::process::id(),
        1,
    )
    .unwrap();
    for phase in [
        ResearchPhase::Preflight,
        ResearchPhase::Prepared,
        ResearchPhase::AwaitSafeWindow,
        ResearchPhase::Restoring,
        ResearchPhase::Completed,
    ] {
        lease.advance(phase, 101).unwrap();
    }
    drop(lease);
    let mut restarted =
        ResearchCoordinator::new(Some(dir.path().to_path_buf()), None, "BlockedSteamLogin");
    let job = restarted.prepare_recovery(102).unwrap();
    restarted.finish_recovery(job.run());
    let terminal = restarted.terminal.as_ref().unwrap();
    assert_eq!(terminal.phase, ResearchPhase::Completed);
    assert_eq!(terminal.code, "CompletedNeedsVerification");
    assert!(restarted.owns_action_slot());
    assert!(restarted.finalization_due());
    restarted.drive_finalization(103);
    assert_eq!(
        restarted.terminal.as_ref().unwrap().code,
        "TerminalVerificationFailed"
    );
    assert!(restarted.owns_action_slot());
    drop(restarted);
    let mut historical =
        ResearchCoordinator::new(Some(dir.path().to_path_buf()), Some(changed_facts), "Ready");
    let job = historical.prepare_recovery(104).unwrap();
    historical.finish_recovery(job.run());
    assert_eq!(
        historical.terminal.as_ref().unwrap().phase,
        ResearchPhase::Completed
    );
    assert_eq!(
        historical.terminal.as_ref().unwrap().code,
        "CompletedNeedsVerification"
    );
}

#[test]
fn a_prepared_restoring_lease_is_scheduled_for_reverification_after_restart() {
    use crate::autotune::research::{
        ProtonSource, ResearchJournal, ResearchTool,
        driver::build_schedule,
        report_store::{build_from_dirs, write_new, write_report},
    };
    let lease_dir = tempfile::tempdir().unwrap();
    let study_root = tempfile::tempdir().unwrap();
    for dir in [&lease_dir, &study_root] {
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let meta = std::fs::metadata(study_root.path()).unwrap();
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
    let digest = intent.digest().unwrap();
    let campaign = study_root.path().join(&digest[..16]);
    for path in [
        &campaign,
        &campaign.join("journal"),
        &campaign.join("dispatch"),
        &campaign.join("evidence"),
    ] {
        std::fs::create_dir(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let campaign_file = File::open(&campaign).unwrap();
    write_new(
        &campaign_file,
        "intent.json",
        &serde_json::to_vec(&intent).unwrap(),
    )
    .unwrap();
    let journal = ResearchJournal::open(
        crate::syscall::open_dir_no_symlinks(&campaign.join("journal")).unwrap(),
        intent.uid,
        build_schedule(&intent).unwrap(),
    )
    .unwrap();
    let report = build_from_dirs(
        &intent,
        &journal,
        &File::open(campaign.join("evidence")).unwrap(),
    )
    .unwrap();
    drop(journal);
    write_report(&campaign_file, &report).unwrap();
    let report_bytes = std::fs::read(campaign.join("report.json")).unwrap();
    write_new(&campaign_file, "finalization-prepared.json", &serde_json::to_vec(&serde_json::json!({
        "schema": 1, "intent_sha256": digest, "report_sha256": stutter_core::hash::sha256_hex(&report_bytes),
        "target_phase": "blocked", "verdict": {},
    })).unwrap()).unwrap();
    let mut lease = CampaignLease::create(
        crate::syscall::open_dir_no_symlinks(lease_dir.path()).unwrap(),
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
        ResearchPhase::Restoring,
    ] {
        lease.advance(phase, 101).unwrap();
    }
    drop(lease);
    let mut restarted =
        ResearchCoordinator::new(Some(lease_dir.path().to_path_buf()), Some(facts), "Ready");
    restarted.set_study_dir(Some(study_root.path().to_path_buf()));
    let job = restarted.prepare_recovery(102).unwrap();
    restarted.finish_recovery(job.run());
    assert_eq!(
        restarted.active.as_ref().unwrap().lease.phase(),
        ResearchPhase::Restoring
    );
    assert!(restarted.finalization_due());
    assert!(restarted.owns_action_slot());
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
        Some(facts.clone()),
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
    restarted.set_host_context(Ok(facts));
    let retry = restarted.prepare_recovery(102).unwrap();
    restarted.finish_recovery(retry.run());
    assert!(send(&mut restarted, ResearchRequest::Status {}, 102).contains("await_safe_window"));
}

#[test]
fn prelaunch_recovery_restores_a_profile_or_retains_ownership_if_unproven() {
    use crate::autotune::research::{
        ProtonSource,
        profile_applier::{StudySettingsFile, apply, pending_edit, ray_tracing_off},
        session_host::arm_prefix_paths,
    };
    for tamper in [false, true] {
        let campaign = tempfile::tempdir().unwrap();
        let study = tempfile::tempdir().unwrap();
        for dir in [&campaign, &study] {
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let metadata = std::fs::metadata(study.path()).unwrap();
        let root = crate::autotune::research::StudyRootIdentity {
            device: std::os::unix::fs::MetadataExt::dev(&metadata),
            inode: std::os::unix::fs::MetadataExt::ino(&metadata),
        };
        let (mut intent, mut facts) = fixture();
        intent.study_root = Some(root);
        facts.study_root = Some(root);
        let digest = intent.digest().unwrap();
        let mut first = ResearchCoordinator::new(
            Some(campaign.path().to_path_buf()),
            Some(facts.clone()),
            "Ready",
        );
        first.set_study_dir(Some(study.path().to_path_buf()));
        assert!(
            send(
                &mut first,
                ResearchRequest::Submit {
                    intent: Box::new(intent.clone())
                },
                100
            )
            .contains("await_safe_window")
        );
        let child = study.path().join(&digest[..16]);
        std::fs::create_dir(&child).unwrap();
        std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["journal", "dispatch"] {
            let dir = child.join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let intent_path = child.join("intent.json");
        std::fs::write(&intent_path, serde_json::to_vec(&intent).unwrap()).unwrap();
        std::fs::set_permissions(intent_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let study_root = std::fs::canonicalize(study.path()).unwrap();
        let settings = arm_prefix_paths(&study_root, ProtonSource::Ge).2;
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        let original = br#"{"version":1,"data":[{"group_name":"/graphics/raytracing","options":[{"name":"RayTracing","type":"bool","value":true},{"name":"RayTracedReflections","type":"bool","value":true},{"name":"RayTracedSunShadows","type":"bool","value":true},{"name":"RayTracedLocalShadows","type":"bool","value":true},{"name":"RayTracedPathTracing","type":"bool","value":false}]}]}"#;
        std::fs::write(&settings, original).unwrap();
        let file = StudySettingsFile::new(&study_root, settings.clone(), crate::syscall::geteuid())
            .unwrap();
        apply(&file, &ray_tracing_off(), false).unwrap();
        if tamper {
            std::fs::write(&settings, b"external change").unwrap();
        }
        drop(first);
        let mut restarted =
            ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
        restarted.set_study_dir(Some(study.path().to_path_buf()));
        let result = restarted.prepare_recovery(101).unwrap().run();
        assert_eq!(result.is_err(), tamper);
        if tamper {
            assert_eq!(std::fs::read(&settings).unwrap(), b"external change");
            assert!(pending_edit(&settings));
        } else {
            assert_eq!(std::fs::read(&settings).unwrap(), original);
            assert!(!pending_edit(&settings));
        }
        restarted.finish_recovery(result);
        assert!(restarted.owns_action_slot());
    }
}
