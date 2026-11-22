//! The player's own data, which a research campaign must leave exactly as it found it (PLAN section
//! 6.1: never delete or change the player's saves, prefix or cloud state; G2: "original data
//! intact"). The study runs in private prefixes, so nothing here should ever change; this is the
//! independent check that nothing did. Before any launch the campaign records a read-only fingerprint
//! of the Cyberpunk save directory in the player's prefix and of the Steam Cloud copy under each
//! account's `userdata`; verification takes the fingerprint again and compares. It reads and hashes,
//! never writes outside the study directory, and does not follow symbolic links.
//!
//! The fingerprint is a content hash, not a timestamp comparison, so a touched-but-identical file is
//! not a change and a rewritten file with the same size is. A tree too large to hash is an error, not a
//! partial fingerprint: an unverifiable state is never reported as unchanged.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::report_store::{read_private, write_new};

const APP_ID: u32 = 1_091_500;
/// More entries than this under one root is not a save directory.
const MAX_ENTRIES: usize = 50_000;
/// A single file larger than this is not hashed (saves are a few megabytes).
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
/// The most bytes hashed across all roots.
const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootPrint {
    /// A stable label for the root (`saves`, or a digest-labeled cloud account), never a private path.
    pub label: String,
    pub present: bool,
    pub entries: u64,
    pub bytes: u64,
    /// SHA-256 over every entry's relative path, kind, size and content hash, in path order.
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema: u32,
    pub roots: Vec<RootPrint>,
}

/// Discover the actual game library independently from the primary client's userdata directory.
fn roots(primary_steamapps: &Path) -> Result<Vec<(String, PathBuf)>, &'static str> {
    let uid = crate::syscall::geteuid();
    let libraries = super::steam_inventory::discover_steam_libraries(primary_steamapps, uid)
        .map_err(|_| "ProtectedStateUnreadable")?;
    let (game_steamapps, _) =
        super::steam_inventory::discover_cyberpunk_installation(&libraries, uid)
            .map_err(|_| "ProtectedStateUnreadable")?;
    let mut roots = vec![(
        "saves".to_owned(),
        game_steamapps
            .join("compatdata")
            .join(APP_ID.to_string())
            .join("pfx/drive_c/users/steamuser/Saved Games/CD Projekt Red/Cyberpunk 2077"),
    )];
    let userdata = primary_steamapps
        .parent()
        .ok_or("ProtectedStateUnreadable")?
        .join("userdata");
    let meta = std::fs::symlink_metadata(&userdata).map_err(|_| "ProtectedStateUnreadable")?;
    if !meta.is_dir() {
        return Err("ProtectedStateUnreadable");
    }
    crate::syscall::open_dir_no_symlinks(&userdata).map_err(|_| "ProtectedStateUnreadable")?;
    let accounts = std::fs::read_dir(&userdata)
        .map_err(|_| "ProtectedStateUnreadable")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "ProtectedStateUnreadable")?;
    let mut ids = Vec::new();
    for account in accounts {
        let id = account
            .file_name()
            .into_string()
            .map_err(|_| "ProtectedStateUnreadable")?;
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        if !std::fs::symlink_metadata(account.path())
            .map_err(|_| "ProtectedStateUnreadable")?
            .is_dir()
        {
            return Err("ProtectedStateUnreadable");
        }
        ids.push(id);
    }
    ids.sort();
    for id in ids {
        roots.push((
            format!("cloud-{}", stutter_core::hash::sha256_hex(id.as_bytes())),
            userdata.join(&id).join(APP_ID.to_string()).join("remote"),
        ));
    }
    Ok(roots)
}

/// Walk a root, adding one line per entry to the digest. Links are recorded by their target text and
/// never followed.
fn walk(
    dir: &Path,
    relative: &str,
    lines: &mut BTreeMap<String, String>,
    bytes: &mut u64,
    budget: &mut u64,
) -> Result<(), &'static str> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|_| "ProtectedStateUnreadable")?
        .collect::<Result<_, _>>()
        .map_err(|_| "ProtectedStateUnreadable")?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if lines.len() >= MAX_ENTRIES {
            return Err("ProtectedStateTooLarge");
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let relative = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        let meta = std::fs::symlink_metadata(&path).map_err(|_| "ProtectedStateUnreadable")?;
        let kind = meta.file_type();
        if kind.is_symlink() {
            let target = std::fs::read_link(&path).map_err(|_| "ProtectedStateUnreadable")?;
            lines.insert(relative, format!("L:{}", target.display()));
        } else if kind.is_dir() {
            lines.insert(relative.clone(), "D".to_owned());
            walk(&path, &relative, lines, bytes, budget)?;
        } else if kind.is_file() {
            if meta.len() > MAX_FILE_BYTES || meta.len() > *budget {
                return Err("ProtectedStateTooLarge");
            }
            *budget -= meta.len();
            *bytes += meta.len();
            let content = std::fs::read(&path).map_err(|_| "ProtectedStateUnreadable")?;
            // The file changed size while it was read: not a stable state to fingerprint.
            if content.len() as u64 != meta.len() {
                return Err("ProtectedStateUnreadable");
            }
            lines.insert(
                relative,
                format!(
                    "F:{}:{}",
                    meta.len(),
                    stutter_core::hash::sha256_hex(&content)
                ),
            );
        } else {
            lines.insert(relative, "O".to_owned());
        }
    }
    Ok(())
}

