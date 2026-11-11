use std::{
    collections::BTreeSet,
    fs::File,
    os::{fd::AsFd, unix::fs::MetadataExt},
    path::Path,
};

use super::{
    super::vdf::VdfDocument,
    MAX_PROC_CMDLINE, MAX_STEAM_CLIENT_FILE, SteamClientIdentity, SteamProcessIdentity,
    tool::{read_trusted_file, string, valid_steam_branch},
};

pub(super) fn stable_proton_version(name: &str) -> Option<&str> {
    let version = name.strip_prefix("Proton ")?;
    (!version.is_empty()
        && version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.'))
    .then_some(version)
}

pub(super) fn steam_webhelper_binding(bytes: &[u8]) -> anyhow::Result<Option<(u32, u64, bool)>> {
    anyhow::ensure!(
        bytes.len() <= MAX_PROC_CMDLINE,
        "Steam command line too large"
    );
    let args = bytes
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .map(std::str::from_utf8)
        .collect::<Result<Vec<_>, _>>()?;
    if !args
        .first()
        .is_some_and(|arg| arg.ends_with("steamwebhelper"))
    {
        return Ok(None);
    }
    let steam_pid = args
        .iter()
        .find_map(|arg| arg.strip_prefix("-steampid="))
        .and_then(|value| value.parse::<u32>().ok());
    let build_id = args
        .iter()
        .find_map(|arg| arg.strip_prefix("-buildid="))
        .and_then(|value| value.parse::<u64>().ok());
    let logged_in = args
        .iter()
        .find_map(|arg| arg.strip_prefix("-steamid="))
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|steam_id| steam_id != 0);
    match (steam_pid, build_id) {
        (Some(pid), Some(build)) if pid != 0 && build != 0 => Ok(Some((pid, build, logged_in))),
        _ => Ok(None),
    }
}

pub(super) fn inspect_steam_process(
    proc_root: &Path,
    steam_root: &Path,
    owner_uid: u32,
    expected_build_id: u64,
) -> anyhow::Result<Option<SteamProcessIdentity>> {
    let proc_entries = std::fs::read_dir(proc_root)?;
    let expected_steam_exe = std::fs::canonicalize(steam_root.join("steamrt64/steam"))?;
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    for entry in proc_entries {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        scanned += 1;
        anyhow::ensure!(scanned <= 65_536, "/proc process scan exceeds limit");
        let process_dir = proc_root.join(pid.to_string());
        let helper_exe = match std::fs::read_link(process_dir.join("exe")) {
            Ok(path) => path,
            Err(_) => continue,
        };
        if helper_exe
            .file_name()
            .is_none_or(|name| name != "steamwebhelper")
            || !helper_exe.starts_with(steam_root)
        {
            continue;
        }
        let cmdline = match std::fs::read(process_dir.join("cmdline")) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let Some((steam_pid, build_id, logged_in)) = steam_webhelper_binding(&cmdline)? else {
            continue;
        };
        anyhow::ensure!(
            build_id == expected_build_id,
            "running Steam Experimental build differs from its installed manifest"
        );
        let helper = crate::telemetry::proc_view::live_process(proc_root, pid)
            .ok_or_else(|| anyhow::anyhow!("Steam webhelper exited during identity read"))?;
        anyhow::ensure!(
            helper.uid == owner_uid && helper.start_ticks != 0,
            "Steam webhelper owner or start identity mismatch"
        );
        let steam_exe = std::fs::canonicalize(proc_root.join(steam_pid.to_string()).join("exe"))?;
        anyhow::ensure!(
            steam_exe == expected_steam_exe,
            "Steam webhelper parent is not the selected Steam installation"
        );
        let steam = crate::telemetry::proc_view::live_process(proc_root, steam_pid)
            .ok_or_else(|| anyhow::anyhow!("Steam client exited during identity read"))?;
        anyhow::ensure!(
            steam.uid == owner_uid && steam.start_ticks != 0,
            "Steam client owner or start identity mismatch"
        );
        let helper_after = crate::telemetry::proc_view::live_process(proc_root, pid)
            .ok_or_else(|| anyhow::anyhow!("Steam webhelper exited during identity read"))?;
        anyhow::ensure!(
            helper.start_ticks == helper_after.start_ticks,
            "Steam webhelper changed during identity read"
        );
        matches.push(SteamProcessIdentity {
            steam_pid,
            steam_start_ticks: steam.start_ticks,
            webhelper_pid: pid,
            webhelper_start_ticks: helper.start_ticks,
            logged_in,
        });
    }
    matches.sort_by_key(|identity| identity.steam_pid);
    matches.dedup();
    anyhow::ensure!(
        matches.len() <= 1,
        "multiple Steam Experimental clients are running"
    );
    Ok(matches.pop())
}

