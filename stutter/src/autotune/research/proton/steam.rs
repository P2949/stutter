use std::{
    fs::File,
    io::Read,
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::MetadataExt,
    },
    path::{Component, Path},
};

use super::{
    super::vdf::VdfDocument,
    EXPERIMENTAL_APP_ID, MAX_LAB_MANIFEST, MAX_STEAM_MANIFEST, MAX_STEAM_MANIFEST_COUNT,
    MAX_STEAM_MANIFEST_SCAN_BYTES, ProtonBlockReason, ProtonCandidate, ProtonEligibility,
    ProtonInventory, ProtonSource, ResearchTool, SteamUpdateStatus,
    custom::custom_candidates,
    steam_client::{inspect_steam_client_identity, stable_proton_version},
    tool::{ToolSpec, inspect_tool_directory, read_trusted_file, string},
};

struct SteamAppManifest {
    app_id: u32,
    name: String,
    install_dir: String,
    build_id: u64,
    update_status: SteamUpdateStatus,
    sha256: String,
    bytes: u64,
}

fn read_steam_appmanifest(
    steamapps: &Path,
    owner_uid: u32,
    app_id: u32,
) -> anyhow::Result<Option<SteamAppManifest>> {
    let dir = crate::syscall::open_dir_no_symlinks(steamapps)?;
    anyhow::ensure!(
        File::from(dir.try_clone()?).metadata()?.uid() == owner_uid,
        "SteamApps directory owner mismatch"
    );
    let path = format!("appmanifest_{app_id}.acf");
    let file = match crate::syscall::openat_nofollow(dir.as_fd(), &path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let before = file.metadata()?;
    anyhow::ensure!(
        before.is_file()
            && before.uid() == owner_uid
            && before.nlink() == 1
            && before.len() <= MAX_STEAM_MANIFEST,
        "untrusted Steam Proton manifest"
    );
    let mut bytes = Vec::new();
    (&file)
        .take(MAX_STEAM_MANIFEST + 1)
        .read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    anyhow::ensure!(
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec(),
        "Steam Proton manifest changed while reading"
    );
    let document = VdfDocument::parse(&bytes)?;
    anyhow::ensure!(
        document.name == "appstate",
        "unexpected Steam Proton manifest root"
    );
    let root = document.root_object()?;
    let number = |key: &str| {
        string(root, key).and_then(|value| {
            (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| value.parse::<u64>().ok())
                .flatten()
        })
    };
    anyhow::ensure!(
        number("appid") == Some(u64::from(app_id)),
        "Steam Proton AppID mismatch"
    );
    let name = string(root, "name").unwrap_or_default().to_owned();
    let install_dir = string(root, "installdir").unwrap_or_default().to_owned();
    let build_id = number("buildid").unwrap_or_default();
    anyhow::ensure!(build_id != 0, "Steam Proton build ID is missing");
    let state_flags = number("stateflags");
    let target = number("targetbuildid");
    let download = (number("bytestodownload"), number("bytesdownloaded"));
    let stage = (number("bytestostage"), number("bytesstaged"));
    let update = if target.is_some_and(|value| value != 0)
        || state_flags.is_some_and(|flags| flags & 2 != 0)
        || matches!(download, (Some(total), Some(done)) if total != done)
        || matches!(stage, (Some(total), Some(done)) if total != done)
    {
        SteamUpdateStatus::Updating
    } else if state_flags == Some(4)
        && target.is_some_and(|value| value == 0)
        && matches!(download, (Some(total), Some(done)) if total == done)
        && matches!(stage, (Some(total), Some(done)) if total == done)
    {
        SteamUpdateStatus::NoPendingUpdateIndicated
    } else {
        SteamUpdateStatus::Unknown
    };
    Ok(Some(SteamAppManifest {
        app_id,
        name,
        install_dir,
        build_id,
        update_status: update,
        sha256: stutter_core::hash::sha256_hex(&bytes),
        bytes: bytes.len() as u64,
    }))
}

fn installed_steam_app_ids(steamapps: &Path, owner_uid: u32) -> anyhow::Result<Vec<u32>> {
    let dir = crate::syscall::open_dir_no_symlinks(steamapps)?;
    let metadata = File::from(dir.try_clone()?).metadata()?;
    anyhow::ensure!(
        metadata.is_dir() && metadata.uid() == owner_uid,
        "SteamApps directory owner mismatch"
    );
    let path = format!("/proc/self/fd/{}", dir.as_raw_fd());
    let mut app_ids = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("SteamApps contains a non-UTF-8 entry"))?;
        let Some(id) = name
            .strip_prefix("appmanifest_")
            .and_then(|name| name.strip_suffix(".acf"))
        else {
            continue;
        };
        anyhow::ensure!(
            !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()),
            "invalid Steam app manifest name"
        );
        app_ids.push(id.parse::<u32>()?);
        anyhow::ensure!(
            app_ids.len() <= MAX_STEAM_MANIFEST_COUNT,
            "too many Steam app manifests"
        );
    }
    app_ids.sort_unstable();
    app_ids.dedup();
    Ok(app_ids)
}

