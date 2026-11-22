use std::os::unix::fs::PermissionsExt;

use super::*;

struct Steam {
    _root: tempfile::TempDir,
    steamapps: PathBuf,
    saves: PathBuf,
    cloud: PathBuf,
}

fn steam() -> Steam {
    let root = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(root.path()).unwrap().join("Steam");
    let steamapps = base.join("steamapps");
    let saves = steamapps.join(
        "compatdata/1091500/pfx/drive_c/users/steamuser/Saved Games/CD Projekt Red/Cyberpunk 2077",
    );
    let cloud = base.join("userdata/42/1091500/remote");
    for dir in [saves.join("ManualSave-1"), cloud.clone()] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let game = steamapps.join("common/Cyberpunk 2077/bin/x64");
    std::fs::create_dir_all(&game).unwrap();
    std::fs::write(game.join("Cyberpunk2077.exe"), b"MZfixture").unwrap();
    std::fs::write(steamapps.join("appmanifest_1091500.acf"),
        "\"AppState\" { \"appid\" \"1091500\" \"buildid\" \"42\" \"installdir\" \"Cyberpunk 2077\" \"StateFlags\" \"4\" }").unwrap();
    std::fs::write(
        steamapps.join("libraryfolders.vdf"),
        format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
            base.display()
        ),
    )
    .unwrap();
    std::fs::write(saves.join("ManualSave-1/sav.dat"), b"the player's save").unwrap();
    std::fs::write(cloud.join("cloud.sav"), b"cloud copy").unwrap();
    Steam {
        _root: root,
        steamapps,
        saves,
        cloud,
    }
}

fn changed(steam: &Steam, edit: impl FnOnce(&Steam)) -> Result<(), &'static str> {
    let before = capture(&steam.steamapps).unwrap();
    edit(steam);
    compare(&before, &capture(&steam.steamapps).unwrap())
}

#[test]
fn an_untouched_tree_has_the_same_fingerprint_and_covers_the_saves_and_each_cloud_account() {
    let steam = steam();
    let first = capture(&steam.steamapps).unwrap();
    assert_eq!(compare(&first, &capture(&steam.steamapps).unwrap()), Ok(()));
    let labels: Vec<_> = first.roots.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "saves",
            &format!("cloud-{}", stutter_core::hash::sha256_hex(b"42"))
        ]
    );
    assert!(
        first
            .roots
            .iter()
            .all(|root| root.present && root.entries > 0)
    );
    // The fingerprint carries no private path.
    let text = serde_json::to_string(&first).unwrap();
    assert!(
        !text.contains("Cyberpunk 2077") && !text.contains("Steam"),
        "{text}"
    );
}

#[test]
fn secondary_game_library_uses_primary_userdata_and_rejects_unreadable_accounts() {
    let steam = steam();
    let primary = steam.steamapps.parent().unwrap();
    let secondary = steam._root.path().join("secondary");
    std::fs::create_dir_all(secondary.join("steamapps")).unwrap();
    let secondary_apps = secondary.join("steamapps");
    std::fs::rename(
        steam.steamapps.join("common"),
        secondary_apps.join("common"),
    )
    .unwrap();
    std::fs::rename(
        steam.steamapps.join("appmanifest_1091500.acf"),
        secondary_apps.join("appmanifest_1091500.acf"),
    )
    .unwrap();
    std::fs::rename(
        steam.steamapps.join("compatdata"),
        secondary_apps.join("compatdata"),
    )
    .unwrap();
    std::fs::write(
        steam.steamapps.join("libraryfolders.vdf"),
        format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} }}",
            primary.display(),
            secondary.display()
        ),
    )
    .unwrap();
    let before = capture(&steam.steamapps).unwrap();
    assert!(before.roots.iter().all(|root| root.present));
    let relocated_save = secondary_apps.join("compatdata/1091500/pfx/drive_c/users/steamuser/Saved Games/CD Projekt Red/Cyberpunk 2077/ManualSave-1/sav.dat");
    std::fs::write(relocated_save, b"changed").unwrap();
    assert_eq!(
        compare(&before, &capture(&steam.steamapps).unwrap()),
        Err("ProtectedStateChanged")
    );
    let userdata = primary.join("userdata");
    std::fs::remove_dir_all(&userdata).unwrap();
    assert_eq!(capture(&steam.steamapps), Err("ProtectedStateUnreadable"));
}