pub(super) fn inspect_steam_client_identity(
    steamapps: &Path,
    owner_uid: u32,
) -> anyhow::Result<Option<SteamClientIdentity>> {
    let Some(steam_root) = steamapps.parent() else {
        return Ok(None);
    };
    let root = crate::syscall::open_dir_no_symlinks(steam_root)?;
    anyhow::ensure!(
        File::from(root.try_clone()?).metadata()?.uid() == owner_uid,
        "Steam root owner mismatch"
    );
    let package = match crate::syscall::openat_dir_nofollow(root.as_fd(), "package") {
        Ok(dir) => dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(
        File::from(package.try_clone()?).metadata()?.uid() == owner_uid,
        "Steam package directory owner mismatch"
    );
    let beta = read_trusted_file(package.as_fd(), "beta", owner_uid, 128, false)?;
    let branch = std::str::from_utf8(&beta.bytes)?.trim();
    let branch = if branch.is_empty() { "public" } else { branch };
    anyhow::ensure!(valid_steam_branch(branch), "invalid Steam client branch");
    let manifest_name = format!("steam_client_{branch}_steamrt64.manifest");
    let installed_name = format!("steam_client_{branch}_steamrt64.installed");
    let manifest = read_trusted_file(
        package.as_fd(),
        &manifest_name,
        owner_uid,
        MAX_STEAM_CLIENT_FILE,
        false,
    )?;
    let installed = read_trusted_file(
        package.as_fd(),
        &installed_name,
        owner_uid,
        MAX_STEAM_CLIENT_FILE,
        false,
    )?;
    let documents = VdfDocument::parse_all(&manifest.bytes)?;
    anyhow::ensure!(
        documents.len() == 3
            && documents
                .iter()
                .map(|document| document.name.as_str())
                .collect::<BTreeSet<_>>()
                == BTreeSet::from(["steamrt64", "kvsign2", "kvsignatures"]),
        "unexpected Steam client manifest roots"
    );
    let document = documents
        .iter()
        .find(|document| document.name == "steamrt64")
        .ok_or_else(|| anyhow::anyhow!("Steam Runtime manifest root is missing"))?;
    let signing = documents
        .iter()
        .find(|document| document.name == "kvsign2")
        .and_then(|document| document.root_object().ok())
        .and_then(|root| string(root, "steamrt64"));
    let signature = documents
        .iter()
        .find(|document| document.name == "kvsignatures")
        .and_then(|document| document.root_object().ok())
        .and_then(|root| string(root, "steamrt64"));
    anyhow::ensure!(
        signing.is_some_and(|value| !value.is_empty())
            && signature.is_some_and(|value| !value.is_empty()),
        "Steam client manifest signature records are incomplete"
    );
    let root = document.root_object()?;
    let version = string(root, "version")
        .ok_or_else(|| anyhow::anyhow!("Steam client manifest lacks version"))?;
    anyhow::ensure!(
        !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit()),
        "invalid Steam client manifest version"
    );
    let steamrt64_build_id = version.parse::<u64>()?;
    anyhow::ensure!(steamrt64_build_id != 0, "invalid Steam Runtime build ID");
    let process = inspect_steam_process(
        Path::new("/proc"),
        steam_root,
        owner_uid,
        steamrt64_build_id,
    )?;
    let identity = serde_json::json!({
        "schema_version": 1,
        "branch": branch,
        "steamrt64_build_id": steamrt64_build_id,
        "steamrt64_manifest_sha256": manifest.sha256,
        "installed_index_sha256": installed.sha256,
    });
    Ok(Some(SteamClientIdentity {
        branch: branch.to_owned(),
        steamrt64_build_id,
        steamrt64_manifest_sha256: manifest.sha256,
        installed_index_sha256: installed.sha256,
        identity_sha256: stutter_core::hash::sha256_hex(&serde_json::to_vec(&identity)?),
        process,
    }))
}
