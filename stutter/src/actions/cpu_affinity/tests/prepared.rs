use std::cell::Cell;

use super::*;

struct Owned(std::process::Child);
impl Drop for Owned {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn child_action(
    path: std::path::PathBuf,
) -> (Owned, CpuAffinityProfileAction, crate::affinity::CpuMask) {
    let child = Owned(
        std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let original = crate::affinity::read_allowed_mask(child.0.id().into()).unwrap();
    let desired = crate::affinity::CpuMask::parse(
        original.to_range_string().split([',', '-']).next().unwrap(),
    )
    .unwrap();
    assert_ne!(
        original, desired,
        "explicit real test requires two allowed CPUs"
    );
    let action = CpuAffinityProfileAction {
        restore_path: Some(path),
        tree_pid: child.0.id(),
        force_restore_overwrite: false,
        profile: crate::profiles::Profile {
            name: "prepared-owned-child".into(),
            rules: vec![crate::profiles::ProfileRule {
                affinity: Some(desired),
                nice: None,
                ionice: None,
                match_class: vec![],
                match_comm: vec![crate::process_tree::CompiledPattern::new("/.*/".into()).unwrap()],
            }],
        },
    };
    (child, action, original)
}

#[test]
#[ignore = "real owned-child affinity mutation; requires two allowed CPUs"]
fn real_affinity_prepares_rollback_before_first_mutation() {
    let root = crate::test_support::TestRoot::new("prepared-before-affinity");
    let (_child, action, original) = child_action(root.join("restore.json"));
    let tid = action.tree_pid.into();
    let seen = Cell::new(false);
    let token = action
        .apply_with_prepared_rollback(&mut |token| {
            seen.set(true);
            assert_eq!(crate::affinity::read_allowed_mask(tid).unwrap(), original);
            let state =
                crate::profile_restore::load_restore_state(token.restore_path().unwrap()).unwrap();
            identity::validate_profile(&state, token.affected_tasks()).unwrap();
            assert_eq!(state.affinity_records.len(), 1);
            assert_eq!(state.affinity_records[0].original_mask, original);
            Ok(())
        })
        .unwrap();
    assert!(
        seen.get(),
        "durable preparation callback must precede mutation"
    );
    assert_ne!(crate::affinity::read_allowed_mask(tid).unwrap(), original);
    action.rollback(&token).unwrap();
    assert_eq!(crate::affinity::read_allowed_mask(tid).unwrap(), original);
}

struct ObservePrepared<'a> {
    action: CpuAffinityProfileAction,
    journal: &'a std::path::Path,
    original: crate::affinity::CpuMask,
    seen: &'a Cell<bool>,
}
impl TuningAction for ObservePrepared<'_> {
    fn id(&self) -> ActionId {
        self.action.id()
    }
    fn describe(&self) -> String {
        self.action.describe()
    }
    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }
    fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
        self.action.descriptor()
    }
    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.action.preflight()
    }
    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.action.dry_run()
    }
    fn apply(&self) -> crate::actions::ApplyResult {
        self.action.apply()
    }
    fn apply_with_prepared_rollback(
        &self,
        callback: &mut dyn FnMut(&RollbackToken) -> anyhow::Result<()>,
    ) -> crate::actions::ApplyResult {
        self.action.apply_with_prepared_rollback(&mut |token| {
            callback(token)?;
            let record =
                crate::autotune::controller_journal::read_controller_journal(self.journal)?;
            anyhow::ensure!(
                record.state()
                    == crate::autotune::controller_journal::ControllerJournalState::Applying
                    && record.rollback_token() == Some(token),
                "exact prepared rollback token must be durable before commit"
            );
            anyhow::ensure!(
                record.verify_result.as_deref() == Some("rollback_prepared_before_mutation"),
                "prepared phase marker missing"
            );
            anyhow::ensure!(
                crate::affinity::read_allowed_mask(self.action.tree_pid.into())? == self.original,
                "mutation preceded prepared journal"
            );
            let state = crate::profile_restore::load_restore_state(token.restore_path().unwrap())?;
            identity::validate_profile(&state, token.affected_tasks())?;
            self.seen.set(true);
            Ok(())
        })
    }
    fn verify(&self) -> anyhow::Result<ActionState> {
        self.action.verify()
    }
    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

