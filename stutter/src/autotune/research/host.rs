//! Independent, read-only host facts for resident Cyberpunk research admission.
//!
//! Caller-supplied campaign JSON is never used as a source of host identity. This collector runs
//! off the monitor event loop and returns only stable hashes and closed identity fields.

use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use super::{ResearchTool, SteamClientIdentity};
use crate::{
    autotune::{research::CampaignFacts, runtime::AutotuneRuntimeConfig},
    daemon::config::autotune::DaemonAutotuneConfig,
};

const MAX_BINARY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_LOGINCTL_OUTPUT: usize = 4096;
const LOGINCTL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostFactsError {
    pub code: &'static str,
}

impl HostFactsError {
    const fn new(code: &'static str) -> Self {
        Self { code }
    }
}

fn valid_source_revision(revision: &str) -> bool {
    revision.len() == 40
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn source_identity() -> anyhow::Result<String> {
    let revision = env!("STUTTER_GIT_REV");
    anyhow::ensure!(
        valid_source_revision(revision),
        "Stutter binary has no full embedded Git revision"
    );
    let executable = std::env::current_exe()?;
    anyhow::ensure!(
        executable.starts_with("/nix/store"),
        "Stutter binary is outside the immutable Nix store"
    );
    let file = File::open(&executable)?;
    let before = file.metadata()?;
    anyhow::ensure!(
        before.is_file()
            && before.uid() == 0
            && before.mode() & 0o222 == 0
            && before.len() <= MAX_BINARY_BYTES,
        "Stutter binary is not immutable root-owned code"
    );
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&file).take(MAX_BINARY_BYTES + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    anyhow::ensure!(
        bytes.len() as u64 == before.len()
            && before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec(),
        "Stutter binary changed while its identity was read"
    );
    let material = serde_json::json!({
        "schema_version": 1,
        "git_revision": revision,
        "binary_sha256": stutter_core::hash::sha256_hex(&bytes),
    });
    Ok(stutter_core::hash::sha256_hex(&serde_json::to_vec(
        &material,
    )?))
}

fn valid_session_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn parse_session_properties(text: &str, requested_id: &str, uid: u32) -> anyhow::Result<String> {
    let mut fields = BTreeMap::new();
    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("malformed loginctl session properties"))?;
        anyhow::ensure!(
            fields.insert(key, value).is_none(),
            "duplicate session property"
        );
    }
    anyhow::ensure!(
        fields.get("Id") == Some(&requested_id)
            && fields
                .get("User")
                .and_then(|value| value.parse::<u32>().ok())
                == Some(uid)
            && fields.get("Active") == Some(&"yes")
            && fields.get("Remote") == Some(&"no")
            && fields.get("Type") == Some(&"wayland")
            && fields.get("State") == Some(&"active")
            && fields.get("Seat").is_some_and(|seat| !seat.is_empty()),
        "desktop session is not the active local Wayland session"
    );
    Ok(requested_id.to_owned())
}

fn loginctl_query(args: &[&str]) -> anyhow::Result<String> {
    let loginctl = Path::new("/run/current-system/sw/bin/loginctl");
    let canonical = std::fs::canonicalize(loginctl)?;
    anyhow::ensure!(
        canonical.starts_with("/nix/store"),
        "loginctl is not NixOS-managed"
    );
    let executable = std::fs::metadata(&canonical)?;
    anyhow::ensure!(
        executable.is_file() && executable.uid() == 0 && executable.mode() & 0o222 == 0,
        "loginctl is not immutable root-owned code"
    );
    let mut child = Command::new(loginctl)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + LOGINCTL_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("loginctl session query timed out");
        }
        thread::sleep(Duration::from_millis(10));
    };
    anyhow::ensure!(status.success(), "loginctl session query failed");
    let output = child.wait_with_output()?;
    anyhow::ensure!(
        output.stdout.len() <= MAX_LOGINCTL_OUTPUT,
        "loginctl response exceeds limit"
    );
    Ok(std::str::from_utf8(&output.stdout)?.to_owned())
}

fn session_ids(text: &str, uid: u32) -> anyhow::Result<Vec<String>> {
    let mut ids = Vec::new();
    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        let id = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing session ID"))?;
        let owner = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing session UID"))?
            .parse::<u32>()?;
        anyhow::ensure!(
            valid_session_id(id) && fields.next().is_some(),
            "invalid session listing"
        );
        if owner == uid {
            anyhow::ensure!(
                !ids.iter().any(|existing| existing == id),
                "duplicate session ID"
            );
            ids.push(id.to_owned());
            anyhow::ensure!(ids.len() <= 32, "too many user sessions");
        }
    }
    Ok(ids)
}

