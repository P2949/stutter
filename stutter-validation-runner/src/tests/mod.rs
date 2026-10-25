//! Runner tests. Pure protocol/policy/state tests run everywhere; tests that need real root
//! detect it at run time and say so instead of silently passing.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::{
    model::{Authorization, Envelope, LiveSuite, Request, RunnerAction, SCHEMA_VERSION, TaskGrant},
    sys::{self, PeerCredentials},
};

mod authorization;
mod boot;
mod credentials;
mod install;
mod packaging;
mod process;
mod recording;
mod resume;
mod server;
mod worker;

pub(super) const MANIFEST: &str =
    "1111111111111111111111111111111111111111111111111111111111111111";
pub(super) const DIGEST: &str = "2222222222222222222222222222222222222222222222222222222222222222";

static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(super) struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "stutter-runner-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A uid that is not root and not the current process's.
pub(super) fn me() -> u32 {
    let uid = sys::effective_uid();
    if uid == 0 { 1000 } else { uid }
}

pub(super) fn peer(uid: u32) -> PeerCredentials {
    PeerCredentials {
        pid: 4242,
        uid,
        gid: uid,
    }
}

pub(super) fn grant(task: &str, action: RunnerAction, suite: Option<LiveSuite>) -> TaskGrant {
    TaskGrant {
        task_id: task.to_owned(),
        action,
        suite,
    }
}

pub(super) fn authorization(repository: &str) -> Authorization {
    Authorization {
        schema_version: SCHEMA_VERSION,
        controller: crate::model::ControllerKind::CoreValidation,
        campaign_id: "core-abc1234-1".to_owned(),
        owner_uid: me(),
        owner_gid: me(),
        repository: repository.to_owned(),
        manifest_sha256: MANIFEST.to_owned(),
        runner_sha256: DIGEST.to_owned(),
        xtask_sha256: DIGEST.to_owned(),
        stutter_sha256: DIGEST.to_owned(),
        path_env: "/usr/bin:/bin".to_owned(),
        grants: BTreeSet::from([
            grant("status", RunnerAction::Status, None),
            grant(
                "live-observation",
                RunnerAction::ExecuteLiveSuite,
                Some(LiveSuite::Observation),
            ),
            grant(
                "overhead-controlled",
                RunnerAction::ExecuteLiveSuite,
                Some(LiveSuite::Overhead),
            ),
            grant("score-runnable", RunnerAction::ExecuteSchedulerBurst, None),
            grant("kernel-matrix", RunnerAction::SetNextBoot, None),
            grant("kernel-matrix", RunnerAction::Reboot, None),
            grant("kernel-matrix", RunnerAction::ResumeCampaign, None),
            grant("kernel-matrix", RunnerAction::CancelCampaign, None),
        ]),
        max_reboots: 2,
        reboot_slack: 2,
        original_boot_entry: Some("original".to_owned()),
        allowed_boot_entries: vec!["original".to_owned(), "kernel-b".to_owned()],
        created_unix_ms: 1,
    }
}

pub(super) fn envelope(task: &str, body: Request) -> Envelope {
    Envelope {
        schema_version: SCHEMA_VERSION,
        campaign_id: "core-abc1234-1".to_owned(),
        task_id: task.to_owned(),
        request_id: format!(
            "req-{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ),
        manifest_sha256: MANIFEST.to_owned(),
        body,
    }
}

pub(super) fn observation() -> Request {
    Request::ExecuteLiveSuite {
        suite: LiveSuite::Observation,
        attempt: 1,
        seconds: Some(20),
        rounds: None,
        benefit: None,
        mutation: None,
    }
}
