//! One-time privileged bootstrap: build the runner, pin the campaign's binaries in a root-owned
//! authorization, and install through `sudo -n` (or directly as root). If neither is possible
//! the campaign reports `BLOCKED_PRIVILEGE_BOOTSTRAP`; there is no attempt to bypass
//! authentication, and after a successful bootstrap no further password or interaction is
//! required.
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, ensure};
use stutter_validation_runner::{
    client::Client,
    model::{
        Authorization, LiveSuite, Request, ResponseStatus, RunnerAction, SCHEMA_VERSION, TaskGrant,
    },
};

use super::{
    manifest::CampaignManifest,
    model::CampaignState,
    repository::file_sha256,
    state::{CampaignDir, unix_ms, write_atomic},
};

pub const DEFAULT_SOCKET: &str = "/run/stutter-validation/runner.sock";
/// The task id the controller uses for campaign-level requests (status, resume, cancel).
pub const CAMPAIGN_TASK: &str = "campaign";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivilegeMode {
    Root,
    /// Passwordless `sudo -n` works.
    Sudo,
    None,
}

/// Pure decision so it can be tested without root or sudo.
pub fn detect_privilege(is_root: bool, sudo_works: impl FnOnce() -> bool) -> PrivilegeMode {
    if is_root {
        PrivilegeMode::Root
    } else if sudo_works() {
        PrivilegeMode::Sudo
    } else {
        PrivilegeMode::None
    }
}

fn sudo_noninteractive_works() -> bool {
    Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Which typed actions each task may request. This is the whole privilege surface of the
/// campaign: tasks not listed can do nothing through the runner.
pub fn grants() -> BTreeSet<TaskGrant> {
    let live = |task: &str, suite| TaskGrant {
        task_id: task.to_owned(),
        action: RunnerAction::ExecuteLiveSuite,
        suite: Some(suite),
    };
    let plain = |task: &str, action| TaskGrant {
        task_id: task.to_owned(),
        action,
        suite: None,
    };
    let mut grants = BTreeSet::from([
        live("live-observation", LiveSuite::Observation),
        live("game-runnable", LiveSuite::GameRunnable),
        live("overhead-controlled", LiveSuite::Overhead),
        live("score-runnable", LiveSuite::ScoreDegradation),
        live("kernel-matrix", LiveSuite::KernelCompat),
        live("rc-live-observation", LiveSuite::Observation),
        live("rc-game-runnable", LiveSuite::GameRunnable),
        live("rc-affinity", LiveSuite::Affinity),
        live("rc-recovery", LiveSuite::Recovery),
    ]);
    for task in ["kernel-matrix"] {
        for action in [
            RunnerAction::SetNextBoot,
            RunnerAction::PrepareKernelCycle,
            RunnerAction::Reboot,
            RunnerAction::ResumeCampaign,
            RunnerAction::CancelCampaign,
        ] {
            grants.insert(plain(task, action));
        }
    }
    for task in ["frame-score", "corpus"] {
        grants.insert(plain(task, RunnerAction::ExecuteSchedulerBurst));
    }
    for action in [
        RunnerAction::Status,
        RunnerAction::ResumeCampaign,
        RunnerAction::CancelCampaign,
    ] {
        grants.insert(plain(CAMPAIGN_TASK, action));
    }
    grants
}

pub fn build_authorization(
    state: &CampaignState,
    manifest: &CampaignManifest,
    runner_sha256: &str,
    path_env: &str,
) -> Authorization {
    Authorization {
        schema_version: SCHEMA_VERSION,
        campaign_id: state.campaign_id.clone(),
        owner_uid: state.uid,
        owner_gid: state.gid,
        repository: state.repository.to_string_lossy().into_owned(),
        manifest_sha256: state.manifest_sha256.clone(),
        runner_sha256: runner_sha256.to_owned(),
        xtask_sha256: state.xtask_sha256.clone(),
        stutter_sha256: state.stutter_sha256.clone(),
        path_env: path_env.to_owned(),
        grants: grants(),
        // Refined with the discovered boot entries by the multi-kernel phase.
        max_reboots: u32::try_from(manifest.kernels.max_reboots).unwrap_or(u32::MAX),
        original_boot_entry: None,
        allowed_boot_entries: Vec::new(),
        created_unix_ms: unix_ms(),
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum BootstrapOutcome {
    AlreadyInstalled,
    DryRun,
    Installed,
    BlockedPrivilege,
}

/// Is a runner serving this campaign right now?
fn runner_ready(socket: &Path, state: &CampaignState) -> bool {
    socket.exists()
        && Client {
            socket,
            campaign_id: &state.campaign_id,
            manifest_sha256: &state.manifest_sha256,
        }
        .send(CAMPAIGN_TASK, Request::Status)
        .is_ok_and(|response| response.status == ResponseStatus::Ok)
}

fn build_runner(repository: &Path) -> anyhow::Result<PathBuf> {
    let status = Command::new("cargo")
        .args(["build", "-p", "stutter-validation-runner"])
        .current_dir(repository)
        .status()
        .context("run cargo build")?;
    ensure!(status.success(), "building the validation runner failed");
    let runner = repository.join("target/debug/stutter-validation-runner");
    ensure!(runner.is_file(), "{} was not produced", runner.display());
    Ok(runner)
}

pub fn bootstrap(
    dir: &CampaignDir,
    state: &CampaignState,
    manifest: &CampaignManifest,
    socket: &Path,
    dry_run: bool,
) -> anyhow::Result<BootstrapOutcome> {
    if runner_ready(socket, state) {
        println!(
            "the validation runner already serves campaign {}",
            state.campaign_id
        );
        return Ok(BootstrapOutcome::AlreadyInstalled);
    }
    let is_root = std::os::unix::fs::MetadataExt::uid(&fs::metadata("/proc/self")?) == 0;
    let mode = detect_privilege(is_root, sudo_noninteractive_works);
    if mode == PrivilegeMode::None && !dry_run {
        println!(
            "BLOCKED_PRIVILEGE_BOOTSTRAP: not root and passwordless sudo is unavailable; the \
             validation runner cannot be installed noninteractively"
        );
        return Ok(BootstrapOutcome::BlockedPrivilege);
    }
    let runner = build_runner(&state.repository)?;
    let authorization = build_authorization(
        state,
        manifest,
        &file_sha256(&runner)?,
        &std::env::var("PATH").unwrap_or_default(),
    );
    authorization.validate_shape().map_err(anyhow::Error::msg)?;
    let staged = dir.0.join("runner/authorization.staged.json");
    write_atomic(&staged, &serde_json::to_vec_pretty(&authorization)?)?;
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o600))?;
    if dry_run {
        println!(
            "dry run: would install the runner for campaign {} as {mode:?}",
            state.campaign_id
        );
        println!("{}", serde_json::to_string_pretty(&authorization)?);
        return Ok(BootstrapOutcome::DryRun);
    }
    let mut install = if mode == PrivilegeMode::Root {
        Command::new(&runner)
    } else {
        let mut sudo = Command::new("sudo");
        sudo.arg("-n").arg(&runner);
        sudo
    };
    install
        .args(["install", "--staged"])
        .arg(&staged)
        .arg("--repository")
        .arg(&state.repository);
    let status = install.status().context("run the runner installer")?;
    ensure!(status.success(), "the runner installer failed");
    for _ in 0..50 {
        if runner_ready(socket, state) {
            println!(
                "the validation runner is installed and authenticated for {}",
                state.campaign_id
            );
            return Ok(BootstrapOutcome::Installed);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    anyhow::bail!("the runner was installed but did not answer an authenticated status request")
}
