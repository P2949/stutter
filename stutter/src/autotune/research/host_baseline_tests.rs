use super::*;
use crate::autotune::research::{steam_inventory::CyberpunkInstallation, tests::intent};

fn game(intent: &ResearchIntentV1) -> CyberpunkInstallation {
    CyberpunkInstallation {
        app_id: intent.app_id,
        build_id: intent.build_id,
        installation_directory: "/steam/common/Cyberpunk 2077".into(),
        manifest_update_status: SteamUpdateStatus::NoPendingUpdateIndicated,
        manifest_sha256: "m".repeat(64),
        game_executable_sha256: Some(intent.game_executable_sha256.clone()),
        launcher_executable_sha256: None,
        entitlement_verified: false,
        native_benchmark_verified: false,
    }
}

#[test]
fn an_unmoved_host_matches_every_frozen_fact() {
    let intent = intent();
    let facts = compare(&intent, Some(&intent.boot_id), &game(&intent));
    assert_eq!(
        facts,
        HostFacts {
            same_boot: true,
            build_id_matches: true,
            game_executable_matches: true,
            installation_settled: true,
        }
    );
}

#[test]
fn each_way_the_host_can_have_moved_is_its_own_fact() {
    let intent = intent();
    // Another boot, or one that cannot be read.
    assert!(!compare(&intent, Some("another-boot"), &game(&intent)).same_boot);
    assert!(!compare(&intent, None, &game(&intent)).same_boot);
    // The game updated.
    let mut updated = game(&intent);
    updated.build_id += 1;
    assert!(!compare(&intent, Some(&intent.boot_id), &updated).build_id_matches);
    // The executable was replaced, or could not be hashed.
    let mut replaced = game(&intent);
    replaced.game_executable_sha256 = Some("f".repeat(64));
    assert!(!compare(&intent, Some(&intent.boot_id), &replaced).game_executable_matches);
    let mut unhashed = game(&intent);
    unhashed.game_executable_sha256 = None;
    assert!(!compare(&intent, Some(&intent.boot_id), &unhashed).game_executable_matches);
    // Steam is mid-update or cannot say.
    for status in [SteamUpdateStatus::Updating, SteamUpdateStatus::Unknown] {
        let mut busy = game(&intent);
        busy.manifest_update_status = status;
        assert!(!compare(&intent, Some(&intent.boot_id), &busy).installation_settled);
    }
}

#[test]
fn an_unreadable_installation_attests_nothing() {
    let intent = intent();
    assert_eq!(
        inspect(&intent, Path::new("/nonexistent/steamapps"), 1000),
        None
    );
}