/// Fingerprint the protected directories under `steamapps`, read-only.
pub fn capture(steamapps: &Path) -> Result<Snapshot, &'static str> {
    let mut budget = MAX_TOTAL_BYTES;
    let mut prints = Vec::new();
    for (label, root) in roots(steamapps)? {
        match std::fs::symlink_metadata(&root) {
            // A root that is absent is part of the state: its appearance later is a change.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => prints.push(RootPrint {
                label,
                present: false,
                entries: 0,
                bytes: 0,
                sha256: stutter_core::hash::sha256_hex(b"absent"),
            }),
            Err(_) => return Err("ProtectedStateUnreadable"),
            Ok(meta) if meta.is_dir() => {
                crate::syscall::open_dir_no_symlinks(&root)
                    .map_err(|_| "ProtectedStateUnreadable")?;
                let (mut lines, mut bytes) = (BTreeMap::new(), 0u64);
                walk(&root, "", &mut lines, &mut bytes, &mut budget)?;
                let text: String = lines
                    .iter()
                    .map(|(path, line)| format!("{path}\0{line}\n"))
                    .collect();
                prints.push(RootPrint {
                    label,
                    present: true,
                    entries: lines.len() as u64,
                    bytes,
                    sha256: stutter_core::hash::sha256_hex(text.as_bytes()),
                });
            }
            // A root that is not a directory (a link or a file) is not what is expected there.
            Ok(_) => return Err("ProtectedStateUnreadable"),
        }
    }
    Ok(Snapshot {
        schema: 1,
        roots: prints,
    })
}

/// Whether `after` is the same protected state as `before`, naming the first root that differs.
pub fn compare(before: &Snapshot, after: &Snapshot) -> Result<(), &'static str> {
    if before.schema != 1 || after.schema != 1 {
        return Err("ProtectedStateSchemaUnsupported");
    }
    if before.roots != after.roots {
        return Err("ProtectedStateChanged");
    }
    Ok(())
}

/// Record the fingerprint before any launch, beside the study. An existing record is kept (a restarted
/// campaign must compare with the state it first saw, not with a later one); one that cannot be made
/// is an error, so nothing launches over data that was never fingerprinted.
pub fn record_baseline(
    steamapps: &Path,
    study_dir: &std::fs::File,
    uid: u32,
    campaign: &str,
) -> anyhow::Result<()> {
    let file = super::report_store::baseline_name("protected-state", campaign)?;
    match read_private(study_dir, &file, uid) {
        Ok(bytes) => {
            serde_json::from_slice::<Snapshot>(&bytes)?;
            return Ok(());
        }
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_none_or(|io| io.kind() != std::io::ErrorKind::NotFound) =>
        {
            return Err(error);
        }
        Err(_) => {}
    }
    let snapshot = capture(steamapps).map_err(|reason| anyhow::anyhow!(reason))?;
    write_new(study_dir, &file, &serde_json::to_vec_pretty(&snapshot)?)
}

/// Seal, once and write-once, that the protected state was unchanged when the campaign was evaluated: a later change
/// to the player's saves cannot then rewrite what the campaign itself did. Nothing is sealed unless the state is
/// unchanged right now.
pub fn seal_unchanged(
    steamapps: &Path,
    study_dir: &std::fs::File,
    uid: u32,
    campaign: &str,
    now: u64,
) -> Result<(), &'static str> {
    status(steamapps, study_dir, uid, campaign)?;
    let file = super::report_store::baseline_name("protected-state-final", campaign)
        .map_err(|_| "CampaignIdentityInvalid")?;
    let record = serde_json::json!({ "schema": 1, "unchanged": true, "sealed_unix": now });
    match write_new(
        study_dir,
        &file,
        &serde_json::to_vec_pretty(&record).map_err(|_| "ProtectedStateSealFailed")?,
    ) {
        Ok(()) => Ok(()),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            Ok(())
        }
        Err(_) => Err("ProtectedStateSealFailed"),
    }
}

/// Whether the unchanged protected state was sealed at finalization (`seal_unchanged`), whatever the saves look
/// like now.
pub fn sealed(study_dir: &std::fs::File, uid: u32, campaign: &str) -> Result<(), &'static str> {
    let file = super::report_store::baseline_name("protected-state-final", campaign)
        .map_err(|_| "CampaignIdentityInvalid")?;
    let value: serde_json::Value = read_private(study_dir, &file, uid)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("ProtectedStateNotSealedAtFinalization")?;
    (value["schema"] == 1 && value["unchanged"] == true)
        .then_some(())
        .ok_or("ProtectedStateNotSealedAtFinalization")
}

/// The protected state now against the baseline recorded before the campaign: `Ok` when unchanged, the
/// typed reason otherwise (`ProtectedStateNotRecorded` when no baseline exists).
pub fn status(
    steamapps: &Path,
    study_dir: &std::fs::File,
    uid: u32,
    campaign: &str,
) -> Result<(), &'static str> {
    let file = super::report_store::baseline_name("protected-state", campaign)
        .map_err(|_| "CampaignIdentityInvalid")?;
    let before: Snapshot = read_private(study_dir, &file, uid)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("ProtectedStateNotRecorded")?;
    compare(&before, &capture(steamapps)?)
}

#[cfg(test)]
#[path = "protected_state_tests.rs"]
mod tests;
