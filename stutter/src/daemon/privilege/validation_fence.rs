//! A validation-only target fence for the privileged worker.
//!
//! Normal operation lets the controller's heuristic focus choose its target, and the worker checks
//! that the named task is still the live one and belongs to the service account and cgroup. A
//! validation run that mutates real resources needs more: proof that the only task it can touch is
//! its own disposable fixture, whatever else the host is running. The fence is that proof, enforced
//! where the mutation is authorised.
//!
//! The fence is a small file in the worker's root-owned state directory naming one process (pid and
//! start time), the account, the boot and an expiry. It can only **narrow**:
//!
//! * with no fence file, nothing changes;
//! * with a valid fence, a candidate is refused unless every task it names belongs to the fenced
//!   process, and a fenced task still has to pass every ordinary check (the fence grants nothing);
//! * a fence that is present but cannot be trusted, read, parsed, or is for another account or boot,
//!   or has expired, refuses **every** mutation. A stale file therefore stops tuning until it is
//!   removed, which is the safe direction and is why the reason says to remove it.
//!
//! The file must be written where only root can write (the owner, mode and absence of symlinks are
//! checked on the open file), so an unprivileged caller cannot fence the worker to a target of its
//! choosing, nor lift a fence. It never widens what the service account may do.
use std::path::Path;

use anyhow::{Context, bail, ensure};
use serde::Deserialize;

/// The fence file's name inside the worker's state directory.
pub(super) const FENCE_FILE: &str = "validation-fence.json";
const MAX_FENCE_BYTES: u64 = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fence {
    version: u32,
    uid: u32,
    root_pid: u32,
    start_ticks: u64,
    boot_id: String,
    expires_unix: u64,
}

/// What the fence file says.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum FenceState {
    /// No fence: ordinary operation.
    Absent,
    /// A validation session is open but no fixture is: every new target is refused (restoring what is
    /// already held is not a target and stays possible).
    DenyAll,
    /// Only the process `root_pid`, started at `start_ticks`, may be targeted.
    Active { root_pid: u32, start_ticks: u64 },
}

/// Reads the fence. Absence is the only way to be unfenced; every other failure refuses mutation.
pub(super) fn load(
    path: &Path,
    now_unix: u64,
    boot_id: &str,
    service_uid: u32,
    trusted_uid: u32,
) -> anyhow::Result<FenceState> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FenceState::Absent);
        }
        Err(error) => bail!("validation_fence_unreadable: {error}"),
        Ok(_) => {}
    }
    let anchor = path
        .parent()
        .context("validation_fence_untrusted: the fence has no directory")?;
    let text = crate::integrations::campaign_receipt::read_trusted_text(
        path,
        anchor,
        trusted_uid,
        MAX_FENCE_BYTES,
    )
    .map_err(|refusal| anyhow::anyhow!("validation_fence_untrusted: {refusal:?}"))?;
    let fence: Fence = serde_json::from_str(&text)
        .map_err(|error| anyhow::anyhow!("validation_fence_malformed: {error}"))?;
    ensure!(
        fence.version == 1,
        "validation_fence_malformed: unknown version"
    );
    ensure!(fence.uid == service_uid, "validation_fence_wrong_account");
    ensure!(
        !boot_id.is_empty() && fence.boot_id == boot_id,
        "validation_fence_wrong_boot"
    );
    ensure!(
        now_unix < fence.expires_unix,
        "validation_fence_expired_remove_it"
    );
    if fence.root_pid == 0 {
        return Ok(FenceState::DenyAll);
    }
    ensure!(fence.root_pid > 1, "validation_fence_malformed: no target");
    Ok(FenceState::Active {
        root_pid: fence.root_pid,
        start_ticks: fence.start_ticks,
    })
}

/// Whether a task of process `tgid` may be targeted under `state`.
pub(super) fn check_task(state: &FenceState, proc_root: &Path, tgid: u32) -> anyhow::Result<()> {
    if *state == FenceState::DenyAll {
        bail!("validation_fence_deny_all: no fixture is active in this validation session");
    }
    let FenceState::Active {
        root_pid,
        start_ticks,
    } = state
    else {
        return Ok(());
    };
    ensure!(tgid == *root_pid, "validation_fence_target_not_fenced");
    // The same pid is the fixture only if it is still the process that was fenced.
    let live = crate::telemetry::proc_view::live_process(proc_root, *root_pid)
        .context("validation_fence_target_not_fenced: the fenced process is gone")?;
    ensure!(
        live.start_ticks == *start_ticks,
        "validation_fence_target_not_fenced: the fenced pid was reused"
    );
    Ok(())
}

#[cfg(test)]
#[path = "validation_fence_tests.rs"]
mod tests;
