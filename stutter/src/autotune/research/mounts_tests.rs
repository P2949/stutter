use super::*;

const MOUNTS: &[u8] = b"1 0 8:1 / / rw - btrfs /dev/root rw\n\
2 1 8:1 /persist/user/Steam /home/user/Steam rw - btrfs /dev/root rw\n\
3 1 8:1 /persist/user/Studies /home/user/Studies rw - btrfs /dev/root rw\n\
4 3 8:1 /persist/user/Steam/steamapps/compatdata /home/user/Studies/attempt/compatdata rw - btrfs /dev/root rw\n";

#[test]
fn bind_mount_alias_inside_study_is_rejected() {
    let steam = Path::new("/home/user/Steam");
    assert!(ensure_disjoint(MOUNTS, Path::new("/home/user/Studies"), steam).is_ok());
    assert!(
        ensure_disjoint(
            MOUNTS,
            Path::new("/home/user/Studies/attempt/compatdata"),
            steam
        )
        .is_err()
    );
    assert!(
        ensure_disjoint(
            MOUNTS,
            Path::new("/home/user/Studies/attempt/compatdata/1091500"),
            steam
        )
        .is_err()
    );
}

#[test]
fn different_devices_and_nonoverlapping_bind_roots_are_distinct() {
    assert!(
        ensure_disjoint(
            MOUNTS,
            Path::new("/home/user/Studies/attempt"),
            Path::new("/home/user/Steam")
        )
        .is_ok()
    );
    let separate = b"1 0 8:1 / / rw - ext4 /dev/a rw\n2 1 8:2 / /other rw - ext4 /dev/b rw\n";
    assert!(ensure_disjoint(separate, Path::new("/safe"), Path::new("/other/steam")).is_ok());
}

#[test]
fn escapes_stacked_mounts_and_unsupported_filesystems_fail_safely() {
    let escaped = b"1 0 8:1 / / rw - btrfs /dev/a rw\n2 1 8:1 /steam\\040data /steam\\040alias rw - btrfs /dev/a rw\n";
    assert!(
        ensure_disjoint(
            escaped,
            Path::new("/steam alias/game"),
            Path::new("/steam data")
        )
        .is_err()
    );
    let stacked = b"1 0 8:1 / / rw - btrfs /dev/a rw\n2 1 8:2 / /x rw - btrfs /dev/b rw\n3 1 8:3 / /x rw - btrfs /dev/c rw\n";
    assert!(resolve(stacked, Path::new("/x/file")).is_err());
    let overlay = b"1 0 0:1 / / rw - overlay overlay rw\n";
    assert!(ensure_disjoint(overlay, Path::new("/a"), Path::new("/b")).is_err());
    assert!(resolve(b"1 0 8: / / rw - btrfs /dev/a rw\n", Path::new("/a")).is_err());
    assert!(decode_path(b"/bad\\000path").is_err());
}

#[test]
#[ignore = "requires the configured workstation study and Steam directories"]
fn workstation_study_and_steam_mounts_are_distinct_when_present() {
    let study = std::env::var_os("STUTTER_TEST_STUDY_ROOT")
        .map(PathBuf::from)
        .expect("study root");
    let steam = std::env::var_os("STUTTER_TEST_STEAM_ROOT")
        .map(PathBuf::from)
        .expect("Steam root");
    assert!(study.is_dir() && steam.is_dir());
    let before = snapshot().unwrap();
    ensure_disjoint(&before, &study, &steam).unwrap();
    assert_eq!(before, snapshot().unwrap());
}
