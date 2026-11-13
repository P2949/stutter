use std::os::unix::fs::PermissionsExt;

use super::*;

#[test]
fn numbered_attempt_dirs_cannot_be_rearmed_and_the_newest_is_selected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = std::fs::File::from(crate::syscall::open_dir_no_symlinks(dir.path()).unwrap());
    let mut intent = super::super::tests::intent();
    intent.uid = crate::syscall::geteuid();
    let slot = |index| AttemptSlot {
        study_sha256: "e".repeat(64),
        context_sha256: "f".repeat(64),
        index,
        arm: intent.tools[0].source,
    };
    let spec = || ArmSpec {
        tool_dir: "/t".into(),
        study_compat_data: "/s".into(),
        arm_env: BTreeMap::new(),
        steamapps: "/a".into(),
        proton_manifest: "/m".into(),
    };
    let now = intent.issued_unix_seconds + 1;
    assert!(latest_attempt_dir(&root).is_err());
    arm_attempt_in_root(&root, &intent, &slot(0), spec(), (now, 60)).unwrap();
    assert!(arm_attempt_in_root(&root, &intent, &slot(0), spec(), (now, 60)).is_err());
    arm_attempt_in_root(&root, &intent, &slot(1), spec(), (now, 60)).unwrap();
    let newest = latest_attempt_dir(&root).unwrap();
    let context = read_context(&newest, intent.uid).unwrap();
    assert_eq!(context.slot.index, 1);
}
