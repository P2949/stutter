use std::os::unix::fs::PermissionsExt;

use super::*;

fn store() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn binary(store: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::fs::canonicalize(store.path()).unwrap().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn shown(exec: &Path, state: &str) -> String {
    format!(
        "ActiveState={state}\nExecStart={{ path={} ; argv[]={} autotune --mode x ; ignore_errors=no }}\n",
        exec.display(),
        exec.display()
    )
}

#[test]
fn the_unit_binary_and_state_are_read_from_systemctl_show() {
    let store = store();
    let exe = binary(&store, "h-stutter", b"release bytes");
    let text = shown(&exe, "active");
    assert_eq!(exec_path(&text).as_deref(), exe.to_str());
    let unit = unit_receipt(
        "stutter-autotune-bounded@p2949.service",
        &text,
        store.path(),
    )
    .unwrap();
    assert_eq!(unit.active_state, "active");
    assert_eq!(
        unit.binary_sha256,
        stutter_core::hash::sha256_hex(b"release bytes")
    );
    assert_eq!(exec_path("ActiveState=active\n"), None);
}

#[test]
fn a_binary_outside_the_store_or_unreadable_is_refused() {
    let store = store();
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = std::fs::canonicalize(elsewhere.path())
        .unwrap()
        .join("stutter");
    std::fs::write(&outside, b"x").unwrap();
    // A mutable path is not an immutable store object.
    assert!(unit_receipt("u", &shown(&outside, "active"), store.path()).is_err());
    assert!(unit_receipt("u", "ActiveState=active\n", store.path()).is_err());
    // A symlink out of the store is refused too.
    let inside = std::fs::canonicalize(store.path()).unwrap().join("link");
    std::os::unix::fs::symlink(&outside, &inside).unwrap();
    assert!(binary_sha256(&inside, store.path()).is_err());
}

#[test]
fn the_receipt_matches_the_pin_only_when_every_unit_runs_it_and_is_active() {
    let store = store();
    let good = binary(&store, "a-stutter", b"pinned");
    let other = binary(&store, "b-stutter", b"something else");
    let pin = stutter_core::hash::sha256_hex(b"pinned");
    let units = |worker: &Path, state: &str| {
        vec![
            unit_receipt("controller", &shown(&good, "active"), store.path()).unwrap(),
            unit_receipt("worker", &shown(worker, state), store.path()).unwrap(),
        ]
    };
    let ok = receipt(units(&good, "active"), &pin, 100);
    assert!(ok.binary_matches_pin && ok.units_active);
    // One unit on another binary, or not active, or no units at all.
    assert!(!receipt(units(&other, "active"), &pin, 100).binary_matches_pin);
    assert!(!receipt(units(&good, "inactive"), &pin, 100).units_active);
    let none = receipt(Vec::new(), &pin, 100);
    assert!(!none.binary_matches_pin && !none.units_active);
}

#[test]
fn g8_needs_a_receipt_about_the_very_binary_doing_the_verification() {
    let store = store();
    let exe = binary(&store, "c-stutter", b"verifier bytes");
    let me = stutter_core::hash::sha256_hex(b"verifier bytes");
    let study = tempfile::tempdir().unwrap();
    let write = |name: &str, receipt: &DeployReceipt| {
        let path = study.path().join(name);
        std::fs::write(&path, serde_json::to_vec(receipt).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    let units = vec![unit_receipt("u", &shown(&exe, "active"), store.path()).unwrap()];
    // No receipt: nothing attested.
    assert!(!deployment_attested(study.path(), &me));
    write("deploy-receipt-100.json", &receipt(units.clone(), &me, 100));
    assert!(deployment_attested(study.path(), &me));
    // A receipt about some other binary says nothing about this one.
    assert!(!deployment_attested(study.path(), "0".repeat(64).as_str()));
    // The newest receipt decides: a later one that no longer matches the pin withdraws it.
    write(
        "deploy-receipt-200.json",
        &receipt(units, &"1".repeat(64), 200),
    );
    assert!(!deployment_attested(study.path(), &me));
}

#[test]
fn unit_names_are_validated_before_anything_runs() {
    assert!(show(Path::new("/nonexistent"), "bad name; rm").is_err());
    assert!(show(Path::new("/nonexistent"), "").is_err());
}