#[test]
fn save_root_with_a_symlinked_ancestor_is_not_followed() {
    let steam = steam();
    let saved_games = steam.saves.ancestors().nth(2).unwrap();
    let moved = saved_games.with_file_name("moved-saved-games");
    std::fs::rename(saved_games, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, saved_games).unwrap();
    assert_eq!(capture(&steam.steamapps), Err("ProtectedStateUnreadable"));
}

#[test]
fn any_change_to_the_players_data_is_a_change_and_a_mere_touch_is_not() {
    // Rewritten with the same size: a content change, not a size change.
    assert_eq!(
        changed(&steam(), |s| std::fs::write(
            s.saves.join("ManualSave-1/sav.dat"),
            b"the player's SAVE"
        )
        .unwrap()),
        Err("ProtectedStateChanged")
    );
    // A new file, a deleted file, a new directory, and a changed cloud copy.
    assert!(
        changed(&steam(), |s| std::fs::write(s.saves.join("new.dat"), b"x")
            .unwrap())
        .is_err()
    );
    assert!(
        changed(&steam(), |s| std::fs::remove_file(
            s.saves.join("ManualSave-1/sav.dat")
        )
        .unwrap())
        .is_err()
    );
    assert!(
        changed(&steam(), |s| std::fs::create_dir(
            s.saves.join("AutoSave-1")
        )
        .unwrap())
        .is_err()
    );
    assert!(
        changed(&steam(), |s| std::fs::write(
            s.cloud.join("cloud.sav"),
            b"other"
        )
        .unwrap())
        .is_err()
    );
    // Rewriting a file with identical bytes (a touch) is not a change.
    assert_eq!(
        changed(&steam(), |s| std::fs::write(
            s.saves.join("ManualSave-1/sav.dat"),
            b"the player's save"
        )
        .unwrap()),
        Ok(())
    );
}

#[test]
fn links_are_recorded_not_followed_and_a_root_appearing_or_vanishing_is_a_change() {
    // A link is fingerprinted by where it points; swapping it is a change, and following it never
    // reads outside the root.
    let steam = steam();
    std::os::unix::fs::symlink("/etc/hostname", steam.saves.join("link")).unwrap();
    assert!(
        changed(&steam, |s| {
            std::fs::remove_file(s.saves.join("link")).unwrap();
            std::os::unix::fs::symlink("/etc/passwd", s.saves.join("link")).unwrap();
        })
        .is_err()
    );
    // The whole save directory vanishing, or the cloud directory being created, is a change.
    assert!(
        changed(&self::steam(), |s| std::fs::remove_dir_all(&s.saves)
            .unwrap())
        .is_err()
    );
    let bare = self::steam();
    std::fs::remove_dir_all(&bare.cloud).unwrap();
    assert!(changed(&bare, |s| std::fs::create_dir_all(&s.cloud).unwrap()).is_err());
    // A save root that is a file where a directory belongs cannot be fingerprinted.
    let odd = self::steam();
    std::fs::remove_dir_all(&odd.saves).unwrap();
    std::fs::write(&odd.saves, b"not a dir").unwrap();
    assert_eq!(capture(&odd.steamapps), Err("ProtectedStateUnreadable"));
}

