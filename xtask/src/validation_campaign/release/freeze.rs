//! `freeze-rc`: pick the exact release candidate (clean local `main`), rebuild the binaries
//! at it, record their digests separately from the evidence-base ones, re-pin the privileged
//! runner to them and ask the controller to restart from the fresh build.
use std::{fs, path::Path};

use super::{blocked, failed};
use crate::validation_campaign::{
    bootstrap::{self, BootstrapOutcome, DEFAULT_SOCKET},
    executor::{RESTART_KEY, RunContext, RunnerOutcome, StateUpdate, TaskRunner},
    graph::TaskSpec,
    model::{BlockReason, FailureClass},
    repository::{file_sha256, reports_revision},
    shell::Shell,
};

/// Re-pins the runner to the RC binaries; `Ok` also when no runner is in use.
pub type Repin = dyn Fn(&RunContext<'_>, &str, &str) -> Result<(), BlockReason>;

pub struct FreezeRcRunner {
    pub shell: Box<dyn Shell>,
    pub repin: Box<Repin>,
}

/// The real re-pin: reinstall the runner with an authorization pinned to the RC digests.
pub fn repin_runner(
    context: &RunContext<'_>,
    stutter_sha256: &str,
    xtask_sha256: &str,
) -> Result<(), BlockReason> {
    let socket = std::env::var_os(stutter_validation_runner::client::SOCKET_ENV)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| DEFAULT_SOCKET.into());
    if !socket.exists() {
        return Ok(());
    }
    let mut pinned = context.state.clone();
    pinned.stutter_sha256 = stutter_sha256.to_owned();
    pinned.xtask_sha256 = xtask_sha256.to_owned();
    match bootstrap::bootstrap(context.dir, &pinned, context.manifest, &socket, false) {
        Ok(BootstrapOutcome::BlockedPrivilege) => Err(BlockReason {
            class: FailureClass::BlockedPrivilege,
            detail: "cannot re-pin the validation runner to the release-candidate binaries without privilege".to_owned(),
        }),
        Ok(_) => Ok(()),
        Err(error) => Err(BlockReason {
            class: FailureClass::Retryable,
            detail: format!("{error:#}"),
        }),
    }
}

impl TaskRunner for FreezeRcRunner {
    fn run(&self, context: &RunContext<'_>, _spec: &TaskSpec, _attempt: u32) -> RunnerOutcome {
        let repo = context.repository;
        let git = |args: &[&str]| self.shell.run("git", args, repo);
        // Local clean `main` is authoritative: there is no remote to pull from.
        let out = git(&["checkout", "main"]);
        if !out.ok() {
            return failed(
                FailureClass::Retryable,
                "sync",
                format!("git checkout main failed: {}", out.stderr.trim()),
            );
        }
        if !git(&["status", "--porcelain"]).stdout.trim().is_empty() {
            return blocked(
                FailureClass::BlockedResource,
                "main is not clean: the release candidate must be an exact committed state",
            );
        }
        let head = git(&["rev-parse", "HEAD"]);
        let commit = head.stdout.trim().to_owned();
        if !head.ok() || commit.len() != 40 {
            return failed(FailureClass::Retryable, "rev-parse", "cannot read HEAD");
        }
        let build = self.shell.run(
            "cargo",
            &[
                "build",
                "-p",
                "stutter",
                "-p",
                "xtask",
                "-p",
                "stutter-validation-runner",
            ],
            repo,
        );
        if !build.ok() {
            return failed(
                FailureClass::SoftwareDefect,
                "build",
                format!(
                    "building the release candidate failed: {}",
                    build.stderr.lines().last().unwrap_or("")
                ),
            );
        }
        let stutter = repo.join("target/debug/stutter");
        let xtask = repo.join("target/debug/xtask");
        let version = self
            .shell
            .run(&stutter.to_string_lossy(), &["--version"], repo);
        if !reports_revision(&version.stdout, &commit) {
            return failed(
                FailureClass::SoftwareDefect,
                "revision",
                format!(
                    "the rebuilt stutter reports {:?}, not {commit}",
                    version.stdout.trim()
                ),
            );
        }
        let (Ok(stutter_sha256), Ok(xtask_sha256)) = (file_sha256(&stutter), file_sha256(&xtask))
        else {
            return failed(
                FailureClass::Retryable,
                "digest",
                "cannot hash the rebuilt binaries",
            );
        };
        if let Err(block) = (self.repin)(context, &stutter_sha256, &xtask_sha256) {
            return RunnerOutcome::Blocked(block);
        }
        if let Err(error) = write_marker(context.dir.0.as_path(), &commit) {
            return failed(FailureClass::Retryable, "storage", error);
        }
        RunnerOutcome::Passed {
            artifacts: Some("release".to_owned()),
            receipt: None,
            limitations: Vec::new(),
            fingerprint: Box::default(),
            updates: vec![
                StateUpdate::SetReleaseCandidate(commit),
                StateUpdate::SetReleaseBinaries {
                    stutter_sha256,
                    xtask_sha256,
                },
                // The remaining tasks must run in a process built from the candidate.
                StateUpdate::Freeze {
                    key: RESTART_KEY.to_owned(),
                    value: "1".to_owned(),
                },
            ],
        }
    }
}

fn write_marker(campaign: &Path, commit: &str) -> Result<(), String> {
    let dir = super::release_dir(campaign).map_err(|e| e.to_string())?;
    fs::write(dir.join("release-candidate.txt"), format!("{commit}\n")).map_err(|e| e.to_string())
}
