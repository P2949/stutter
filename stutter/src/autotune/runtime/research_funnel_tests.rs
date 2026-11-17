//! The resident controller driving a campaign as the child-study funnel (default off). A fake host
//! stands in for Steam; the lease, launch fence, cancel, shutdown and verification paths are the
//! controller's real ones.

use std::collections::BTreeMap;

use super::*;
use crate::autotune::research::{
    child_study::Profile,
    driver_attempts::DriverStudyAttempts,
    funnel_driver::{FunnelDriver, inputs_for_study},
    study_executor::{ProfileFailure, ProfileHook},
};

struct NoProfiles {
    rollback_unproven: bool,
}

impl ProfileHook for NoProfiles {
    fn apply(&mut self, _: Profile) -> Result<bool, ProfileFailure> {
        if self.rollback_unproven {
            Err(ProfileFailure::Unrestored("RollbackUnproven"))
        } else {
            Ok(false)
        }
    }
    fn recover(&mut self) -> Result<(), &'static str> {
        Ok(())
    }
    fn restore(&mut self) -> Result<(), &'static str> {
        Ok(())
    }
}

pub(in crate::autotune::runtime) fn builder() -> FunnelBuilder {
    builder_with_rollback(false)
}

fn builder_with_rollback(rollback_unproven: bool) -> FunnelBuilder {
    Box::new(move |intent, campaign, host| {
        let parent = intent.clone();
        let attempts = DriverStudyAttempts::new(
            campaign.try_clone()?,
            intent.clone(),
            host,
            // The controller's own time governs every tick; this clock is never consulted.
            Box::new(|| 0),
            Box::new(|| Ok(())),
            Box::new(|_| std::path::PathBuf::from("/nonexistent")),
            Box::new(move |study| inputs_for_study(&parent, study, &BTreeMap::new(), None)),
        );
        FunnelDriver::new(
            campaign,
            intent,
            Box::new(NoProfiles { rollback_unproven }),
            attempts,
            |_| None,
            Default::default(),
        )
    })
}

struct Setup {
    campaign: tempfile::TempDir,
    _study: tempfile::TempDir,
    coordinator: ResearchCoordinator,
    script: Arc<Mutex<Script>>,
}

fn setup(script: Script, max_attempts: u32, funnel: bool) -> Setup {
    setup_with_builder(script, max_attempts, funnel.then(builder))
}

fn setup_with_builder(script: Script, max_attempts: u32, funnel: Option<FunnelBuilder>) -> Setup {
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
    intent.max_attempts = max_attempts;
    intent.max_attempt_seconds = 50;
    let script = Arc::new(Mutex::new(script));
    let mut coordinator = ResearchCoordinator::new(
        Some(campaign.path().to_path_buf()),
        Some(facts),
        "BlockedHostIdentity",
    );
    coordinator.set_study_dir(Some(study.path().to_path_buf()));
    let handle = script.clone();
    coordinator.set_host_factory(Some(Box::new(move |_| Ok(Box::new(Fake(handle.clone()))))));
    if let Some(builder) = funnel {
        coordinator.set_funnel_builder(Some(builder));
    }
    let reply = send(
        &mut coordinator,
        ResearchRequest::Submit {
            intent: Box::new(intent),
        },
        100,
    );
    assert!(reply.contains("await_safe_window"), "{reply}");
    Setup {
        campaign,
        _study: study,
        coordinator,
        script,
    }
}

#[test]
fn failed_profile_rollback_retains_the_real_coordinator_lease_and_mutation_slot() {
    let mut s = setup_with_builder(
        Script {
            safe: true,
            ..Default::default()
        },
        40,
        Some(builder_with_rollback(true)),
    );
    s.coordinator.drive_attempts(101);
    assert_eq!(s.script.lock().unwrap().launches, 0);
    assert!(s.coordinator.owns_action_slot());
    assert!(s.coordinator.terminal.is_none());
    assert_eq!(s.coordinator.storage_blocker, Some("UnsafeToResume"));
    // An attempted shutdown cannot transform the latched failure into a settled owner.
    assert!(s.coordinator.shutdown_driver(102).is_err());
    assert!(s.coordinator.owns_action_slot());
}

#[test]
fn a_child_launch_with_a_lost_acknowledgement_keeps_the_resident_lease() {
    let mut s = setup(
        Script {
            safe: true,
            launch_unresolved: true,
            ..Default::default()
        },
        40,
        true,
    );
    s.coordinator.drive_attempts(101);
    let script = s.script.lock().unwrap();
    assert_eq!(script.launches, 1);
    assert_eq!(
        script.aborts, 0,
        "a no-op host abort cannot settle a possibly late launch"
    );
    drop(script);
    assert!(s.coordinator.owns_action_slot());
    assert!(s.coordinator.terminal.is_none());
    assert_eq!(s.coordinator.storage_blocker, Some("UnsafeToResume"));
    assert!(s.coordinator.shutdown_driver(102).is_err());
}

