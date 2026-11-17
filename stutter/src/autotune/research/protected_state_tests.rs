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
    assert_eq!(labels, ["saves", "cloud-42"]);
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
        status(&steam.steamapps, &dir, uid),
        Err("ProtectedStateNotRecorded")
    );
    record_baseline(&steam.steamapps, &dir, uid).unwrap();
    assert_eq!(status(&steam.steamapps, &dir, uid), Ok(()));
    // The player's data changes after the baseline.
    std::fs::write(steam.saves.join("ManualSave-1/sav.dat"), b"overwritten").unwrap();
    assert_eq!(
        status(&steam.steamapps, &dir, uid),
        Err("ProtectedStateChanged")
    );
    // A restarted campaign keeps the original baseline rather than re-recording the changed state
    // (which would hide the change).
    record_baseline(&steam.steamapps, &dir, uid).unwrap();
    assert_eq!(
        status(&steam.steamapps, &dir, uid),
        Err("ProtectedStateChanged")
    );
    // A baseline that cannot be made stops the campaign before it starts.
    let broken = self::steam();
    std::fs::remove_dir_all(&broken.saves).unwrap();
    std::fs::write(&broken.saves, b"x").unwrap();
    let other = tempfile::tempdir().unwrap();
    std::fs::set_permissions(other.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let other_dir = std::fs::File::open(other.path()).unwrap();
    assert!(record_baseline(&broken.steamapps, &other_dir, uid).is_err());
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
