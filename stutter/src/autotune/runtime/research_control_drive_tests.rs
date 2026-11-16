use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use super::*;
use crate::autotune::research::{
    AttemptSlot, ProtonSource, ResearchTool,
    dispatch_context::{ArmSpec, DispatchContextV1},
    driver::{AttemptHost, Progress},
    report::AttemptEvidenceV1,
};

#[derive(Default)]
struct Script {
    safe: bool,
    polls: VecDeque<Progress>,
    launches: u32,
    aborts: u32,
    abort_fails: bool,
    launch_fails: bool,
    launch_unresolved: bool,
    current: u32,
    auto_finish: bool,
    launch_fence_dir: Option<std::path::PathBuf>,
}

struct Fake(Arc<Mutex<Script>>);

impl AttemptHost for Fake {
    fn safe_window(&mut self) -> bool {
        self.0.lock().unwrap().safe
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
        let mut script = self.0.lock().unwrap();
        if let Some(dir) = &script.launch_fence_dir {
            let mut launches = 0;
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().starts_with("event-") {
                    let value: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(entry.path())?)?;
                    if value.get("to").and_then(|v| v.as_str()) == Some("launching") {
                        launches += 1;
                    }
                }
            }
            anyhow::ensure!(launches == script.launches + 1, "launch fence missing");
        }
        script.current = slot.index;
        script.launches += 1;
        if script.launch_unresolved {
            return Err(crate::autotune::research::driver::LaunchUnresolved(
                "lost acknowledgement",
            )
            .into());
        }
        anyhow::ensure!(!script.launch_fails, "launch failed");
        Ok(())
    }
    fn poll(&mut self) -> Progress {
        {
            let script = self.0.lock().unwrap();
            if script.auto_finish {
                let slot_index = script.current;
                return Progress::Finished {
                    evidence: AttemptEvidenceV1 {
                        schema_version: 1,
                        slot_index,
                        average_fps: 60.0 + f64::from(slot_index % 3),
                        native_completion: None,
                    },
                };
            }
        }
        self.0
            .lock()
            .unwrap()
            .polls
            .pop_front()
            .unwrap_or(Progress::Running)
    }
    fn abort(&mut self) -> anyhow::Result<()> {
        let mut script = self.0.lock().unwrap();
        script.aborts += 1;
        anyhow::ensure!(!script.abort_fails, "settlement unverified");
        Ok(())
    }
}

fn phase(coordinator: &ResearchCoordinator) -> Option<ResearchPhase> {
    coordinator
        .active
        .as_ref()
        .map(|active| active.lease.phase())
}

fn finished_schedule_is_blocked_until_verified(report_conflict: bool) {
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
    intent.max_attempts = 8;
    intent.max_attempt_seconds = 50;
    let script = Arc::new(Mutex::new(Script::default()));
    script.lock().unwrap().launch_fence_dir = Some(campaign.path().to_path_buf());
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

    // No safe window: nothing launches.
    coordinator.drive_attempts(101);
    assert_eq!(script.lock().unwrap().launches, 0);
    assert_eq!(phase(&coordinator), Some(ResearchPhase::AwaitSafeWindow));

    script.lock().unwrap().safe = true;
    script.lock().unwrap().auto_finish = true;
    for step in 0..8u64 {
        coordinator.drive_attempts(102 + step * 3);
        assert_eq!(phase(&coordinator), Some(ResearchPhase::Launching));
        coordinator.drive_attempts(103 + step * 3);
        assert_eq!(phase(&coordinator), Some(ResearchPhase::AwaitSafeWindow));
    }
    if report_conflict {
        let dir = coordinator.open_campaign_study_dir().unwrap();
        crate::autotune::research::report_store::write_new(&dir, "report.json", b"conflict")
            .unwrap();
    }
    coordinator.drive_attempts(200);
    assert_eq!(script.lock().unwrap().launches, 8);
    assert!(coordinator.active.is_none());
    assert_eq!(
        coordinator.terminal.as_ref().unwrap().phase,
        ResearchPhase::Blocked
    );
    assert_eq!(
        coordinator.terminal.as_ref().unwrap().code,
        if report_conflict {
            "ReportPersistenceFailed"
        } else {
            "IncompleteProvenance"
        }
    );
    if report_conflict {
        return;
    }
    let report = send(&mut coordinator, ResearchRequest::Report {}, 201);
    assert!(
        report.contains("\"code\":\"Report\"") && report.contains("arms"),
        "{report}"
    );
    let verified = send(&mut coordinator, ResearchRequest::Verify {}, 202);
    assert!(
        verified.contains("IncompleteProvenance")
            && verified.contains("\"reconstruction\":\"passed\""),
        "{verified}"
    );
}

