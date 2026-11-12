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
    current: u32,
    auto_finish: bool,
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
        script.current = slot.index;
        script.launches += 1;
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
    fn abort(&mut self) {
        self.0.lock().unwrap().aborts += 1;
    }
}

fn phase(coordinator: &ResearchCoordinator) -> Option<ResearchPhase> {
    coordinator
        .active
        .as_ref()
        .map(|active| active.lease.phase())
}

#[test]
fn resident_driver_runs_every_scheduled_attempt_once_and_completes() {
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
    let mut coordinator = ResearchCoordinator::new(
        Some(campaign.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );
    coordinator.set_study_dir(Some(study.path().to_path_buf()));
    let handle = script.clone();
    coordinator.set_host_factory(Some(Box::new(move || Box::new(Fake(handle.clone())))));
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
    coordinator.drive_attempts(200);
    assert_eq!(script.lock().unwrap().launches, 8);
    assert!(coordinator.active.is_none());
    assert_eq!(
        coordinator.terminal.as_ref().unwrap().phase,
        ResearchPhase::Completed
    );
    let report = send(&mut coordinator, ResearchRequest::Report {}, 201);
    assert!(
        report.contains("\"code\":\"Report\"") && report.contains("arms"),
        "{report}"
    );
    let verified = send(&mut coordinator, ResearchRequest::Verify {}, 202);
    assert!(verified.contains("ReportVerified"), "{verified}");
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
    coordinator.set_host_factory(Some(Box::new(move || Box::new(Fake(handle.clone())))));
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
}