#[test]
fn the_baseline_is_taken_once_before_any_launch_and_later_state_is_compared_with_it() {
    let steam = steam();
    let study = tempfile::tempdir().unwrap();
    std::fs::set_permissions(study.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = std::fs::File::open(study.path()).unwrap();
    let uid = crate::syscall::geteuid();
    // No baseline yet: nothing can be said to be unchanged.
    assert_eq!(
        status(&steam.steamapps, &dir, uid, &"a".repeat(64)),
        Err("ProtectedStateNotRecorded")
    );
    record_baseline(&steam.steamapps, &dir, uid, &"a".repeat(64)).unwrap();
    assert_eq!(status(&steam.steamapps, &dir, uid, &"a".repeat(64)), Ok(()));
    // The player's data changes after the baseline.
    std::fs::write(steam.saves.join("ManualSave-1/sav.dat"), b"overwritten").unwrap();
    assert_eq!(
        status(&steam.steamapps, &dir, uid, &"a".repeat(64)),
        Err("ProtectedStateChanged")
    );
    assert_eq!(
        status(&steam.steamapps, &dir, uid, &"b".repeat(64)),
        Err("ProtectedStateNotRecorded")
    );
    record_baseline(&steam.steamapps, &dir, uid, &"b".repeat(64)).unwrap();
    assert_eq!(status(&steam.steamapps, &dir, uid, &"b".repeat(64)), Ok(()));
    assert_eq!(
        status(&steam.steamapps, &dir, uid, &"a".repeat(64)),
        Err("ProtectedStateChanged")
    );
    // A restarted campaign keeps the original baseline rather than re-recording the changed state
    // (which would hide the change).
    record_baseline(&steam.steamapps, &dir, uid, &"a".repeat(64)).unwrap();
    assert_eq!(
        status(&steam.steamapps, &dir, uid, &"a".repeat(64)),
        Err("ProtectedStateChanged")
    );
    // A baseline that cannot be made stops the campaign before it starts.
    let broken = self::steam();
    std::fs::remove_dir_all(&broken.saves).unwrap();
    std::fs::write(&broken.saves, b"x").unwrap();
    let other = tempfile::tempdir().unwrap();
    std::fs::set_permissions(other.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let other_dir = std::fs::File::open(other.path()).unwrap();
    assert!(record_baseline(&broken.steamapps, &other_dir, uid, &"a".repeat(64)).is_err());
}

/// Read-only fingerprint of a real Steam library, printing only counts:
/// `STUTTER_REAL_STEAMAPPS=<steamapps> cargo test real_protected_state -- --ignored --nocapture`.
#[test]
#[ignore = "needs a real Steam library"]
fn real_protected_state() {
    let steamapps = std::env::var_os("STUTTER_REAL_STEAMAPPS").expect("set STUTTER_REAL_STEAMAPPS");
    let started = std::time::Instant::now();
    let first = capture(Path::new(&steamapps)).unwrap();
    for root in &first.roots {
        println!(
            "{}: present={} entries={} bytes={}",
            root.label, root.present, root.entries, root.bytes
        );
    }
    println!("fingerprinted in {:?}", started.elapsed());
    assert_eq!(
        compare(&first, &capture(Path::new(&steamapps)).unwrap()),
        Ok(())
    );
}

#[test]
fn the_unchanged_state_is_sealed_once_and_a_later_change_cannot_unseal_it_or_seal_a_changed_one() {
    let world = steam();
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let study = std::fs::File::open(dir.path()).unwrap();
    let uid = crate::syscall::geteuid();
    let campaign = "ab".repeat(32);
    assert_eq!(
        sealed(&study, uid, &campaign),
        Err("ProtectedStateNotSealedAtFinalization")
    );
    assert_eq!(
        seal_unchanged(&world.steamapps, &study, uid, &campaign, 10),
        Err("ProtectedStateNotRecorded")
    );
    record_baseline(&world.steamapps, &study, uid, &campaign).unwrap();
    assert_eq!(
        seal_unchanged(&world.steamapps, &study, uid, &campaign, 10),
        Ok(())
    );
    assert_eq!(sealed(&study, uid, &campaign), Ok(()));
    // Sealing again is idempotent; the player later changes a save: the live comparison fails but the seal stands.
    assert_eq!(
        seal_unchanged(&world.steamapps, &study, uid, &campaign, 20),
        Ok(())
    );
    std::fs::write(world.saves.join("ManualSave-1/sav.dat"), b"edited later").unwrap();
    assert!(status(&world.steamapps, &study, uid, &campaign).is_err());
    assert_eq!(sealed(&study, uid, &campaign), Ok(()));
    // A changed state is never sealed as unchanged: a different campaign with the same changed saves cannot seal.
    let other = "cd".repeat(32);
    let dir2 = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir2.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let study2 = std::fs::File::open(dir2.path()).unwrap();
    let before = steam();
    record_baseline(&before.steamapps, &study2, uid, &other).unwrap();
    std::fs::write(before.saves.join("ManualSave-1/sav.dat"), b"changed").unwrap();
    assert!(seal_unchanged(&before.steamapps, &study2, uid, &other, 30).is_err());
    assert_eq!(
        sealed(&study2, uid, &other),
        Err("ProtectedStateNotSealedAtFinalization")
    );
}

#[test]
fn only_the_preterminal_finalization_step_may_write_the_seal() {
    // Ordinary verification must stay read-only, or a later report could create the fact it then reports.
    let report = include_str!("../runtime/research_control_report.rs");
    assert!(
        !report.contains("seal_unchanged("),
        "report verification must not seal"
    );
    let finalize = include_str!("../runtime/research_control_finalize.rs");
    assert!(
        finalize.contains("seal_unchanged("),
        "the preterminal step seals"
    );
    assert!(
        finalize.contains("protected_state_seal_sha256"),
        "and binds the digest into the decision"
    );
}
