//! Root grants and source checks share the established bootstrap and binary provenance code.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use anyhow::ensure;
use stutter_validation_runner::{
    client::{Client, SOCKET_ENV},
    model::{ControllerKind, Request, ResponseStatus, RunnerAction, TaskGrant},
};

use super::{
    manifest,
    model::{Stage, State},
};
use crate::validation_campaign::{
    bootstrap::{CAMPAIGN_TASK, DEFAULT_SOCKET, RunnerTarget},
    repository::{Binaries, check_binary_revision, file_sha256, git},
};
pub fn socket() -> PathBuf {
    std::env::var_os(SOCKET_ENV)
        .filter(|value| !value.is_empty())
        .map_or_else(|| DEFAULT_SOCKET.into(), PathBuf::from)
}
pub fn target(state: &State) -> RunnerTarget {
    let mut grants = BTreeSet::new();
    for stage in Stage::ALL {
        grants.insert(TaskGrant {
            task_id: stage.id().to_owned(),
            action: RunnerAction::BoundedService,
            suite: None,
        });
        if matches!(stage, Stage::Discovery | Stage::Recovery | Stage::Soak) {
            grants.insert(TaskGrant {
                task_id: stage.id().to_owned(),
                action: RunnerAction::ExecuteSchedulerBurst,
                suite: None,
            });
        }
    }
    for action in [
        RunnerAction::Status,
        RunnerAction::ResumeCampaign,
        RunnerAction::CancelCampaign,
    ] {
        grants.insert(TaskGrant {
            task_id: CAMPAIGN_TASK.to_owned(),
            action,
            suite: None,
        });
    }
    grants.insert(TaskGrant {
        task_id: Stage::Reboots.id().to_owned(),
        action: RunnerAction::Reboot,
        suite: None,
    });
    RunnerTarget {
        campaign_id: state.campaign_id.clone(),
        uid: state.uid,
        gid: state.gid,
        repository: state.repository.clone(),
        manifest_sha256: state.manifest_sha256.clone(),
        xtask_sha256: state.xtask_sha256.clone(),
        stutter_sha256: state.stutter_sha256.clone(),
        controller: ControllerKind::ServiceGraduation,
        grants,
        max_reboots: state.required_reboots,
        reboot_slack: 0,
    }
}
pub fn integrity(root: &Path, state: &State) -> anyhow::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let owner = std::fs::metadata("/proc/self")?;
    ensure!(
        owner.uid() == state.uid && owner.gid() == state.gid,
        "service campaign owner changed"
    );
    ensure!(
        root.canonicalize()? == state.repository.canonicalize()?,
        "service campaign repository binding changed"
    );
    ensure!(
        std::fs::read_to_string("/proc/sys/kernel/osrelease")?.trim() == state.kernel_release,
        "service campaign kernel contract changed"
    );
    ensure!(
        git(root, &["status", "--porcelain"])?.is_empty(),
        "service campaign source is dirty"
    );
    ensure!(
        git(root, &["rev-parse", "HEAD"])? == state.source_commit,
        "service campaign source moved"
    );
    for (path, expected) in [
        (root.join(manifest::FILE), &state.manifest_sha256),
        (root.join("Cargo.lock"), &state.cargo_lock_sha256),
        (root.join("target/debug/stutter"), &state.stutter_sha256),
        (root.join("target/debug/xtask"), &state.xtask_sha256),
    ] {
        ensure!(
            file_sha256(&path)? == *expected,
            "service campaign fingerprint changed: {}",
            path.display()
        );
    }
    check_binary_revision(
        &Binaries::for_repository(root)?.stutter,
        &state.source_commit,
    )
}
pub fn request(state: &State, stage: Stage, body: Request) -> anyhow::Result<serde_json::Value> {
    let socket = socket();
    let response = Client {
        socket: &socket,
        campaign_id: &state.campaign_id,
        manifest_sha256: &state.manifest_sha256,
    }
    .send(stage.id(), body)?;
    ensure!(
        response.status == ResponseStatus::Ok,
        "typed service action refused: {:?}",
        response.error
    );
    Ok(response.data.unwrap_or(serde_json::Value::Null))
}

/// A request the runner grants under the campaign task rather than a stage task (status,
/// resume acknowledgement, cancel).
pub fn request_campaign(state: &State, body: Request) -> anyhow::Result<serde_json::Value> {
    let socket = socket();
    let response = Client {
        socket: &socket,
        campaign_id: &state.campaign_id,
        manifest_sha256: &state.manifest_sha256,
    }
    .send(crate::validation_campaign::bootstrap::CAMPAIGN_TASK, body)?;
    ensure!(
        response.status == ResponseStatus::Ok,
        "typed campaign request refused: {:?}",
        response.error
    );
    Ok(response.data.unwrap_or(serde_json::Value::Null))
}
