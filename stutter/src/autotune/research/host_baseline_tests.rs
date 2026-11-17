use serde_json::json;

use super::*;
use crate::autotune::research::{
    proton::ProtonEligibility, steam_inventory::CyberpunkInstallation, tests::intent,
};

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

/// The inventory the campaign froze: every tool installed and eligible, the same Steam client.
fn inventory(intent: &ResearchIntentV1) -> ProtonInventory {
    let candidates: Vec<_> = intent
        .tools
        .iter()
        .map(|tool| {
            json!({
                "source": tool.source,
                "version": "test",
                "tool": tool,
                "eligibility": ProtonEligibility::Eligible,
            })
        })
        .collect();
    serde_json::from_value(json!({
        "schema_version": 1,
        "candidates": candidates,
        "steam_client": intent.steam_client,
    }))
    .unwrap()
}

fn facts(intent: &ResearchIntentV1) -> HostFacts {
    compare(
        intent,
        Some(&intent.boot_id),
        &game(intent),
        Some(&inventory(intent)),
        true,
    )
}

#[test]
fn an_unmoved_host_matches_every_frozen_fact() {
    let intent = intent();
    assert_eq!(
        facts(&intent),
        HostFacts {
            same_boot: true,
            build_id_matches: true,
            game_executable_matches: true,
            installation_settled: true,
            proton_identities_match: true,
            steam_client_matches: true,
            entitlement_verified: true,
        }
    );
}

#[test]
fn each_way_the_host_can_have_moved_is_its_own_fact() {
    let intent = intent();
    let compared = |boot: Option<&str>, game: &CyberpunkInstallation| {
        compare(&intent, boot, game, Some(&inventory(&intent)), true)
    };
    // Another boot, or one that cannot be read.
    assert!(!compared(Some("another-boot"), &game(&intent)).same_boot);
    assert!(!compared(None, &game(&intent)).same_boot);
    // The game updated.
    let mut updated = game(&intent);
    updated.build_id += 1;
    assert!(!compared(Some(&intent.boot_id), &updated).build_id_matches);
    // The executable was replaced, or could not be hashed.
    let mut replaced = game(&intent);
    replaced.game_executable_sha256 = Some("f".repeat(64));
    assert!(!compared(Some(&intent.boot_id), &replaced).game_executable_matches);
    let mut unhashed = game(&intent);
    unhashed.game_executable_sha256 = None;
    assert!(!compared(Some(&intent.boot_id), &unhashed).game_executable_matches);
    // Steam is mid-update or cannot say.
    for status in [SteamUpdateStatus::Updating, SteamUpdateStatus::Unknown] {
        let mut busy = game(&intent);
        busy.manifest_update_status = status;
        assert!(!compared(Some(&intent.boot_id), &busy).installation_settled);
    }
}

#[test]
fn a_changed_proton_build_or_steam_client_or_a_lapsed_launch_record_is_its_own_fact() {
    let intent = intent();
    let at = |inventory: Option<&ProtonInventory>, entitlement: bool| {
        compare(
            &intent,
            Some(&intent.boot_id),
            &game(&intent),
            inventory,
            entitlement,
        )
    };
    // A frozen Proton build whose identity changed (an update in place) is no longer the one frozen.
    let mut changed = intent.clone();
    changed.tools[0].identity_sha256 = "9".repeat(64);
    assert!(!at(Some(&inventory(&changed)), true).proton_identities_match);
    // A frozen build that is no longer installed at all.
    let mut fewer = intent.clone();
    fewer.tools.pop();
    assert!(!at(Some(&inventory(&fewer)), true).proton_identities_match);
    // An inventory that could not be read matches nothing, however few tools were frozen.
    let none = at(None, true);
    assert!(!none.proton_identities_match && !none.steam_client_matches);
    // A different Steam client branch or runtime identity.
    let mut other_branch = inventory(&intent);
    other_branch.steam_client.as_mut().unwrap().branch = "stable".into();
    assert!(!at(Some(&other_branch), true).steam_client_matches);
    let mut other_runtime = inventory(&intent);
    other_runtime.steam_client.as_mut().unwrap().identity_sha256 = "8".repeat(64);
    assert!(!at(Some(&other_runtime), true).steam_client_matches);
    // The account's launch record could not be found.
    assert!(!at(Some(&inventory(&intent)), false).entitlement_verified);
}

#[test]
fn an_unreadable_installation_attests_nothing() {
    let intent = intent();
    assert_eq!(
        inspect(&intent, Path::new("/nonexistent/steamapps"), None, 1000),
        None
    );
}
