//! Root-owned ownership receipt for campaign-installed units and configuration.
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Serialize, Deserialize)]
struct Receipt {
    campaign: String,
    binary_sha256: String,
    files: std::collections::BTreeMap<PathBuf, String>,
}

fn directory(auth: &Authorization) -> PathBuf {
    crate::process::service_bin_dir(auth)
}

pub(super) fn binary(auth: &Authorization) -> PathBuf {
    directory(auth).join("stutter")
}

pub(super) fn paths(user: &str) -> Vec<PathBuf> {
    let mut files = vec![
        "/run/systemd/system/stutter-autotune-bounded@.service".into(),
        "/run/systemd/system/stutter-privileged-worker@.service".into(),
        fixture::config_path(user),
    ];
    let profiles = fixture::config_path(user).with_extension("profiles.toml");
    if profiles.exists() {
        files.push(profiles);
    }
    files
}

pub(super) fn record(auth: &Authorization, user: &str) -> Result<(), Denial> {
    let mut files = std::collections::BTreeMap::new();
    for path in paths(user) {
        protected_directory(&path)?;
        files.insert(
            path.clone(),
            sha256_hex(&fs::read(path).map_err(|error| failure(error.to_string()))?),
        );
    }
    let dir = directory(auth);
    protected_directory(&dir)?;
    let receipt = Receipt {
        campaign: auth.campaign_id.clone(),
        binary_sha256: auth.stutter_sha256.clone(),
        files,
    };
    atomic_write(
        &dir.join("ownership.json"),
        &serde_json::to_vec_pretty(&receipt).map_err(|error| failure(error.to_string()))?,
    )
}

pub(super) fn verify(auth: &Authorization, user: &str) -> Result<(), Denial> {
    let dir = directory(auth);
    let path = dir.join("ownership.json");
    protected_directory(&path)?;
    let receipt: Receipt =
        serde_json::from_slice(&fs::read(path).map_err(|error| failure(error.to_string()))?)
            .map_err(|error| failure(error.to_string()))?;
    if receipt.campaign != auth.campaign_id
        || receipt.binary_sha256 != auth.stutter_sha256
        || receipt
            .files
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            != paths(user).into_iter().collect()
    {
        return Err(failure("bounded service campaign ownership differs"));
    }
    protected_directory(&dir.join("stutter"))?;
    verify_pinned(&dir.join("stutter"), &auth.stutter_sha256)?;
    protected_directory(&dir.join("xtask"))?;
    verify_pinned(&dir.join("xtask"), &auth.xtask_sha256)?;
    for (path, expected) in receipt.files {
        protected_directory(&path)?;
        verify_pinned(&path, &expected)?;
    }
    // Reject loaded drop-ins or a replacement fragment, even when the template file survived.
    for component in [ServiceComponent::Controller, ServiceComponent::Worker] {
        let unit = unit_name(user, component)?;
        let expected = match component {
            ServiceComponent::Controller => "/run/systemd/system/stutter-autotune-bounded@.service",
            ServiceComponent::Worker => "/run/systemd/system/stutter-privileged-worker@.service",
        };
        let fragment = systemctl(&["show", "--property=FragmentPath", "--value", &unit])?;
        let overrides = systemctl(&["show", "--property=DropInPaths", "--value", &unit])?;
        if fragment.trim() != expected || !overrides.trim().is_empty() {
            return Err(failure(
                "bounded service fragment or drop-ins changed outside campaign ownership",
            ));
        }
    }
    Ok(())
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Denial> {
    let dir = path
        .parent()
        .ok_or_else(|| failure("missing protected parent"))?;
    protected_directory(dir)?;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| failure(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| failure(error.to_string()))?;
    fs::set_permissions(&temp, fs::Permissions::from_mode(0o644))
        .map_err(|error| failure(error.to_string()))?;
    fs::rename(&temp, path).map_err(|error| failure(error.to_string()))?;
    fs::File::open(dir)
        .and_then(|file| file.sync_all())
        .map_err(|error| failure(error.to_string()))
}
