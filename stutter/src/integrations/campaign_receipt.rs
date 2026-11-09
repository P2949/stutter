//! The trusted way to obtain an [`OfflineAuthorization`] (package W11d).
//!
//! A campaign name alone is not authorization. The proof is a receipt file written by the
//! privileged validation runner: it must sit in a path every component of which is a real
//! directory (no symlink) owned by root and not writable by group or others, and the file itself
//! must be root-owned and read-only to everyone else, so an unprivileged caller cannot write one.
//! The receipt names the campaign, the scopes it grants, the user it is for, the boot it belongs
//! to and when it expires; each of those is checked against the host *now*. Minting also appends a
//! line to the campaign journal and syncs it before the authorization is returned, so an
//! authorization is never in use without a durable record of having been issued.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;

use super::scx_loader::OfflineAuthorization;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CampaignScope {
    PowerProfileHold,
    SchedulerSwitch,
}

impl CampaignScope {
    fn name(self) -> &'static str {
        match self {
            Self::PowerProfileHold => "power_profile_hold",
            Self::SchedulerSwitch => "scheduler_switch",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum MintRefusal {
    /// A path component is a symlink, not owned by the trusted user, or writable by others.
    UntrustedPath(PathBuf),
    Unreadable,
    Malformed,
    Expired,
    /// The receipt belongs to another boot.
    WrongBoot,
    /// The receipt is for a different user than the caller.
    WrongUser,
    ScopeNotGranted,
    /// The issue could not be recorded durably, so nothing was issued.
    JournalFailed,
}

#[derive(Deserialize)]
struct Receipt {
    campaign_id: String,
    scopes: Vec<String>,
    owner_uid: u32,
    boot_id: String,
    expires_unix: u64,
}

/// Every component from `anchor` down to `path` must be a real, non-symlink entry owned by
/// `trusted_uid` that group and others cannot write.
fn trusted(path: &Path, anchor: &Path, trusted_uid: u32) -> Result<(), MintRefusal> {
    let relative = path
        .strip_prefix(anchor)
        .map_err(|_| MintRefusal::UntrustedPath(path.to_path_buf()))?;
    let mut current = anchor.to_path_buf();
    let mut steps = vec![current.clone()];
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(MintRefusal::UntrustedPath(path.to_path_buf()));
        };
        current.push(name);
        steps.push(current.clone());
    }
    for step in steps {
        let meta = fs::symlink_metadata(&step).map_err(|_| MintRefusal::Unreadable)?;
        if meta.file_type().is_symlink() || meta.uid() != trusted_uid || meta.mode() & 0o022 != 0 {
            return Err(MintRefusal::UntrustedPath(step));
        }
    }
    Ok(())
}

/// The facts about the host a receipt is checked against.
#[derive(Clone, Copy)]
struct HostFacts<'a> {
    now_unix: u64,
    boot_id: &'a str,
    caller_uid: u32,
    /// The directory the trust walk starts from (`/` in production).
    anchor: &'a Path,
    /// The user that must own every component (root in production).
    trusted_uid: u32,
}

fn mint_with(
    receipt: &Path,
    journal: &Path,
    scope: CampaignScope,
    host: &HostFacts<'_>,
) -> Result<OfflineAuthorization, MintRefusal> {
    let HostFacts {
        now_unix,
        boot_id,
        caller_uid,
        anchor,
        trusted_uid,
    } = *host;
    trusted(receipt, anchor, trusted_uid)?;
    let text = fs::read_to_string(receipt).map_err(|_| MintRefusal::Unreadable)?;
    let parsed: Receipt = serde_json::from_str(&text).map_err(|_| MintRefusal::Malformed)?;
    if parsed.campaign_id.is_empty() || parsed.campaign_id.len() > 120 {
        return Err(MintRefusal::Malformed);
    }
    if parsed.expires_unix <= now_unix {
        return Err(MintRefusal::Expired);
    }
    if parsed.boot_id.trim() != boot_id.trim() || boot_id.trim().is_empty() {
        return Err(MintRefusal::WrongBoot);
    }
    if parsed.owner_uid != caller_uid {
        return Err(MintRefusal::WrongUser);
    }
    if !parsed.scopes.iter().any(|granted| granted == scope.name()) {
        return Err(MintRefusal::ScopeNotGranted);
    }
    // Durable before use: the issue is recorded and synced, or nothing is issued.
    let line = format!(
        "{}\n",
        serde_json::json!({
            "campaign": parsed.campaign_id,
            "scope": scope.name(),
            "issued_unix": now_unix,
        })
    );
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(journal)
        .map_err(|_| MintRefusal::JournalFailed)?;
    file.write_all(line.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| MintRefusal::JournalFailed)?;
    Ok(OfflineAuthorization::minted(parsed.campaign_id))
}

/// Mints an authorization for `scope` from a runner-written receipt, as the calling user, now.
pub fn mint_authorization(
    receipt: &Path,
    journal: &Path,
    scope: CampaignScope,
) -> Result<OfflineAuthorization, MintRefusal> {
    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
    let caller_uid = crate::syscall::geteuid();
    mint_with(
        receipt,
        journal,
        scope,
        &HostFacts {
            now_unix,
            boot_id: &boot_id,
            caller_uid,
            anchor: Path::new("/"),
            trusted_uid: 0,
        },
    )
}

#[cfg(test)]
#[path = "campaign_receipt_tests.rs"]
mod tests;