#[test]
#[ignore = "real owned-child affinity mutation; requires two allowed CPUs"]
fn real_journal_token_precedes_affinity_mutation_through_boxed_runner() {
    let root = crate::test_support::TestRoot::new("prepared-journal-real");
    let journal = root.join("controller.json");
    let audit = root.join("audit.jsonl");
    let (_child, action, original) = child_action(root.join("restore.json"));
    let tid = action.tree_pid.into();
    let seen = Cell::new(false);
    let action: Box<dyn TuningAction + '_> = Box::new(ObservePrepared {
        action,
        journal: &journal,
        original: original.clone(),
        seen: &seen,
    });
    let applied = crate::autotune::action_transaction::apply_journaled_action(
        &action,
        crate::actions::runner::ActionRunPolicy::apply_low_risk(
            crate::daemon_policy::ActionSource::Test,
            false,
        ),
        crate::autotune::action_transaction::JournaledActionInput {
            journal_path: &journal,
            audit_path: &audit,
            experiment_id: "prepared-real-child",
            metadata: Default::default(),
        },
    )
    .unwrap();
    assert!(seen.get());
    assert!(applied.state.applied);
    assert_ne!(crate::affinity::read_allowed_mask(tid).unwrap(), original);
    let recovered = crate::autotune::startup_recovery::recover_controller_journal_on_startup(
        crate::autotune::startup_recovery::StartupRecoveryConfig {
            rollback_on_crash_recovery: true,
            journal_path: journal.clone(),
            audit_path: audit,
            history_path: root.join("history.jsonl"),
            state_snapshot_path: root.join("daemon_state.json"),
            privileged_worker_socket: None,
        },
    )
    .unwrap();
    assert!(matches!(
        recovered,
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Recovered {
            affected_tasks: 1,
            ..
        }
    ));
    assert_eq!(crate::affinity::read_allowed_mask(tid).unwrap(), original);
    assert!(
        crate::autotune::controller_journal::read_controller_journal(&journal)
            .unwrap()
            .is_clean()
    );
}

#[test]
#[ignore = "real owned-child affinity preparation; requires two allowed CPUs"]
fn real_rejected_prepared_callback_never_mutates() {
    let root = crate::test_support::TestRoot::new("prepared-rejection-real");
    let path = root.join("restore.json");
    let (_child, action, original) = child_action(path.clone());
    let seen = Cell::new(false);
    let error = action
        .apply_with_prepared_rollback(&mut |_| {
            seen.set(true);
            anyhow::bail!("injected prepared journal rejection")
        })
        .unwrap_err();
    assert!(seen.get());
    assert!(
        error.rollback.is_none(),
        "no mutation occurred, so do not issue restore syscalls"
    );
    assert_eq!(
        crate::affinity::read_allowed_mask(action.tree_pid.into()).unwrap(),
        original
    );
    assert!(path.is_file(), "retain original snapshot for inspection");
}

#[test]
fn only_affinity_only_profiles_claim_the_prepared_transaction() {
    let action = |nice: Option<i32>| CpuAffinityProfileAction {
        restore_path: None,
        tree_pid: 1,
        force_restore_overwrite: false,
        profile: crate::profiles::Profile {
            name: "claim".into(),
            rules: vec![crate::profiles::ProfileRule {
                affinity: Some(crate::affinity::CpuMask::parse("0").unwrap()),
                nice,
                ionice: None,
                match_class: vec![],
                match_comm: vec![],
            }],
        },
    };
    assert!(action(None).prepared_before_mutation());
    assert!(!action(Some(5)).prepared_before_mutation());
}