#[test]
fn a_funnel_campaign_walks_the_lease_per_attempt_through_the_real_fence_and_ends_blocked() {
    let mut s = setup(
        Script {
            safe: true,
            auto_finish: true,
            ..Default::default()
        },
        40,
        true,
    );
    // The fake host refuses a launch unless the lease durably recorded `launching` first.
    s.script.lock().unwrap().launch_fence_dir = Some(s.campaign.path().to_path_buf());
    let mut now = 101;
    while s.coordinator.terminal.is_none() && now < 400 {
        s.coordinator.drive_attempts(now);
        now += 1;
    }
    let terminal = s.coordinator.terminal.as_ref().expect("the funnel ends");
    assert_eq!(terminal.phase, ResearchPhase::Blocked);
    assert_eq!(terminal.code, "IncompleteProvenance");
    // Only the two screens could run: the fake host seals no native evidence, so nothing qualifies
    // and everything that depends on a screen is blocked. Two attempts each, two tools.
    assert_eq!(s.script.lock().unwrap().launches, 4);
    // The campaign's verification reports both children, reconstructed from their own records.
    let verified = send(&mut s.coordinator, ResearchRequest::Verify {}, now + 1);
    assert!(verified.contains("IncompleteProvenance"), "{verified}");
    let value: serde_json::Value = serde_json::from_str(verified.trim()).unwrap();
    let children = value["child_reports"]["children"].as_array().unwrap();
    assert_eq!(children.len(), 2, "{verified}");
    assert!(
        children.iter().all(|c| c["reconstruction"] == true),
        "{verified}"
    );
    // Every planned study has a recorded decision; none was qualified.
    let studies = value["child_studies"].as_array().unwrap();
    assert!(studies.iter().all(|d| !d["result"].is_null()), "{verified}");
    assert!(
        studies.iter().all(|d| d["result"] != "qualified"),
        "{verified}"
    );
}

#[test]
fn without_the_funnel_builder_a_campaign_runs_the_flat_schedule_unchanged() {
    let mut s = setup(
        Script {
            safe: true,
            auto_finish: true,
            ..Default::default()
        },
        4,
        false,
    );
    let mut now = 101;
    while s.coordinator.terminal.is_none() && now < 200 {
        s.coordinator.drive_attempts(now);
        now += 1;
    }
    assert_eq!(s.script.lock().unwrap().launches, 4);
    let verified = send(&mut s.coordinator, ResearchRequest::Verify {}, now + 1);
    let value: serde_json::Value = serde_json::from_str(verified.trim()).unwrap();
    assert!(
        value["child_reports"]["children"]
            .as_array()
            .unwrap()
            .is_empty(),
        "a flat campaign has no child studies: {verified}"
    );
}

#[test]
fn cancelling_a_funnel_campaign_settles_the_host_and_ends_aborted() {
    let mut s = setup(
        Script {
            safe: true,
            ..Default::default()
        },
        40,
        true,
    );
    s.coordinator.drive_attempts(101);
    assert_eq!(phase(&s.coordinator), Some(ResearchPhase::Launching));
    let reply = send(&mut s.coordinator, ResearchRequest::Cancel {}, 102);
    assert!(reply.contains("CancellationRequested"), "{reply}");
    s.coordinator.drive_attempts(103);
    s.coordinator.drive_attempts(104);
    assert_eq!(
        s.script.lock().unwrap().aborts,
        1,
        "the running attempt is settled"
    );
    assert_eq!(
        s.script.lock().unwrap().launches,
        1,
        "nothing else launched"
    );
    assert_eq!(
        s.coordinator.terminal.as_ref().unwrap().phase,
        ResearchPhase::Aborted
    );
}

#[test]
fn a_health_halt_with_unproven_settlement_keeps_the_campaign_and_the_mutation_slot() {
    for abort_fails in [false, true] {
        let mut s = setup(
            Script {
                safe: true,
                ..Default::default()
            },
            40,
            true,
        );
        s.coordinator.drive_attempts(101);
        assert_eq!(phase(&s.coordinator), Some(ResearchPhase::Launching));
        {
            let mut script = s.script.lock().unwrap();
            script.health_blocked = true;
            script.abort_fails = abort_fails;
        }
        s.coordinator.drive_attempts(102);
        if abort_fails {
            // Whether the game is settled is unknown: ownership is retained and nothing else runs.
            assert!(s.coordinator.owns_action_slot(), "ownership is retained");
            assert!(s.coordinator.terminal.is_none(), "no terminal transition");
            assert_eq!(
                s.coordinator.storage_blocker,
                Some("UnsafeToResume"),
                "resumption is suppressed"
            );
        } else {
            assert!(!s.coordinator.owns_action_slot());
            assert_eq!(
                s.coordinator.terminal.as_ref().unwrap().phase,
                ResearchPhase::Blocked
            );
        }
    }
}

#[test]
fn controller_shutdown_settles_a_funnel_attempt_or_refuses_when_settlement_is_unproven() {
    for abort_fails in [false, true] {
        let mut s = setup(
            Script {
                safe: true,
                abort_fails,
                ..Default::default()
            },
            40,
            true,
        );
        s.coordinator.drive_attempts(101);
        let result = s.coordinator.shutdown_driver(102);
        assert_eq!(result.is_err(), abort_fails);
        if abort_fails {
            assert!(s.coordinator.owns_action_slot(), "ownership is retained");
            assert!(s.coordinator.terminal.is_none());
        } else {
            assert!(!s.coordinator.owns_action_slot());
            assert_eq!(
                s.coordinator.terminal.as_ref().unwrap().phase,
                ResearchPhase::Aborted
            );
        }
    }
}