#[test]
fn resident_driver_does_not_complete_on_unqualified_evidence() {
    finished_schedule_is_blocked_until_verified(false);
}

#[test]
fn report_write_failure_does_not_leave_a_completed_lease() {
    finished_schedule_is_blocked_until_verified(true);
}

#[test]
fn active_budget_exhaustion_preserves_a_partial_report() {
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
    intent.max_active_seconds = 3;
    intent.max_attempt_seconds = 3;
    intent.max_owned_pressure_seconds_per_study = 3;
    let script = Arc::new(Mutex::new(Script {
        safe: true,
        ..Default::default()
    }));
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
    coordinator.drive_attempts(103);
    coordinator.drive_attempts(104);
    assert_eq!(
        coordinator.terminal.as_ref().unwrap().code,
        "ActiveBudgetExceeded"
    );
    let report = send(&mut coordinator, ResearchRequest::Report {}, 105);
    assert!(report.contains("\"attempts_sealed\":1"), "{report}");
    assert_eq!(script.lock().unwrap().launches, 1);
}

#[test]
fn cancelling_a_driven_attempt_aborts_the_host_and_settles_aborted() {
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
    let mut coordinator = ResearchCoordinator::new(
        Some(campaign.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );
    coordinator.set_study_dir(Some(study.path().to_path_buf()));
    let handle = script.clone();
    coordinator.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
    send(
        &mut coordinator,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    coordinator.drive_attempts(101);
    assert_eq!(phase(&coordinator), Some(ResearchPhase::Launching));
    let reply = send(&mut coordinator, ResearchRequest::Cancel {}, 102);
    assert!(reply.contains("CancellationRequested"), "{reply}");
    coordinator.drive_attempts(103);
    coordinator.drive_attempts(104);
    assert_eq!(script.lock().unwrap().aborts, 1);
    assert_eq!(script.lock().unwrap().launches, 1);
    assert_eq!(
        coordinator.terminal.as_ref().unwrap().phase,
        ResearchPhase::Aborted
    );
    let report = send(&mut coordinator, ResearchRequest::Report {}, 105);
    assert!(report.contains("\"attempts_sealed\":1"), "{report}");
}

#[test]
fn failed_settlement_retains_the_campaign_and_mutation_slot() {
    for mode in 0..3 {
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
            abort_fails: mode != 2,
            launch_fails: mode == 1,
            auto_finish: mode == 2,
            ..Default::default()
        }));
        let mut coordinator =
            ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
        coordinator.set_study_dir(Some(study.path().to_path_buf()));
        let handle = script.clone();
        coordinator.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
        send(
            &mut coordinator,
            ResearchRequest::Submit {
                intent: Box::new(intent),
            },
            100,
        );
        coordinator.drive_attempts(101);
        if mode == 0 {
            send(&mut coordinator, ResearchRequest::Cancel {}, 102);
            coordinator.drive_attempts(103);
        } else if mode == 2 {
            std::fs::write(campaign.path().join("intent.json"), b"tampered").unwrap();
            coordinator.drive_attempts(103);
        }
        assert!(coordinator.owns_action_slot());
        assert_eq!(coordinator.storage_blocker, Some("UnsafeToResume"));
        assert!(coordinator.terminal.is_none());
        assert!(coordinator.driver.is_some());
        assert!(!coordinator.drive_due());
        assert!(send(&mut coordinator, ResearchRequest::Cancel {}, 104).contains("UnsafeToResume"));
        coordinator.on_deadline(4_000);
        assert!(coordinator.owns_action_slot());
        assert_eq!(script.lock().unwrap().launches, 1);
    }
}

