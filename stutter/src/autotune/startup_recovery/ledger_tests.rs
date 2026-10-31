use std::cell::RefCell;

use super::*;
use crate::autotune::kept_ledger::{
    KeptLedgerEntry, kept_ledger_path, read_kept_ledger, record_kept,
};

#[derive(Default)]
struct CountingExecutor {
    restored: RefCell<Vec<RollbackToken>>,
    fail: bool,
}

impl StartupRecoveryRollbackExecutor for CountingExecutor {
    fn rollback(
        &mut self,
        token: &RollbackToken,
    ) -> anyhow::Result<StartupRecoveryRollbackSummary> {
        anyhow::ensure!(!self.fail, "restore refused");
        self.restored.borrow_mut().push(token.clone());
        Ok(StartupRecoveryRollbackSummary {
            affected_tasks: 1,
            message: "restored".to_owned(),
        })
    }
}

fn token(name: &str) -> RollbackToken {
    RollbackToken::CpuAffinityRestoreFile {
        path: PathBuf::from(format!("/tmp/stutter-ledger-test-{name}.json")),
        affected_tasks: 1,
    }
}

fn entry(name: &str) -> KeptLedgerEntry {
    KeptLedgerEntry {
        boot_id: stutter_core::boot::current_boot_id().ok(),
        experiment_id: ExperimentId::new(format!("experiment-{name}")),
        action_id: ActionId::new(format!("cpu-affinity-profile:{name}")),
        candidate: name.to_owned(),
        rollback_token: token(name),
        mode: None,
        safety_class: None,
        kept_unix_nanos: 1,
    }
}

fn config(name: &str) -> StartupRecoveryConfig {
    let dir = std::env::temp_dir().join(format!(
        "stutter-ledger-recovery-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    StartupRecoveryConfig {
        rollback_on_crash_recovery: true,
        journal_path: dir.join("controller_journal.json"),
        audit_path: dir.join("audit.jsonl"),
        history_path: dir.join("history.jsonl"),
        state_snapshot_path: dir.join("daemon-state.json"),
        privileged_worker_socket: None,
        worker_only_mutation: false,
    }
}

#[test]
fn every_kept_action_is_restored_once_and_retired() {
    let config = config("restore-all");
    let ledger = kept_ledger_path(&config.journal_path);
    record_kept(&ledger, entry("a")).unwrap();
    record_kept(&ledger, entry("b")).unwrap();
    let mut executor = CountingExecutor::default();

    let outcome = recover_controller_journal_with_executor(config, &mut executor).unwrap();

    assert!(matches!(outcome, StartupRecoveryOutcome::Recovered { .. }));
    assert_eq!(executor.restored.borrow().len(), 2);
    assert!(!ledger.exists(), "every entry was retired");
}

#[test]
fn a_failed_restore_keeps_the_entry_and_faults_the_start() {
    let config = config("restore-fails");
    let ledger = kept_ledger_path(&config.journal_path);
    record_kept(&ledger, entry("a")).unwrap();
    let mut executor = CountingExecutor {
        fail: true,
        ..CountingExecutor::default()
    };

    let outcome = recover_controller_journal_with_executor(config, &mut executor).unwrap();

    assert!(outcome.is_faulted(), "{outcome:?}");
    assert_eq!(read_kept_ledger(&ledger).unwrap().entries.len(), 1);
}

#[test]
fn the_journal_and_the_ledger_naming_one_token_restore_it_once() {
    let config = config("same-token");
    let ledger = kept_ledger_path(&config.journal_path);
    record_kept(&ledger, entry("a")).unwrap();
    crate::autotune::controller_journal::write_controller_journal_phase_with_metadata(
        &config.journal_path,
        ControllerJournalState::Keeping,
        ExperimentId::new("experiment-a"),
        ActionId::new("cpu-affinity-profile:a"),
        Some(token("a")),
        Default::default(),
    )
    .unwrap();
    let mut executor = CountingExecutor::default();

    recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

    assert_eq!(executor.restored.borrow().len(), 1);
    assert!(
        read_controller_journal(&config.journal_path)
            .unwrap()
            .is_clean()
    );
}

#[test]
fn earlier_boot_task_ledger_is_retired_without_calling_the_executor() {
    let config = config("boot-superseded");
    let mut old = entry("old-boot");
    old.boot_id = Some("00000000-0000-0000-0000-000000000000".into());
    record_kept(&kept_ledger_path(&config.journal_path), old).unwrap();
    let mut executor = CountingExecutor::default();
    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();
    assert!(matches!(
        outcome,
        StartupRecoveryOutcome::Recovered {
            affected_tasks: 0,
            ..
        }
    ));
    assert!(executor.restored.borrow().is_empty());
    assert!(
        read_kept_ledger(&kept_ledger_path(&config.journal_path))
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(
        std::fs::read_to_string(config.audit_path)
            .unwrap()
            .contains("boot_superseded")
    );
}

#[test]
fn earlier_boot_journal_is_retired_and_missing_boot_identity_faults() {
    let config = config("journal-boot-policy");
    let mut record = crate::autotune::controller_journal::ControllerJournalRecord::applied(
        ExperimentId::new("boot-test"),
        ActionId::new("cpu-affinity-profile:boot-test"),
        token("boot-test"),
    );
    record.boot_id = Some("00000000-0000-0000-0000-000000000000".into());
    crate::autotune::controller_journal::write_controller_journal_record(
        &config.journal_path,
        &record,
    )
    .unwrap();
    let mut executor = CountingExecutor::default();
    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();
    assert!(matches!(
        outcome,
        StartupRecoveryOutcome::Recovered {
            affected_tasks: 0,
            ..
        }
    ));
    assert!(executor.restored.borrow().is_empty());
    assert!(
        read_controller_journal(&config.journal_path)
            .unwrap()
            .is_clean()
    );
    // Legacy JSON bypasses the new-write validation, exercising the migration fail-closed path.
    record.boot_id = None;
    std::fs::write(&config.journal_path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(
        recover_controller_journal_with_executor(config, &mut executor)
            .unwrap()
            .is_faulted()
    );
    assert!(executor.restored.borrow().is_empty());
}