fn active_wayland_session(uid: u32) -> anyhow::Result<String> {
    let listing = loginctl_query(&["--no-pager", "--no-legend", "list-sessions"])?;
    let mut active = Vec::new();
    for session in session_ids(&listing, uid)? {
        let properties = loginctl_query(&[
            "--no-pager",
            "show-session",
            &session,
            "--property=Id",
            "--property=User",
            "--property=Active",
            "--property=Remote",
            "--property=Type",
            "--property=State",
            "--property=Seat",
        ])?;
        if let Ok(id) = parse_session_properties(&properties, &session, uid) {
            active.push(id);
        }
    }
    anyhow::ensure!(
        active.len() == 1,
        "active local Wayland session is missing or ambiguous"
    );
    Ok(active.remove(0))
}

fn configured_paths(config: &DaemonAutotuneConfig) -> Result<(PathBuf, PathBuf), HostFactsError> {
    match (
        config.research_steamapps_dir.as_deref(),
        config.research_proton_manifest.as_deref(),
    ) {
        (Some(steamapps), Some(manifest)) if steamapps.is_absolute() && manifest.is_absolute() => {
            Ok((steamapps.to_owned(), manifest.to_owned()))
        }
        _ => Err(HostFactsError::new("BlockedResearchConfiguration")),
    }
}

/// Rebuild live identity from installed, read-only host state. A failure returns a short typed
/// reason suitable for IPC; path-rich details never enter the user-facing response.
pub fn collect_campaign_facts(
    runtime: &AutotuneRuntimeConfig,
    validation_override_present: bool,
    mutable_action_present: bool,
) -> Result<CampaignFacts, HostFactsError> {
    let uid = crate::syscall::geteuid();
    if uid == 0 {
        return Err(HostFactsError::new("BlockedWrongUser"));
    }
    let (steamapps, proton_manifest) = configured_paths(&runtime.daemon_config.autotune)?;
    let source_sha256 =
        source_identity().map_err(|_| HostFactsError::new("BlockedSourceIdentity"))?;
    let policy_sha256 = stutter_core::hash::sha256_hex(
        &serde_json::to_vec(&runtime.daemon_policy)
            .map_err(|_| HostFactsError::new("BlockedPolicyIdentity"))?,
    );
    let boot_id = crate::telemetry::identity::read_boot_id()
        .map_err(|_| HostFactsError::new("BlockedHostIdentity"))?;
    if boot_id.is_empty() {
        return Err(HostFactsError::new("BlockedHostIdentity"));
    }
    let session_id =
        active_wayland_session(uid).map_err(|_| HostFactsError::new("BlockedDesktopSession"))?;
    let libraries = crate::autotune::research::discover_steam_libraries(&steamapps, uid)
        .map_err(|_| HostFactsError::new("BlockedSteamLibraries"))?;
    let (_, game) = crate::autotune::research::discover_cyberpunk_installation(&libraries, uid)
        .map_err(|_| HostFactsError::new("BlockedGameInstallation"))?;
    if game.manifest_update_status
        != crate::autotune::research::SteamUpdateStatus::NoPendingUpdateIndicated
    {
        return Err(HostFactsError::new("BlockedGameUpdate"));
    }
    let game_executable_sha256 = game
        .game_executable_sha256
        .ok_or_else(|| HostFactsError::new("BlockedGameInstallation"))?;
    let proton = crate::autotune::research::inspect_proton_inventory(
        &proton_manifest,
        &steamapps,
        &libraries,
        uid,
    )
    .map_err(|_| HostFactsError::new("BlockedProtonInventory"))?;
    let tools: Vec<ResearchTool> = proton.tools();
    let steam_client: SteamClientIdentity = proton
        .steam_client
        .clone()
        .ok_or_else(|| HostFactsError::new("BlockedSteamClientIdentity"))?;
    if steam_client.branch != "publicbeta" || steam_client.process.is_none() {
        return Err(HostFactsError::new("BlockedSteamClientIdentity"));
    }
    if !steam_client
        .process
        .as_ref()
        .is_some_and(|process| process.logged_in)
    {
        return Err(HostFactsError::new("BlockedSteamLogin"));
    }
    if tools.len() < 2 {
        return Err(HostFactsError::new("BlockedProtonSources"));
    }
    Ok(CampaignFacts {
        uid,
        boot_id,
        session_id,
        steam_client,
        source_sha256,
        policy_sha256,
        game_executable_sha256,
        build_id: game.build_id,
        tools,
        validation_override_present,
        mutable_action_present,
    })
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
