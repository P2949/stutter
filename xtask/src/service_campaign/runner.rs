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
    repository::{check_binary_revision, file_sha256, git},
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
        if matches!(
            stage,
            Stage::Discovery | Stage::Recovery | Stage::Reboots | Stage::Soak
        ) {
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
        (state.pinned_stutter(), &state.stutter_sha256),
        (state.pinned_xtask(), &state.xtask_sha256),
    ] {
        ensure!(
            file_sha256(&path)? == *expected,
            "service campaign fingerprint changed: {}",
            path.display()
        );
    }
    check_binary_revision(&state.pinned_stutter(), &state.source_commit)?;
    let system = system_closure("/run/current-system")?;
    ensure!(
        system == state.host_system,
        "service campaign host system changed from {} to {system}",
        state.host_system
    );
    Ok(())
}

/// The NixOS system closure a link resolves to, so a reboot or switch into another one is caught.
pub fn system_closure(link: &str) -> anyhow::Result<String> {
    use anyhow::Context as _;
    Ok(std::fs::canonicalize(link)
        .with_context(|| format!("resolve {link}"))?
        .display()
        .to_string())
}
pub fn request(state: &State, stage: Stage, body: Request) -> anyhow::Result<serde_json::Value> {
    let socket = socket();
    let response = Client {
        socket: &socket,
        campaign_id: &state.campaign_id,
        manifest_sha256: &state.manifest_sha256,
    }
    .send(stage.id(), body)?;
    if let Some(error) = &response.error
        && error.code == stutter_validation_runner::model::ErrorCode::SuspendNotEntered
    {
        return Err(anyhow::Error::new(SuspendNotEntered(error.message.clone())));
    }
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

/// Asks the root runner to cancel this campaign. `Ok(true)` is a durable cancellation;
/// `Ok(false)` means the runner does not serve this campaign at all, so nothing of it is armed;
/// anything else is a failure and the campaign must not be called cancelled.
pub fn cancel_campaign(state: &State) -> anyhow::Result<bool> {
    use stutter_validation_runner::model::ErrorCode;
    let socket = socket();
    if !socket.exists() {
        // Without a runner a campaign that never installed has nothing armed; one that did must
        // be cancelled where its resume state lives.
        anyhow::ensure!(
            state.tasks[Stage::Install.id()].status
                != crate::validation_campaign::model::TaskStatus::Passed,
            "the validation runner socket is absent, so an installed campaign cannot be cancelled"
        );
        return Ok(false);
    }
    let response = Client {
        socket: &socket,
        campaign_id: &state.campaign_id,
        manifest_sha256: &state.manifest_sha256,
    }
    .send(
        crate::validation_campaign::bootstrap::CAMPAIGN_TASK,
        Request::CancelCampaign,
    )?;
    match (response.status, response.error) {
        (ResponseStatus::Ok, _) => Ok(true),
        (_, Some(error)) if error.code == ErrorCode::UnknownCampaign => Ok(false),
        (_, error) => anyhow::bail!("root cancellation was refused: {error:?}"),
    }
}

/// The root runner verified that the kernel never entered sleep, so a suspend fault was not
/// injected. Carried as a type so no caller has to read error text.
#[derive(Debug)]
pub struct SuspendNotEntered(pub String);

impl std::fmt::Display for SuspendNotEntered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the kernel did not suspend the machine: {}", self.0)
    }
}

impl std::error::Error for SuspendNotEntered {}
