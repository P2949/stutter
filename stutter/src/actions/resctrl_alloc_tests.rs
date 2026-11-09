use std::fs;

use super::*;

fn mount(name: &str, closids: u32) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-resctrl-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("info/L3")).unwrap();
    fs::write(root.join("info/L3/cbm_mask"), "fff\n").unwrap();
    fs::write(root.join("info/L3/min_cbm_bits"), "2\n").unwrap();
    fs::write(root.join("info/L3/num_closids"), format!("{closids}\n")).unwrap();
    fs::write(root.join("schemata"), "L3:0=fff;1=fff\nMB:0=100;1=100\n").unwrap();
    root
}

fn request(mask: u64) -> CatRequest {
    CatRequest {
        name: "trial1".into(),
        mask,
    }
}

#[test]
fn a_prepared_group_commits_with_readback_and_restores_by_removal() {
    let root = mount("cycle", 4);
    let prepared = prepare(&root, &request(0x0f0)).unwrap();
    assert_eq!(prepared.schemata(), "L3:0=f0;1=f0");
    assert!(
        !root.join("stutter-cat-trial1").exists(),
        "prepare writes nothing"
    );
    let committed = prepared.commit().unwrap();
    assert_eq!(
        fs::read_to_string(root.join("stutter-cat-trial1/schemata"))
            .unwrap()
            .trim(),
        "L3:0=f0;1=f0"
    );
    assert_eq!(committed.restore(), CatRestore::Removed);
    assert!(!root.join("stutter-cat-trial1").exists());
    assert_eq!(
        fs::read_to_string(root.join("schemata")).unwrap(),
        "L3:0=fff;1=fff\nMB:0=100;1=100\n",
        "the default group is untouched"
    );
}

#[test]
fn bad_masks_names_and_absent_hardware_are_denied_before_any_write() {
    let root = mount("deny", 4);
    for mask in [0, 0b101, 0b1, 0x1000, 0x3_0000_0000_u64] {
        assert_eq!(
            prepare(&root, &request(mask)).unwrap_err(),
            CatDenial::BadMask,
            "{mask:#x}"
        );
    }
    let bad_name = CatRequest {
        name: "../x".into(),
        mask: 0xf0,
    };
    assert_eq!(prepare(&root, &bad_name).unwrap_err(), CatDenial::BadName);
    let absent = std::env::temp_dir().join("stutter-resctrl-not-mounted");
    assert_eq!(
        prepare(&absent, &request(0xf0)).unwrap_err(),
        CatDenial::Unavailable
    );
    fs::write(root.join("schemata"), "MB:0=100\n").unwrap();
    assert_eq!(
        prepare(&root, &request(0xf0)).unwrap_err(),
        CatDenial::RootUnreadable
    );
}

#[test]
fn a_depleted_closid_pool_and_an_existing_group_are_refused() {
    let root = mount("closid", 2);
    fs::create_dir(root.join("other")).unwrap();
    fs::write(root.join("other/schemata"), "L3:0=fff;1=fff\n").unwrap();
    assert_eq!(
        prepare(&root, &request(0xf0)).unwrap_err(),
        CatDenial::NoFreeClosid
    );
    let roomy = mount("exists", 8);
    fs::create_dir(roomy.join("stutter-cat-trial1")).unwrap();
    assert_eq!(
        prepare(&roomy, &request(0xf0)).unwrap_err(),
        CatDenial::GroupExists
    );
}

#[test]
fn restore_never_removes_a_replaced_group() {
    let root = mount("replaced", 4);
    let committed = prepare(&root, &request(0xf0)).unwrap().commit().unwrap();
    // Another owner's directory takes the path while ours is still alive (distinct inode).
    fs::rename(root.join("stutter-cat-trial1"), root.join("moved-away")).unwrap();
    fs::create_dir(root.join("stutter-cat-trial1")).unwrap();
    assert_eq!(committed.restore(), CatRestore::UnsafeToRestore);
    assert!(
        root.join("stutter-cat-trial1").exists(),
        "the other group survives"
    );
}

#[test]
fn restore_reports_an_already_absent_group() {
    let root = mount("absent", 4);
    let committed = prepare(&root, &request(0xf0)).unwrap().commit().unwrap();
    fs::remove_dir_all(root.join("stutter-cat-trial1")).unwrap();
    assert_eq!(committed.restore(), CatRestore::Absent);
}

#[test]
fn a_group_whose_identity_cannot_be_read_after_creation_is_removed_not_leaked() {
    let root = mount("identity-fault", 4);
    let prepared = prepare(&root, &request(0x0f0)).unwrap();
    let error = prepared
        .commit_with(|_| Err(io::Error::other("injected metadata failure")))
        .unwrap_err();
    assert_eq!(error, CatCommitError::IdentityUnreadable { removed: true });
    assert!(
        !root.join("stutter-cat-trial1").exists(),
        "the created group was left behind"
    );
}
