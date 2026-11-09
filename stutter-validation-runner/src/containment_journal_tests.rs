use std::os::unix::fs::{PermissionsExt, symlink};

use super::*;

fn fixture(name: &str) -> (crate::tests::Scratch, Birth) {
    let scratch = crate::tests::Scratch::new(name);
    fs::set_permissions(scratch.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let uid = crate::sys::effective_uid();
    let registry = Registry {
        path: scratch.path().to_owned(),
        uid,
        boot: "fixture".into(),
    };
    let record = Record {
        version: 1,
        boot: "fixture".into(),
        uid,
        pid: 424242,
        ticks: 987,
        parent: "/sys/fs/cgroup/fixture".into(),
        parent_id: KernelId {
            device: 1,
            inode: 2,
        },
        name: "stutter-owned-validation-424242-987-1".into(),
        resource: None,
    };
    (scratch, Birth { registry, record })
}

#[test]
fn birth_publication_is_atomic_and_never_replaces_an_existing_stage() {
    let (_scratch, mut birth) = fixture("birth-atomic");
    birth.write(true).unwrap();
    let original = birth.registry.records().unwrap();
    assert_eq!(original, [birth.record.clone()]);
    birth.record.version = 2;
    assert!(birth.write(true).is_err());
    assert_eq!(birth.registry.records().unwrap(), original);
    assert!(
        !birth
            .registry
            .path
            .join(format!("{}.json.tmp", birth.record.name))
            .exists()
    );
}

#[test]
fn regular_files_cannot_be_completed_as_kernel_birth_proof() {
    let (scratch, mut birth) = fixture("birth-not-kernel");
    birth.write(true).unwrap();
    let path = scratch.path().join("not-a-cgroup");
    fs::write(&path, "unrelated").unwrap();
    assert!(birth.complete(&File::open(path).unwrap()).is_err());
    assert!(birth.record.resource.is_none());
    assert!(birth.registry.records().unwrap()[0].resource.is_none());
}

#[test]
fn symlink_hardlink_and_oversize_records_are_refused_without_touching_the_target() {
    for kind in ["symlink", "hardlink", "oversize"] {
        let (scratch, birth) = fixture(kind);
        let target = scratch.path().join("unrelated");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&target)
            .unwrap();
        file.write_all(b"preserve").unwrap();
        drop(file);
        let record = scratch.path().join(format!("{}.json", birth.record.name));
        match kind {
            "symlink" => symlink(&target, record).unwrap(),
            "hardlink" => fs::hard_link(&target, record).unwrap(),
            _ => {
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .mode(0o600)
                    .open(record)
                    .unwrap();
                file.write_all(&vec![b' '; 8193]).unwrap();
            }
        }
        assert!(birth.registry.records().is_err(), "{kind}");
        assert_eq!(fs::read_to_string(target).unwrap(), "preserve");
    }
}

#[test]
fn retiring_a_birth_record_is_idempotent() {
    let (_scratch, birth) = fixture("birth-retire");
    birth.write(true).unwrap();
    birth.remove().unwrap();
    birth.remove().unwrap();
    assert!(birth.registry.records().unwrap().is_empty());
}

#[test]
fn an_unpublished_dead_owner_temp_is_retired_without_constructing_a_resource() {
    let (_scratch, birth) = fixture("birth-unpublished");
    let path = birth
        .registry
        .path
        .join("stutter-owned-validation-4294967295-1-1.json.tmp");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(b"partial JSON is not resource proof")
        .unwrap();
    drop(file);
    assert!(birth.registry.records().unwrap().is_empty());
    assert!(!path.exists());
}
