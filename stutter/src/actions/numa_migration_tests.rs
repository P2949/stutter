use std::{cell::Cell, fs};

use super::*;

struct Fake {
    proc_root: PathBuf,
    node_root: PathBuf,
    task: u32,
}

fn fake(name: &str) -> Fake {
    let base = std::env::temp_dir().join(format!("stutter-numa-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let task = 4242;
    let dir = base.join("proc").join(task.to_string());
    fs::create_dir_all(&dir).unwrap();
    // `stat` with start time 777 at field 22, parent 1.
    fs::write(
        dir.join("stat"),
        format!("{task} (app) S 1 {task} {task} 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 777 0 0 0\n"),
    )
    .unwrap();
    fs::write(dir.join("numa_maps"), "7f00 default anon=100 N0=100\n").unwrap();
    let node_root = base.join("node");
    fs::create_dir_all(&node_root).unwrap();
    fs::write(node_root.join("online"), "0-1\n").unwrap();
    Fake {
        proc_root: base.join("proc"),
        node_root,
        task,
    }
}

fn action(fake: &Fake) -> NumaMigrationAction {
    NumaMigrationAction {
        proc_root: fake.proc_root.clone(),
        node_root: fake.node_root.clone(),
        task: fake.task,
        start_ticks: 777,
        from_nodes: vec![0],
        to_nodes: vec![1],
    }
}

fn allowed() -> NumaMigrationPolicy {
    NumaMigrationPolicy {
        allow: true,
        acknowledge_irreversible: true,
    }
}

/// A kernel stand-in that moves the pages in the fake numa_maps and reports a few unmovable.
struct Mover<'a> {
    fake: &'a Fake,
    calls: Cell<u32>,
}

impl PageMigrator for Mover<'_> {
    fn migrate(&self, _task: u32, from: u64, to: u64) -> std::io::Result<u64> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!((from, to), (0b01, 0b10));
        fs::write(
            self.fake
                .proc_root
                .join(self.fake.task.to_string())
                .join("numa_maps"),
            "7f00 default anon=100 N0=7 N1=93\n",
        )
        .unwrap();
        Ok(7)
    }
}

#[test]
fn the_default_policy_denies_and_the_action_is_high_risk_with_a_persistent_effect() {
    let fake = fake("deny");
    let action = action(&fake);
    assert!(action.preflight().is_err());
    assert!(action.dry_run().is_err());
    assert_eq!(action.safety_class(), SafetyClass::HighRisk);
    let descriptor = action.descriptor();
    assert!(descriptor.persistent_effect && descriptor.touches_system_wide_state);
    let unacknowledged = NumaMigrationPolicy {
        allow: true,
        acknowledge_irreversible: false,
    };
    let mover = Mover {
        fake: &fake,
        calls: Cell::new(0),
    };
    assert!(action.apply_with_policy(&unacknowledged, &mover).is_err());
    assert_eq!(mover.calls.get(), 0);
}

#[test]
fn an_allowed_migration_accounts_for_what_moved_and_cannot_be_rolled_back() {
    let fake = fake("cycle");
    let action = action(&fake);
    let mover = Mover {
        fake: &fake,
        calls: Cell::new(0),
    };
    let token = action.apply_with_policy(&allowed(), &mover).unwrap();
    assert_eq!(mover.calls.get(), 1);
    let RollbackToken::NumaMigration { records } = &token else {
        panic!("wrong token");
    };
    assert_eq!(records[0].before, vec![(0, 100)]);
    assert_eq!(records[0].after, vec![(0, 7), (1, 93)]);
    assert_eq!(records[0].not_moved, 7);
    assert!(
        action.verify().unwrap().pending_changes == 1,
        "7 pages remain on the source"
    );
    let json = serde_json::to_string(&token).unwrap();
    assert_eq!(serde_json::from_str::<RollbackToken>(&json).unwrap(), token);
    assert!(
        action.rollback(&token).is_err(),
        "migration is not exactly reversible"
    );
    let descriptor = crate::actions::rollback::recovery::recovery_descriptor(&token);
    assert_eq!(
        descriptor.automatic_recovery,
        crate::actions::rollback::recovery::AutomaticRecoveryCapability::ManualOnly
    );
    assert!(descriptor.identity_contract.contains("irreversible"));
}

#[test]
fn a_reused_pid_bad_nodes_and_overlapping_sets_are_refused_before_the_call() {
    let fake = fake("refuse");
    let mover = Mover {
        fake: &fake,
        calls: Cell::new(0),
    };
    let mut reused = action(&fake);
    reused.start_ticks = 1;
    assert!(reused.apply_with_policy(&allowed(), &mover).is_err());
    let mut offline = action(&fake);
    offline.to_nodes = vec![5];
    assert!(offline.apply_with_policy(&allowed(), &mover).is_err());
    let mut overlap = action(&fake);
    overlap.to_nodes = vec![0];
    assert!(overlap.apply_with_policy(&allowed(), &mover).is_err());
    let mut wide = action(&fake);
    wide.to_nodes = vec![64];
    assert!(wide.apply_with_policy(&allowed(), &mover).is_err());
    assert_eq!(mover.calls.get(), 0);
}

#[test]
fn recovery_names_the_manual_command_and_the_executor_refuses_to_pretend() {
    let fake = fake("manual");
    let mover = Mover {
        fake: &fake,
        calls: Cell::new(0),
    };
    let token = action(&fake).apply_with_policy(&allowed(), &mover).unwrap();
    assert_eq!(
        crate::autotune::emergency_restore::manual_restore_command_for_token(&token),
        "migratepages 4242 1 0"
    );
    assert!(crate::autotune::emergency_restore::restore_rollback_token(&token).is_err());
}
