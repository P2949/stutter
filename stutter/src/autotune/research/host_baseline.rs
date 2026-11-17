//! A read-only comparison of the host the campaign was frozen against with the host as it is now
//! (PLAN G0). The intent freezes the boot, the game's build id and its executable's hash; this
//! re-reads the Steam manifest and the game executable (changing nothing) and says whether each
//! still holds. A reboot, an update or a replaced executable since the campaign makes the baseline
//! unattestable, and each is its own blocker. It does not prove the original capture was right, only
//! that the host has not moved away from what was frozen.

use std::path::Path;

use super::{ResearchIntentV1, gate_verdict::HostFacts, steam_inventory::SteamUpdateStatus};

/// Compare the frozen intent with the game installation as read now and the current boot.
pub fn compare(
    intent: &ResearchIntentV1,
    current_boot_id: Option<&str>,
    game: &super::steam_inventory::CyberpunkInstallation,
) -> HostFacts {
    HostFacts {
        same_boot: current_boot_id == Some(intent.boot_id.as_str()),
        build_id_matches: game.build_id == intent.build_id,
        // An executable that could not be hashed is not the frozen one.
        game_executable_matches: game.game_executable_sha256.as_deref()
            == Some(intent.game_executable_sha256.as_str()),
        installation_settled: game.manifest_update_status
            == SteamUpdateStatus::NoPendingUpdateIndicated,
    }
}

/// Re-read the Steam library and game installation under `steamapps` and compare. `None` when the
/// installation cannot be read (then nothing is attested).
pub fn inspect(intent: &ResearchIntentV1, steamapps: &Path, uid: u32) -> Option<HostFacts> {
    let libraries = super::discover_steam_libraries(steamapps, uid).ok()?;
    let (_, game) = super::discover_cyberpunk_installation(&libraries, uid).ok()?;
    let boot = stutter_core::boot::current_boot_id().ok();
    Some(compare(intent, boot.as_deref(), &game))
}

#[cfg(test)]
#[path = "host_baseline_tests.rs"]
mod tests;
