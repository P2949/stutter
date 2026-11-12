use std::{
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
};

use super::*;

fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let steam = root.path().join("Steam");
    let primary = steam.join("steamapps");
    let study = root.path().join("study");
    std::fs::create_dir_all(primary.join("compatdata/1091500")).unwrap();
    std::fs::create_dir_all(study.join("compatdata")).unwrap();
    std::fs::set_permissions(&study, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(
        study.join("compatdata"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::fs::write(
        primary.join("libraryfolders.vdf"),
        format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} }}",
            steam.display()
        ),
    )
    .unwrap();
    (root, primary, study)
}

fn check(
    primary: &Path,
    study: &Path,
    actual: &Path,
    empty: bool,
) -> anyhow::Result<StudyPrefixIdentity> {
    verify_study_compatdata(primary, study, actual, crate::syscall::geteuid(), empty)
}

#[test]
fn accepts_only_exact_private_study_path_and_checks_freshness() {
    let (_root, primary, study) = fixture();
    let owned = study.join("compatdata");
    let identity = check(&primary, &study, &owned, true).unwrap();
    assert_ne!(identity.inode, 0);
    assert!(check(&primary, &study, &primary.join("compatdata/1091500"), false).is_err());
    std::fs::write(owned.join("existing"), b"fixture").unwrap();
    assert!(check(&primary, &study, &owned, true).is_err());
    assert!(check(&primary, &study, &owned, false).is_ok());
}

#[test]
fn refuses_steam_overlap_public_directories_and_symlinked_prefixes() {
    let (root, primary, study) = fixture();
    let owned = study.join("compatdata");
    std::fs::set_permissions(&study, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(check(&primary, &study, &owned, false).is_err());
    std::fs::set_permissions(&study, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::remove_dir(&owned).unwrap();
    symlink(primary.join("compatdata/1091500"), &owned).unwrap();
    assert!(check(&primary, &study, &owned, false).is_err());
    let nested = primary.parent().unwrap().join("research");
    std::fs::create_dir_all(nested.join("compatdata")).unwrap();
    std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::set_permissions(
        nested.join("compatdata"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert!(check(&primary, &nested, &nested.join("compatdata"), true).is_err());
    drop(root);
}
