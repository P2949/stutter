use super::*;

#[test]
fn clean_journal_needs_no_recovery() {
    let dir = temp_dir("clean");
    let config = config_for_dir(&dir, true);
    write_controller_journal_clean(&config.journal_path).unwrap();
    let mut executor = FakeRollbackExecutor::default();

    let outcome = recover_controller_journal_with_executor(config, &mut executor).unwrap();

    assert_eq!(outcome, StartupRecoveryOutcome::Clean);
    assert_eq!(executor.calls, 0);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn pre_apply_transaction_phase_cleans_without_rollback() {
    let dir = temp_dir("planned");
    let config = config_for_dir(&dir, true);
    let record = ControllerJournalRecord::for_phase(
        ControllerJournalState::Planned,
        crate::autotune::experiment::ExperimentId::try_new("experiment-1").unwrap(),
        crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
        None,
    )
    .with_candidate("game-main");
    write_controller_journal_record(&config.journal_path, &record).unwrap();
    let mut executor = FakeRollbackExecutor::default();

    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

    assert_eq!(outcome, StartupRecoveryOutcome::Clean);
    assert_eq!(executor.calls, 0);
    assert!(
        crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
            .unwrap()
            .is_clean()
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn reverted_transaction_phase_cleans_without_rollback() {
    let dir = temp_dir("reverted");
    let config = config_for_dir(&dir, true);
    let record = ControllerJournalRecord::for_phase(
        ControllerJournalState::Reverted,
        crate::autotune::experiment::ExperimentId::try_new("experiment-live").unwrap(),
        crate::actions::ActionId::try_new("cpu-affinity-profile:game-main").unwrap(),
        Some(rollback_token()),
    );
    write_controller_journal_record(&config.journal_path, &record).unwrap();
    let mut executor = FakeRollbackExecutor {
        calls: 0,
        fail: false,
        affected_tasks: 31,
    };

    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

    assert_eq!(outcome, StartupRecoveryOutcome::Clean);
    assert_eq!(executor.calls, 0);
    assert!(
        crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
            .unwrap()
            .is_clean()
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn journal_clean_state_does_nothing() {
    let dir = temp_dir("phase-15-5-clean-does-nothing");
    let config = config_for_dir(&dir, true);

    write_controller_journal_clean(&config.journal_path).unwrap();

    let mut executor = FakeRollbackExecutor {
        calls: 0,
        fail: false,
        affected_tasks: 31,
    };

    let outcome = recover_controller_journal_with_executor(config.clone(), &mut executor).unwrap();

    assert_eq!(outcome, StartupRecoveryOutcome::Clean);
    assert_eq!(executor.calls, 0);

    assert!(
        crate::autotune::controller_journal::read_controller_journal(&config.journal_path)
            .unwrap()
            .is_clean()
    );
    assert!(
        !config.audit_path.exists(),
        "clean recovery must not write an audit event"
    );
    assert!(
        !config.history_path.exists(),
        "clean recovery must not write a history event"
    );

    fs::remove_dir_all(dir).ok();
}