#[test]
fn controller_shutdown_stops_the_attempt_or_reports_unverified_settlement() {
    for abort_fails in [false, true] {
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
            abort_fails,
            ..Default::default()
        }));
        let mut coordinator =
            ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
        coordinator.set_study_dir(Some(study.path().to_path_buf()));
        let handle = script.clone();
        coordinator.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
        send(
            &mut coordinator,
            ResearchRequest::Submit {
                intent: Box::new(intent),
            },
            100,
        );
        coordinator.drive_attempts(101);
        let result = coordinator.shutdown_driver(102);
        assert_eq!(result.is_err(), abort_fails);
        assert_eq!(script.lock().unwrap().aborts, 1);
        if abort_fails {
            assert!(coordinator.owns_action_slot());
            assert!(coordinator.terminal.is_none());
        } else {
            assert!(!coordinator.owns_action_slot());
            assert_eq!(
                coordinator.terminal.as_ref().unwrap().phase,
                ResearchPhase::Aborted
            );
        }
    }
}

#[test]
fn an_unresolved_launch_never_reaches_abort_and_keeps_ownership() {
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
        launch_unresolved: true,
        ..Default::default()
    }));
    let mut coordinator =
        ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
    coordinator.set_study_dir(Some(study.path().to_path_buf()));
    let handle = script.clone();
    coordinator.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
    send(
        &mut coordinator,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    coordinator.drive_attempts(101);
    // The request may still start: no abort, no seal, no retry, ownership retained.
    assert_eq!(script.lock().unwrap().aborts, 0);
    assert_eq!(script.lock().unwrap().launches, 1);
    assert!(coordinator.launch_unresolved);
    assert!(coordinator.owns_action_slot());
    assert!(coordinator.terminal.is_none());
    // Later ticks and a cancel request still never abort, relaunch or settle it.
    coordinator.drive_attempts(102);
    send(&mut coordinator, ResearchRequest::Cancel {}, 103);
    coordinator.drive_attempts(104);
    coordinator.on_deadline(4_000);
    assert_eq!(script.lock().unwrap().aborts, 0);
    assert_eq!(script.lock().unwrap().launches, 1);
    assert!(coordinator.owns_action_slot());
    // Shutdown cannot settle it either, and does not abort.
    assert!(coordinator.shutdown_driver(4_001).is_err());
    assert_eq!(script.lock().unwrap().aborts, 0);
}

#[test]
fn a_host_that_cannot_be_built_leaves_no_durable_study_state() {
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
    let mut coordinator =
        ResearchCoordinator::new(Some(campaign.path().to_path_buf()), Some(facts), "Ready");
    coordinator.set_study_dir(Some(study.path().to_path_buf()));
    let seen = Arc::new(Mutex::new(None));
    let record = seen.clone();
    coordinator.set_host_factory(Some(Box::new(move |intent| {
        // The factory receives the frozen intent it must qualify the host against.
        *record.lock().unwrap() = Some(intent.build_id);
        anyhow::bail!("BlockedHostIdentity")
    })));
    let build_id = intent.build_id;
    send(
        &mut coordinator,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    assert_eq!(*seen.lock().unwrap(), Some(build_id));
    assert!(coordinator.driver.is_none());
    // No campaign directory, journal or intent file was created before the failure.
    assert_eq!(std::fs::read_dir(study.path()).unwrap().count(), 0);
}

#[test]
fn the_unqualified_host_stays_unless_live_launch_is_enabled_with_every_source() {
    use crate::daemon::config::autotune::DaemonAutotuneConfig;
    let intent = fixture().0;
    let safe = |config: &DaemonAutotuneConfig| {
        let mut host = select_host_factory(config)(&intent).expect("must build the fallback");
        host.safe_window()
    };
    // The default configuration never launches.
    let mut config = DaemonAutotuneConfig::default();
    assert!(!safe(&config));
    // Opting in without the sources keeps the unqualified host rather than a half-built one.
    config.research_live_launch = true;
    assert!(!safe(&config));
    // Opting in with the sources selects the composed host, which refuses to build here because
    // the sources do not exist, so a campaign is blocked instead of launching.
    config.research_study_dir = Some("/definitely/not/a/study".into());
    config.research_steamapps_dir = Some("/definitely/not/steamapps".into());
    config.research_proton_manifest = Some("/definitely/not/manifest.json".into());
    assert!(select_host_factory(&config)(&intent).is_err());
}

#[path = "research_control_launched_recovery_tests.rs"]
mod launched_recovery;

#[path = "research_funnel_tests.rs"]
mod funnel_mode;
