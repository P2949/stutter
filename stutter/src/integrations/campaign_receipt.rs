//! The trusted way to obtain an [`OfflineAuthorization`] (package W11d).
//!
//! A campaign name alone is not authorization. The proof is a receipt file written by the
//! privileged validation runner: it must sit in a path every component of which is a real
//! directory (no symlink) owned by root and not writable by group or others, and the file itself
//! must be root-owned and read-only to everyone else, so an unprivileged caller cannot write one.
//! The receipt names the campaign, the scopes it grants, the user it is for, the boot it belongs
//! to and when it expires.
//!
//! The authorization this returns is bound to exactly one scope, user, boot and expiry, and to the
//! identity of the receipt file itself. Every adapter checks all of that again at the moment it
//! acts ([`OfflineAuthorization::require`]), so an authorization cannot be used for another
//! scope, after it expires, in another boot, by another user, or after its receipt was removed or
//! replaced (revocation). The authoritative record of an issue is the runner's root-owned
//! `issued.jsonl`; the journal line written here is the caller's own local use record and proves
//! nothing on its own.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

use serde::Deserialize;

use super::scx_loader::OfflineAuthorization;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CampaignScope {
    PowerProfileHold,
    SchedulerSwitch,
    /// Runtime resource-control properties on Stutter-owned systemd units.
    UnitProperties,
}

impl CampaignScope {
    fn name(self) -> &'static str {
        match self {
            Self::PowerProfileHold => "power_profile_hold",
            Self::SchedulerSwitch => "scheduler_switch",
            Self::UnitProperties => "unit_properties",
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
    /// The issue could not be recorded locally, so nothing was issued.
    JournalFailed,
}

/// Why an authorization may not be used for an action right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorizationDenied {
    WrongScope {
        required: CampaignScope,
        granted: CampaignScope,
    },
    Expired,
    WrongBoot,
    WrongUser,
    /// The receipt it came from is gone, replaced or no longer trusted.
    Revoked,
}

#[derive(Deserialize)]
struct Receipt {
    campaign_id: String,
    scopes: Vec<String>,
    owner_uid: u32,
    boot_id: String,
    expires_unix: u64,
}

/// The facts an authorization is checked against when it is used.
#[derive(Clone, Debug)]
pub(super) struct UseFacts {
    pub now_unix: u64,
    pub boot_id: String,
    pub caller_uid: u32,
}

impl UseFacts {
    pub(super) fn now() -> Self {
        Self {
            now_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
            boot_id: current_boot_id(),
            caller_uid: crate::syscall::geteuid(),
        }
    }
}

pub(super) fn current_boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// The identity of the receipt file an authorization came from, re-verified at every use.
#[derive(Clone, Debug)]
pub(super) struct ReceiptBinding {
    path: PathBuf,
    anchor: PathBuf,
    trusted_uid: u32,
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
}

impl ReceiptBinding {
    fn of(path: &Path, meta: &fs::Metadata, anchor: &Path, trusted_uid: u32) -> Self {
        Self {
            path: path.to_path_buf(),
            anchor: anchor.to_path_buf(),
            trusted_uid,
            dev: meta.dev(),
            ino: meta.ino(),
            len: meta.len(),
            mtime: (meta.mtime(), meta.mtime_nsec()),
        }
    }

    /// The same file, still reached through a trusted path.
    pub(super) fn still_valid(&self) -> bool {
        trusted(&self.path, &self.anchor, self.trusted_uid).is_ok()
            && fs::symlink_metadata(&self.path).is_ok_and(|meta| {
                (
                    meta.dev(),
                    meta.ino(),
                    meta.len(),
                    (meta.mtime(), meta.mtime_nsec()),
                ) == (self.dev, self.ino, self.len, self.mtime)
            })
    }
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

/// Opens the receipt without following a symlink, checks the *open file's* own owner and mode (so
/// nothing can be swapped between the check and the read), and returns its text and identity.
fn read_receipt(
    path: &Path,
    host: &HostFacts<'_>,
) -> Result<(String, ReceiptBinding), MintRefusal> {
    trusted(path, host.anchor, host.trusted_uid)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| MintRefusal::Unreadable)?;
    let meta = file.metadata().map_err(|_| MintRefusal::Unreadable)?;
    if !meta.is_file() || meta.uid() != host.trusted_uid || meta.mode() & 0o022 != 0 {
        return Err(MintRefusal::UntrustedPath(path.to_path_buf()));
    }
    if meta.len() > 64 * 1024 {
        return Err(MintRefusal::Malformed);
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|_| MintRefusal::Unreadable)?;
    Ok((
        text,
        ReceiptBinding::of(path, &meta, host.anchor, host.trusted_uid),
    ))
}

/// Opens the caller's local use journal without following a symlink, and only if it is the
/// caller's own regular file.
fn open_local_journal(journal: &Path, caller_uid: u32) -> Result<File, MintRefusal> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(journal)
        .map_err(|_| MintRefusal::JournalFailed)?;
    let meta = file.metadata().map_err(|_| MintRefusal::JournalFailed)?;
    if !meta.is_file() || meta.uid() != caller_uid {
        return Err(MintRefusal::JournalFailed);
    }
    Ok(file)
}

fn mint_with(
    receipt: &Path,
    journal: &Path,
    scope: CampaignScope,
    host: &HostFacts<'_>,
) -> Result<OfflineAuthorization, MintRefusal> {
    let (text, binding) = read_receipt(receipt, host)?;
    let parsed: Receipt = serde_json::from_str(&text).map_err(|_| MintRefusal::Malformed)?;
    if parsed.campaign_id.is_empty() || parsed.campaign_id.len() > 120 {
        return Err(MintRefusal::Malformed);
    }
    if parsed.expires_unix <= host.now_unix {
        return Err(MintRefusal::Expired);
    }
    if parsed.boot_id.trim() != host.boot_id.trim() || host.boot_id.trim().is_empty() {
        return Err(MintRefusal::WrongBoot);
    }
    if parsed.owner_uid != host.caller_uid {
        return Err(MintRefusal::WrongUser);
    }
    if !parsed.scopes.iter().any(|granted| granted == scope.name()) {
        return Err(MintRefusal::ScopeNotGranted);
    }
    // Recorded locally before use: the issue is written and synced, or nothing is issued.
    let line = format!(
        "{}\n",
        serde_json::json!({
            "campaign": parsed.campaign_id,
            "scope": scope.name(),
            "issued_unix": host.now_unix,
        })
    );
    let mut file = open_local_journal(journal, host.caller_uid)?;
    file.write_all(line.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| MintRefusal::JournalFailed)?;
    Ok(OfflineAuthorization::minted(
        parsed.campaign_id,
        scope,
        parsed.owner_uid,
        parsed.boot_id.trim().to_owned(),
        parsed.expires_unix,
        binding,
    ))
}

/// Mints an authorization for `scope` from a runner-written receipt, as the calling user, now.
pub fn mint_authorization(
    receipt: &Path,
    journal: &Path,
    scope: CampaignScope,
) -> Result<OfflineAuthorization, MintRefusal> {
    let facts = UseFacts::now();
    mint_with(
        receipt,
        journal,
        scope,
        &HostFacts {
            now_unix: facts.now_unix,
            boot_id: &facts.boot_id,
            caller_uid: facts.caller_uid,
            anchor: Path::new("/"),
            trusted_uid: 0,
        },
    )
}

#[cfg(test)]
#[path = "campaign_receipt_tests.rs"]
mod tests;
