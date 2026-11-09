//! The runner's side of the offline-authorization receipt.
//!
//! A privileged campaign that needs an offline bus adapter (a scoped power-profile hold, a
//! scheduler switch) gets a *receipt*: a small JSON file the runner writes, as root, into its own
//! state directory. The product library's `campaign_authorization` verifies it (every path
//! component root-owned and not writable by others, the boot, the user, the scope, the expiry)
//! and journals the issue before any adapter runs. Only a root process can write where that check
//! accepts a receipt, so an unprivileged caller can name a campaign but never authorize one.
//!
//! The field names here are the contract the library parses.
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde_json::json;

/// Scopes a receipt may grant, the same names the library accepts.
pub const SCOPES: [&str; 3] = ["power_profile_hold", "scheduler_switch", "unit_properties"];

/// Longest a receipt may live; a campaign that needs longer is issued a new one.
pub const MAX_LIFETIME_SECONDS: u64 = 6 * 60 * 60;

fn safe_campaign(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 120
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// Writes `<state_root>/receipts/<campaign>.json` atomically (temporary file, sync, rename) with
/// mode 0644 and returns its path. The directory is created 0755. Refuses an unsafe campaign id,
/// an unknown scope, an empty boot id and a lifetime over [`MAX_LIFETIME_SECONDS`].
pub fn write_receipt(
    state_root: &Path,
    campaign_id: &str,
    owner_uid: u32,
    scopes: &[&str],
    boot_id: &str,
    now_unix: u64,
    lifetime_seconds: u64,
) -> io::Result<PathBuf> {
    let bad = |message: &str| io::Error::new(io::ErrorKind::InvalidInput, message.to_owned());
    if !safe_campaign(campaign_id) {
        return Err(bad("unsafe campaign id"));
    }
    if scopes.is_empty() || scopes.iter().any(|scope| !SCOPES.contains(scope)) {
        return Err(bad("unknown or empty scope"));
    }
    if boot_id.trim().is_empty() {
        return Err(bad("empty boot id"));
    }
    if lifetime_seconds == 0 || lifetime_seconds > MAX_LIFETIME_SECONDS {
        return Err(bad("lifetime out of range"));
    }
    let dir = state_root.join("receipts");
    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755))?;
    let body = json!({
        "campaign_id": campaign_id,
        "scopes": scopes,
        "owner_uid": owner_uid,
        "boot_id": boot_id.trim(),
        "expires_unix": now_unix.saturating_add(lifetime_seconds),
    });
    let temporary = dir.join(format!(".{campaign_id}.json.tmp"));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o644)
        .open(&temporary)?;
    file.write_all(body.to_string().as_bytes())?;
    file.sync_all()?;
    // The authoritative issue record lives with the root-owned receipts, not with the caller: it is
    // appended and synced before the receipt becomes visible, so a receipt never exists unrecorded.
    let journal_line = format!(
        "{}\n",
        json!({
            "campaign": campaign_id,
            "owner_uid": owner_uid,
            "scopes": scopes,
            "boot_id": boot_id.trim(),
            "issued_unix": now_unix,
            "expires_unix": now_unix.saturating_add(lifetime_seconds),
        })
    );
    let recorded = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o644)
        .open(dir.join("issued.jsonl"))
        .and_then(|mut journal| {
            journal.write_all(journal_line.as_bytes())?;
            journal.sync_all()
        });
    if let Err(error) = recorded {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let target = dir.join(format!("{campaign_id}.json"));
    fs::rename(&temporary, &target)?;

    fs::File::open(&dir)?.sync_all()?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use super::*;

    fn root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("stutter-rcpt-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_receipt_is_written_atomically_with_the_fields_the_library_parses() {
        let state = root("ok");
        let path = write_receipt(
            &state,
            "w19-power",
            1000,
            &["power_profile_hold"],
            "boot-1\n",
            5_000,
            600,
        )
        .unwrap();
        assert_eq!(path, state.join("receipts/w19-power.json"));
        let meta = fs::metadata(&path).unwrap();
        assert_eq!(
            meta.mode() & 0o777,
            0o644,
            "readable, never group or world writable"
        );
        assert_eq!(
            fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
            0o755
        );
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["campaign_id"], "w19-power");
        assert_eq!(value["scopes"][0], "power_profile_hold");
        assert_eq!(value["owner_uid"], 1000);
        assert_eq!(value["boot_id"], "boot-1");
        assert_eq!(value["expires_unix"], 5_600);
        assert!(!state.join("receipts/.w19-power.json.tmp").exists());
        let issued = fs::read_to_string(state.join("receipts/issued.jsonl")).unwrap();
        assert_eq!(issued.lines().count(), 1);
        assert!(
            issued.contains("w19-power") && issued.contains("power_profile_hold"),
            "{issued}"
        );
    }

    #[test]
    fn a_receipt_is_never_visible_without_its_authoritative_journal_line() {
        let state = root("journal-fail");
        fs::create_dir_all(state.join("receipts/issued.jsonl")).unwrap();
        let result = write_receipt(&state, "w19", 1000, &["scheduler_switch"], "b", 1, 60);
        assert!(result.is_err(), "the journal could not be appended");
        assert!(
            !state.join("receipts/w19.json").exists(),
            "a receipt appeared unrecorded"
        );
    }

    #[test]
    fn unsafe_ids_unknown_scopes_and_bad_lifetimes_are_refused_and_write_nothing() {
        let state = root("bad");
        for (id, scopes, boot, life) in [
            ("../escape", vec!["power_profile_hold"], "b", 60),
            ("", vec!["power_profile_hold"], "b", 60),
            ("ok", vec!["root_shell"], "b", 60),
            ("ok", vec![], "b", 60),
            ("ok", vec!["power_profile_hold"], "  ", 60),
            ("ok", vec!["power_profile_hold"], "b", 0),
            (
                "ok",
                vec!["power_profile_hold"],
                "b",
                MAX_LIFETIME_SECONDS + 1,
            ),
        ] {
            assert!(
                write_receipt(&state, id, 1000, &scopes, boot, 1, life).is_err(),
                "{id:?} {scopes:?} {boot:?} {life}"
            );
        }
        assert!(
            !state.join("receipts").exists()
                || fs::read_dir(state.join("receipts")).unwrap().count() == 0
        );
    }
}
