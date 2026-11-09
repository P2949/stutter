use std::{fs, path::Path};

use super::*;
use crate::actions::block_scheduler::BlockSchedulerRequest;

fn action(name: &str, device: &str, scheduler: &str) -> BlockSchedulerAction {
    let root = std::env::temp_dir().join(format!("stutter-blkact-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for dev in ["disk1", "rootdisk"] {
        fs::create_dir_all(root.join(dev).join("queue")).unwrap();
        fs::write(root.join(dev).join("dev"), format!("259:{}\n", dev.len())).unwrap();
        fs::write(
            root.join(dev).join("queue/scheduler"),
            "none [mq-deadline] kyber bfq\n",
        )
        .unwrap();
    }
    BlockSchedulerAction {
        sys_block: root,
        request: BlockSchedulerRequest {
            device: device.into(),
            scheduler: scheduler.into(),
            authorized: ["disk1".to_owned(), "rootdisk".to_owned()].into(),
            protected: ["rootdisk".to_owned()].into(),
        },
    }
}

fn scheduler(root: &Path, device: &str) -> String {
    fs::read_to_string(root.join(device).join("queue/scheduler")).unwrap()
}

#[test]
fn the_default_policy_denies_and_the_action_is_high_risk_system_wide() {
    let action = action("deny", "disk1", "bfq");
    assert!(action.preflight().is_err());
    assert!(action.dry_run().is_err());
    assert!(action.apply().is_err());
    assert_eq!(action.safety_class(), SafetyClass::HighRisk);
    assert!(action.descriptor().touches_system_wide_state);
    assert!(scheduler(&action.sys_block, "disk1").contains("[mq-deadline]"));
}

#[test]
fn a_protected_device_is_refused_even_when_allowed() {
    let action = action("protected", "rootdisk", "bfq");
    assert!(
        action
            .apply_with_policy(&BlockSchedulerPolicy { allow: true })
            .is_err()
    );
    assert!(scheduler(&action.sys_block, "rootdisk").contains("[mq-deadline]"));
}

#[test]
fn a_foreign_token_is_refused_by_rollback() {
    let action = action("foreign-token", "disk1", "bfq");
    assert!(
        action
            .rollback(&RollbackToken::NiceRestore { records: vec![] })
            .is_err()
    );
}

#[test]
fn recovery_is_verified_automatic_and_the_token_round_trips() {
    use crate::actions::block_scheduler::BlockSchedulerRecord;
    let record = BlockSchedulerRecord {
        queue: PathBuf::from("/sys/block/none-existent-device/queue/scheduler"),
        device_number: "1:1".into(),
        original: "mq-deadline".into(),
        applied: "bfq".into(),
    };
    let token = RollbackToken::BlockSchedulerRestore {
        records: vec![record],
    };
    assert_eq!(token.kind(), "block-scheduler-restore");
    let json = serde_json::to_string(&token).unwrap();
    assert_eq!(serde_json::from_str::<RollbackToken>(&json).unwrap(), token);
    let descriptor = crate::actions::rollback::recovery::recovery_descriptor(&token);
    assert_eq!(
        descriptor.automatic_recovery,
        crate::actions::rollback::recovery::AutomaticRecoveryCapability::VerifiedAutomatic
    );
    // The device is not there: recovery refuses to write anywhere and reports it.
    let summary = crate::autotune::emergency_restore::restore_rollback_token(&token).unwrap();
    assert_eq!(summary.restored_items, 0);
    assert_eq!(summary.skipped_identity_mismatch, 1);
}

/// Live: a disposable loop device named by `STUTTER_LIVE_BLOCK_DEVICE`, run as root. The real
/// `/sys/block` scheduler list is read, a different scheduler selected, read back and restored.
#[test]
#[ignore = "live: needs root and a disposable loop device in STUTTER_LIVE_BLOCK_DEVICE"]
fn live_real_loop_device_scheduler_changes_and_restores() {
    let device = std::env::var("STUTTER_LIVE_BLOCK_DEVICE").expect("BlockedResource: loop device");
    let queue = Path::new("/sys/block")
        .join(&device)
        .join("queue/scheduler");
    let before = fs::read_to_string(&queue).unwrap();
    let other = before
        .split_whitespace()
        .map(|w| w.trim_matches(|c| c == '[' || c == ']').to_owned())
        .find(|name| !before.contains(&format!("[{name}]")))
        .expect("a second scheduler");
    let action = BlockSchedulerAction::new(BlockSchedulerRequest {
        device: device.clone(),
        scheduler: other.clone(),
        authorized: [device.clone()].into(),
        protected: Default::default(),
    });
    let token = action
        .apply_with_policy(&BlockSchedulerPolicy { allow: true })
        .unwrap();
    let during = fs::read_to_string(&queue).unwrap();
    action.rollback(&token).unwrap();
    let after = fs::read_to_string(&queue).unwrap();
    {
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "block live: {device} before={before:?} during={during:?} after={after:?}"
        );
    }
    assert!(during.contains(&format!("[{other}]")));
    assert_eq!(after.trim(), before.trim());
}
