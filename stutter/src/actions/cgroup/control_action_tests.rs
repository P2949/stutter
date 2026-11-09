use std::fs;

use super::*;

fn group(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("stutter-cgact-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("delegated");
    fs::create_dir_all(&dir).unwrap();
    for (file, text) in [
        ("cgroup.type", "domain\n"),
        ("cpu.idle", "0\n"),
        ("cpu.weight", "100\n"),
        ("memory.high", "max\n"),
    ] {
        fs::write(dir.join(file), text).unwrap();
    }
    fs::write(
        root.join("cgroup.subtree_control"),
        "cpu cpuset memory io\n",
    )
    .unwrap();
    dir
}

fn allowed() -> CgroupControlPolicy {
    CgroupControlPolicy {
        allow: true,
        io_weight_honored: None,
    }
}

#[test]
fn the_default_policy_denies_and_every_knob_is_high_risk_manual() {
    let action = CgroupControlAction {
        dir: group("deny"),
        control: CgroupControl::CpuWeight(300),
    };
    assert!(action.preflight().is_err());
    assert!(action.dry_run().is_err());
    assert!(action.apply().is_err());
    assert_eq!(action.safety_class(), SafetyClass::HighRisk);
    assert!(action.descriptor().touches_system_wide_state);
    assert_eq!(
        fs::read_to_string(action.dir.join("cpu.weight")).unwrap(),
        "100\n"
    );
}

#[test]
fn an_allowed_write_is_read_back_and_rollback_restores_the_baseline() {
    let dir = group("cycle");
    let action = CgroupControlAction {
        dir: dir.clone(),
        control: CgroupControl::MemoryHigh(Some(1 << 30)),
    };
    let token = action.apply_with_policy(&allowed()).unwrap();
    assert_eq!(token.kind(), "cgroup-control-restore");
    assert!(action.verify().unwrap().applied);
    let json = serde_json::to_string(&token).unwrap();
    assert_eq!(serde_json::from_str::<RollbackToken>(&json).unwrap(), token);
    action.rollback(&token).unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("memory.high")).unwrap().trim(),
        "max"
    );
}

#[test]
fn a_foreign_value_set_after_ours_is_never_overwritten_by_rollback() {
    let dir = group("foreign");
    let action = CgroupControlAction {
        dir: dir.clone(),
        control: CgroupControl::CpuWeight(300),
    };
    let token = action.apply_with_policy(&allowed()).unwrap();
    fs::write(dir.join("cpu.weight"), "555\n").unwrap();
    assert!(action.rollback(&token).is_err());
    assert_eq!(
        fs::read_to_string(dir.join("cpu.weight")).unwrap().trim(),
        "555"
    );
    assert!(
        action
            .rollback(&RollbackToken::NiceRestore { records: vec![] })
            .is_err()
    );
}

#[test]
fn recovery_through_the_registry_restores_only_what_is_still_ours() {
    let dir = group("registry");
    let action = CgroupControlAction {
        dir: dir.clone(),
        control: CgroupControl::CpuWeight(300),
    };
    let token = action.apply_with_policy(&allowed()).unwrap();
    let descriptor = crate::actions::rollback::recovery::recovery_descriptor(&token);
    assert_eq!(
        descriptor.automatic_recovery,
        crate::actions::rollback::recovery::AutomaticRecoveryCapability::VerifiedAutomatic
    );
    let summary = crate::autotune::emergency_restore::restore_rollback_token(&token).unwrap();
    assert_eq!(summary.restored_items, 1);
    assert_eq!(
        fs::read_to_string(dir.join("cpu.weight")).unwrap().trim(),
        "100"
    );
}

#[test]
fn preflight_denials_surface_before_any_write() {
    let dir = group("threaded");
    fs::write(dir.join("cgroup.type"), "domain threaded\n").unwrap();
    let action = CgroupControlAction {
        dir: dir.clone(),
        control: CgroupControl::CpuWeight(300),
    };
    assert!(action.apply_with_policy(&allowed()).is_err());
    assert_eq!(fs::read_to_string(dir.join("cpu.weight")).unwrap(), "100\n");
}

/// Live: a disposable empty cgroup under the real cgroup2 root, run as root. Every knob the host's
/// root enables is written through the action, read back from the kernel and restored.
#[test]
#[ignore = "live: needs root and cgroup2 at /sys/fs/cgroup"]
fn live_real_cgroup_knobs_write_read_back_and_restore() {
    let root = PathBuf::from("/sys/fs/cgroup");
    let dir = root.join(format!("stutter-live-{}", std::process::id()));
    fs::create_dir(&dir).expect("BlockedResource: root and cgroup2");
    let cases = [
        CgroupControl::CpuWeight(300),
        CgroupControl::CpuIdle(true),
        CgroupControl::CpuMax {
            quota_us: Some(50_000),
            period_us: 100_000,
        },
        CgroupControl::MemoryHigh(Some(1 << 30)),
        CgroupControl::MemoryLow(1 << 20),
        CgroupControl::MemoryMin(1 << 20),
    ];
    let mut proven = Vec::new();
    for control in cases {
        let action = CgroupControlAction {
            dir: dir.clone(),
            control: control.clone(),
        };
        match action.apply_with_policy(&allowed()) {
            Ok(token) => {
                assert!(
                    action.verify().unwrap().applied,
                    "{control:?} not read back"
                );
                action.rollback(&token).unwrap();
                assert!(
                    !action.verify().unwrap().applied,
                    "{control:?} not restored"
                );
                proven.push(control.file_and_controller().0);
            }
            Err(error) => eprintln!("cgroup live: {control:?} not exercised: {error}"),
        }
    }
    eprintln!("cgroup live: proven {proven:?}");
    fs::remove_dir(&dir).unwrap();
    assert!(
        proven.contains(&"cpu.weight"),
        "cpu.weight must be exercisable"
    );
}
