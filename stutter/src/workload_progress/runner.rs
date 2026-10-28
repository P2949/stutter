use std::{path::Path, process::Command};

use anyhow::{Context, bail};

use super::{FixedWorkIdentity, ProgressSource, WorkloadPhase, WorkloadProgress};

/// A Git command for `dir` that ignores the repository an enclosing hook or tool exported.
pub(super) fn git_in(dir: impl AsRef<Path>) -> Command {
    let mut command = Command::new("git");
    command.current_dir(dir);
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_PREFIX",
    ] {
        command.env_remove(name);
    }
    command
}

fn verify_source(identity: &FixedWorkIdentity) -> anyhow::Result<()> {
    let expected = identity.source_tree.strip_prefix("git:").ok_or_else(|| anyhow::anyhow!("command runner requires a clean Git tree identity; digest-based external markers need an external source verifier"))?;
    let tree = git_in(&identity.working_directory)
        .args(["rev-parse", "HEAD^{tree}"])
        .output()?;
    let status = git_in(&identity.working_directory)
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()?;
    if !tree.status.success()
        || !status.status.success()
        || String::from_utf8_lossy(&tree.stdout).trim() != expected
        || !status.stdout.is_empty()
    {
        bail!("fixed-work source is dirty or does not match the declared Git tree");
    }
    Ok(())
}

fn require_target_ancestor(root: u32) -> anyhow::Result<()> {
    let mut pid = std::process::id();
    for _ in 0..128 {
        if pid == root {
            return Ok(());
        }
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let parent = stat
            .rsplit_once(')')
            .and_then(|(_, fields)| fields.split_whitespace().nth(1))
            .and_then(|parent| parent.parse::<u32>().ok())
            .filter(|parent| *parent > 0 && *parent != pid)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "fixed-work target must contain the command runner in its process tree"
                )
            })?;
        pid = parent;
    }
    bail!("fixed-work target ancestry exceeds validation depth");
}

/// Execute the declared argv directly. Only a successful command with unchanged
/// source and target identity may emit WORKLOAD_COMPLETE. The callback can feed
/// a recorder, scenario, or an NDJSON marker stream without a compiler adapter.
pub fn run_fixed_work(
    identity: FixedWorkIdentity,
    execution_id: String,
    target_root_pid: u32,
    mut emit: impl FnMut(&WorkloadProgress) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    identity.validate().map_err(anyhow::Error::msg)?;
    require_target_ancestor(target_root_pid)?;
    verify_source(&identity)?;
    let starttime = crate::process_tree::process_starttime_at(Path::new("/proc"), target_root_pid)
        .ok_or_else(|| anyhow::anyhow!("fixed-work target identity unavailable"))?;
    let start = WorkloadProgress {
        schema_version: 1,
        execution_id,
        identity,
        source: ProgressSource::CustomCommand,
        phase: WorkloadPhase::Start,
        target_root_pid,
        target_starttime_ticks: starttime,
        monotonic_ns: crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC)?,
        elapsed_ns: 0,
        completed_units: 0,
        total_units: Some(1),
    };
    start.validate().map_err(anyhow::Error::msg)?;
    emit(&start)?;
    // invariant: identity.validate requires at least one command element.
    let status = Command::new(&start.identity.command[0])
        .args(&start.identity.command[1..])
        .current_dir(&start.identity.working_directory)
        .status();
    let completed_ns = crate::syscall::clock_gettime_ns(libc::CLOCK_MONOTONIC)?;
    let unchanged_source = verify_source(&start.identity);
    let same_target =
        crate::process_tree::process_starttime_at(Path::new("/proc"), target_root_pid)
            == Some(starttime);
    let success = status.as_ref().is_ok_and(|status| status.success())
        && unchanged_source.is_ok()
        && same_target
        && completed_ns > start.monotonic_ns;
    let complete = WorkloadProgress {
        phase: if success {
            WorkloadPhase::Complete
        } else {
            WorkloadPhase::Failed
        },
        monotonic_ns: completed_ns,
        elapsed_ns: completed_ns
            .checked_sub(start.monotonic_ns)
            .ok_or_else(|| anyhow::anyhow!("monotonic clock moved backwards"))?,
        completed_units: u64::from(success),
        ..start
    };
    emit(&complete)?;
    let status = status.context("fixed-work command failed to start")?;
    if !status.success() {
        bail!("fixed-work command exited unsuccessfully: {status}");
    }
    unchanged_source?;
    if !same_target {
        bail!("fixed-work target identity changed during command");
    }
    complete.validate().map_err(anyhow::Error::msg)
}
