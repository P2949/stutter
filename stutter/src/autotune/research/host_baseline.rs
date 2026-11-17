//! A read-only comparison of the host the campaign was frozen against with the host as it is now
//! (PLAN G0). The intent freezes the boot, the game's build id and executable hash, the Proton
//! identities and the Steam client; this re-reads the Steam manifest, the game executable, the
//! Proton inventory and the signed-in account's launch record (changing nothing) and says whether
//! each still holds. A reboot, an update, a replaced executable, a changed Proton build or Steam
//! client, or a lapsed launch record since the campaign makes the baseline unattestable, and each is
//! its own blocker. It does not prove the original capture was right, only that the host has not
//! moved away from what was frozen.

use std::path::Path;

use super::{
    ResearchIntentV1, gate_verdict::HostFacts, proton::ProtonInventory,
    steam_inventory::SteamUpdateStatus,
};

/// Compare the frozen intent with the game installation and Proton inventory as read now, the
/// current boot and whether the account's launch record was found.
pub fn compare(
    intent: &ResearchIntentV1,
    current_boot_id: Option<&str>,
    game: &super::steam_inventory::CyberpunkInstallation,
    inventory: Option<&ProtonInventory>,
    entitlement_verified: bool,
) -> HostFacts {
    let installed = inventory.map(ProtonInventory::tools).unwrap_or_default();
    HostFacts {
        same_boot: current_boot_id == Some(intent.boot_id.as_str()),
        build_id_matches: game.build_id == intent.build_id,
        // An executable that could not be hashed is not the frozen one.
        game_executable_matches: game.game_executable_sha256.as_deref()
            == Some(intent.game_executable_sha256.as_str()),
        installation_settled: game.manifest_update_status
            == SteamUpdateStatus::NoPendingUpdateIndicated,
        // Every frozen tool must still be installed and eligible, exactly as frozen; an inventory
        // that could not be read matches nothing.
        proton_identities_match: inventory.is_some()
            && intent.tools.iter().all(|tool| installed.contains(tool)),
        steam_client_matches: inventory
            .and_then(|inventory| inventory.steam_client.as_ref())
            .is_some_and(|client| {
                client.branch == intent.steam_client.branch
                    && client.identity_sha256 == intent.steam_client.identity_sha256
            }),
        entitlement_verified,
    }
}

/// Re-read the Steam library, game installation, Proton inventory and account launch record under
/// `steamapps` and compare. `None` when the installation cannot be read (then nothing is attested);
/// an unreadable Proton inventory or launch record is reported as not matching, never skipped.
pub fn inspect(
    intent: &ResearchIntentV1,
    steamapps: &Path,
    proton_manifest: Option<&Path>,
    uid: u32,
) -> Option<HostFacts> {
    let libraries = super::discover_steam_libraries(steamapps, uid).ok()?;
    let (_, game) = super::discover_cyberpunk_installation(&libraries, uid).ok()?;
    let inventory = proton_manifest.and_then(|manifest| {
        super::inspect_proton_inventory(manifest, steamapps, &libraries, uid).ok()
    });
    let entitlement = match (
        steamapps.parent(),
        inventory
            .as_ref()
            .and_then(|inventory| inventory.steam_client.as_ref())
            .and_then(|client| client.process.as_ref()),
    ) {
        (Some(steam_root), Some(process)) => super::entitlement::authenticated_account_has_played(
            Path::new("/proc"),
            steam_root,
            process,
            intent.app_id,
        ),
        _ => false,
    };
    let boot = stutter_core::boot::current_boot_id().ok();
    Some(compare(
        intent,
        boot.as_deref(),
        &game,
        inventory.as_ref(),
        entitlement,
    ))
}

#[cfg(test)]
#[path = "host_baseline_tests.rs"]
mod tests;
