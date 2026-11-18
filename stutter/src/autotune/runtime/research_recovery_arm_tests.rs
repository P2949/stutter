use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::autotune::research::{ProtonSource, session_host::arm_prefix_paths};

fn study() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

#[test]
fn an_arm_with_no_settings_file_and_no_snapshot_has_nothing_to_restore() {
    let root = study();
    let study = std::fs::canonicalize(root.path()).unwrap();
    assert!(restore_arm(&study, ProtonSource::Ge, crate::syscall::geteuid()).is_ok());
}

#[test]
fn a_snapshot_that_survives_without_its_settings_file_makes_recovery_refuse() {
    let root = study();
    let study = std::fs::canonicalize(root.path()).unwrap();
    let settings = arm_prefix_paths(&study, ProtonSource::Ge).2;
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    // The edit's durable snapshot is there; the file it belongs to is gone.
    std::fs::write(
        settings.with_file_name("UserSettings.json.stutter-original"),
        b"{}",
    )
    .unwrap();
    let error = restore_arm(&study, ProtonSource::Ge, crate::syscall::geteuid())
        .unwrap_err()
        .to_string();
    assert!(error.contains("UnsafeToResume"), "{error}");
    // Another arm with nothing pending is unaffected.
    assert!(restore_arm(&study, ProtonSource::Em, crate::syscall::geteuid()).is_ok());
}

#[test]
fn a_marker_without_its_settings_file_keeps_recovery_unresolved() {
    let root = study();
    let study = std::fs::canonicalize(root.path()).unwrap();
    let settings = arm_prefix_paths(&study, ProtonSource::Ge).2;
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let marker = settings.with_file_name("UserSettings.json.stutter-applied");
    std::fs::write(&marker, b"unsettled").unwrap();
    assert!(restore_arm(&study, ProtonSource::Ge, crate::syscall::geteuid()).is_err());
    assert!(marker.exists());
}