pub(super) fn inspect_valve_stable(
    steamapps: &Path,
    owner_uid: u32,
) -> anyhow::Result<ProtonCandidate> {
    let mut installed = Vec::new();
    let mut scanned_bytes = 0u64;
    for app_id in installed_steam_app_ids(steamapps, owner_uid)? {
        let Some(manifest) = read_steam_appmanifest(steamapps, owner_uid, app_id)? else {
            continue;
        };
        scanned_bytes = scanned_bytes
            .checked_add(manifest.bytes)
            .ok_or_else(|| anyhow::anyhow!("Steam manifest scan size overflow"))?;
        anyhow::ensure!(
            scanned_bytes <= MAX_STEAM_MANIFEST_SCAN_BYTES,
            "Steam manifest scan exceeds size limit"
        );
        let Some(version) = stable_proton_version(&manifest.name) else {
            continue;
        };
        anyhow::ensure!(
            Path::new(&manifest.install_dir).components().count() == 1
                && Path::new(&manifest.install_dir)
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "unsafe Valve Proton Stable install directory"
        );
        let common = Path::new(steamapps).join("common");
        let common_fd = crate::syscall::open_dir_no_symlinks(&common)?;
        anyhow::ensure!(
            File::from(common_fd.try_clone()?).metadata()?.uid() == owner_uid,
            "Steam common directory owner mismatch"
        );
        let path = common.join(&manifest.install_dir);
        let provenance = serde_json::json!({
            "app_id": manifest.app_id,
            "build_id": manifest.build_id,
            "app_manifest_sha256": manifest.sha256,
        });
        let candidate = inspect_tool_directory(ToolSpec {
            source: ProtonSource::ValveStable,
            expected_id: "valve_stable",
            expected_version: version,
            compatibility_display_name: None,
            path: &path,
            owner_uid,
            immutable: false,
            provenance: &provenance,
            app_manifest_sha256: Some(&manifest.sha256),
        })?;
        installed.push((candidate, manifest.update_status));
    }
    match installed.len() {
        0 => Ok(ProtonCandidate::blocked(
            ProtonSource::ValveStable,
            ProtonBlockReason::NotInstalled,
        )),
        1 => {
            let (mut candidate, update) = installed.remove(0);
            if update != SteamUpdateStatus::NoPendingUpdateIndicated {
                candidate.eligibility = ProtonEligibility::Blocked(ProtonBlockReason::Updating);
            }
            Ok(candidate)
        }
        _ => {
            let mut identities: Vec<_> = installed
                .into_iter()
                .filter_map(|(candidate, _)| candidate.tool)
                .collect();
            identities.sort_by(|left, right| left.identity_sha256.cmp(&right.identity_sha256));
            let identity_sha256 = stutter_core::hash::sha256_hex(&serde_json::to_vec(&identities)?);
            Ok(ProtonCandidate {
                source: ProtonSource::ValveStable,
                version: None,
                tool: Some(ResearchTool {
                    source: ProtonSource::ValveStable,
                    identity_sha256,
                }),
                eligibility: ProtonEligibility::Blocked(ProtonBlockReason::AmbiguousStableBuild),
                path: None,
            })
        }
    }
}

/// Read the immutable Nix tool manifest and the fixed Steam Experimental AppID without changing
/// either installation. The caller supplies the controller-owned manifest path and the SteamApps
/// directory already bound to the authenticated UID.
pub(crate) fn inspect_proton_inventory(
    manifest_path: &Path,
    steamapps: &Path,
    owner_uid: u32,
) -> anyhow::Result<ProtonInventory> {
    anyhow::ensure!(owner_uid != 0, "Steam user must be unprivileged");
    anyhow::ensure!(
        manifest_path.parent() == Some(Path::new("/nix/store")),
        "Nix Proton manifest must be in /nix/store"
    );
    let store = crate::syscall::open_dir_no_symlinks(Path::new("/nix/store"))?;
    let manifest_name = manifest_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid Proton manifest name"))?;
    let manifest = read_trusted_file(store.as_fd(), manifest_name, 0, MAX_LAB_MANIFEST, true)?;
    let mut candidates = custom_candidates(&manifest.bytes, 0, Path::new("/nix/store"))?;

    let experimental = if let Some(manifest) =
        read_steam_appmanifest(steamapps, owner_uid, EXPERIMENTAL_APP_ID)?
    {
        anyhow::ensure!(
            manifest.name == "Proton Experimental"
                && manifest.install_dir == "Proton - Experimental"
                && Path::new(&manifest.install_dir).components().count() == 1
                && Path::new(&manifest.install_dir)
                    .components()
                    .all(|item| matches!(item, Component::Normal(_))),
            "unexpected Proton Experimental manifest"
        );
        let common = Path::new(steamapps).join("common");
        let common_fd = crate::syscall::open_dir_no_symlinks(&common)?;
        anyhow::ensure!(
            File::from(common_fd.try_clone()?).metadata()?.uid() == owner_uid,
            "Steam common directory owner mismatch"
        );
        let path = common.join(&manifest.install_dir);
        let provenance = serde_json::json!({
            "app_id": EXPERIMENTAL_APP_ID,
            "build_id": manifest.build_id,
        });
        let mut candidate = inspect_tool_directory(ToolSpec {
            source: ProtonSource::ValveExperimental,
            expected_id: "proton_experimental",
            expected_version: "experimental",
            compatibility_display_name: None,
            path: &path,
            owner_uid,
            immutable: false,
            provenance: &provenance,
            app_manifest_sha256: Some(&manifest.sha256),
        })?;
        if manifest.update_status != SteamUpdateStatus::NoPendingUpdateIndicated {
            candidate.eligibility = ProtonEligibility::Blocked(ProtonBlockReason::Updating);
        }
        candidate
    } else {
        ProtonCandidate::blocked(
            ProtonSource::ValveExperimental,
            ProtonBlockReason::NotInstalled,
        )
    };
    candidates.push(experimental);
    candidates.push(inspect_valve_stable(steamapps, owner_uid)?);
    let steam_client = inspect_steam_client_identity(steamapps, owner_uid)?;
    let inventory = ProtonInventory {
        schema_version: 1,
        candidates,
        steam_client,
    };
    inventory.validate()?;
    Ok(inventory)
}
