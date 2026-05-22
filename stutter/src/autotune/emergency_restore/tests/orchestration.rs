use std::fs;

use super::{super::*, support::*};
use crate::{
    actions::*,
    autotune::{controller_journal::*, history::*},
};

#[test]
fn clean_journal_reports_no_active_action() {
    let dir = temp_dir("clean");
    let input = command_input_for_dir(&dir, false);
    write_controller_journal_clean(input.journal_path.as_deref().unwrap()).unwrap();

    let outcome = restore_known_autotune_actions(input).unwrap();

    assert_eq!(outcome.status, AutotuneRestoreStatus::Clean);
    assert_eq!(outcome.restored_actions, 0);
    assert!(outcome.messages[0].contains("no active autotune action"));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn reverted_journal_reports_clean_and_normalizes_journal() {
    let dir = temp_dir("reverted");
    let input = command_input_for_dir(&dir, false);
    let journal_path = input.journal_path.clone().unwrap();
    let record = ControllerJournalRecord::for_phase(
        ControllerJournalState::Reverted,
        "experiment-live",
        "cpu-affinity-profile:game-main",
        Some(RollbackToken::CpuAffinityRestoreFile {
            path: dir.join("restore.json"),
            affected_tasks: 1,
        }),
    );
    write_controller_journal_record(&journal_path, &record).unwrap();

    let outcome = restore_known_autotune_actions(input).unwrap();

    assert_eq!(outcome.status, AutotuneRestoreStatus::Clean);
    assert_eq!(outcome.restored_actions, 0);
    assert!(read_controller_journal(&journal_path).unwrap().is_clean());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn applying_journal_reports_no_rollback_token() {
    let dir = temp_dir("applying");
    let input = command_input_for_dir(&dir, false);
    write_controller_journal_applying(
        input.journal_path.as_deref().unwrap(),
        "experiment-1",
        "cpu-affinity-profile:game-main",
    )
    .unwrap();

    let outcome = restore_known_autotune_actions(input).unwrap();

    assert_eq!(
        outcome.status,
        AutotuneRestoreStatus::ApplyingWithoutRollbackToken
    );
    assert_eq!(outcome.skipped_actions, 1);
    assert!(outcome.messages[0].contains("without rollback_token"));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn dry_run_for_applied_journal_does_not_clean_journal() {
    let dir = temp_dir("dry-run");
    let input = command_input_for_dir(&dir, true);
    let journal_path = input.journal_path.clone().unwrap();
    write_controller_journal_applied(
        &journal_path,
        "experiment-1",
        "nice:set:5:targets:1",
        RollbackToken::NiceRestore {
            records: vec![NiceRestoreRecord::new(
                TaskRestoreIdentity::observed(123, None, Some("test".to_owned()), None, None),
                0,
            )],
        },
    )
    .unwrap();

    let outcome = restore_known_autotune_actions(input).unwrap();

    assert_eq!(outcome.status, AutotuneRestoreStatus::DryRun);
    assert_eq!(outcome.skipped_actions, 1);
    assert!(
        outcome
            .messages
            .iter()
            .any(|message| { message.contains("sudo renice -n 0 -p 123") })
    );
    assert!(!read_controller_journal(&journal_path).unwrap().is_clean());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn dry_run_for_live_runtime_phases_does_not_clean_journal() {
    for phase in [
        ControllerJournalState::Measuring,
        ControllerJournalState::Keeping,
        ControllerJournalState::Reverting,
    ] {
        let dir = temp_dir(phase.as_str());
        let input = command_input_for_dir(&dir, true);
        let journal_path = input.journal_path.clone().unwrap();
        let record = ControllerJournalRecord::for_phase(
            phase,
            "experiment-live",
            "nice:set:5:targets:1",
            Some(RollbackToken::NiceRestore {
                records: vec![NiceRestoreRecord::new(
                    TaskRestoreIdentity::observed(123, None, Some("test".to_owned()), None, None),
                    0,
                )],
            }),
        )
        .with_candidate("game-main");
        write_controller_journal_record(&journal_path, &record).unwrap();

        let outcome = restore_known_autotune_actions(input).unwrap();

        assert_eq!(outcome.status, AutotuneRestoreStatus::DryRun);
        assert_eq!(outcome.skipped_actions, 1);
        assert!(!read_controller_journal(&journal_path).unwrap().is_clean());
        fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn default_rollback_registry_previews_and_restores_sysfs_token() {
    let dir = temp_dir("registry-sysfs-token");
    let target = dir.join("sysfs-knob");
    fs::write(&target, "changed").unwrap();

    let token = RollbackToken::SysfsRestore {
        path: target.clone(),
        original_value: "original".to_owned(),
    };
    let registry = default_autotune_rollback_registry();

    let preview = registry.preview_token(&token).unwrap();
    assert_eq!(preview.handler_id, "vm-knob-rollback");
    assert_eq!(preview.restore_path, target);
    assert_eq!(preview.affected_tasks, 1);
    assert!(preview.message.contains("rollback_kind=sysfs-restore"));

    let result = registry.restore_token(&token).unwrap();
    assert_eq!(result.handler_id, "vm-knob-rollback");
    assert_eq!(result.restored, 1);
    assert_eq!(result.errors, 0);
    assert_eq!(fs::read_to_string(&target).unwrap(), "original");

    fs::remove_dir_all(dir).ok();
}

#[test]
fn sysfs_restore_token_restores_file_and_cleans_journal_and_writes_logs() {
    let dir = temp_dir("sysfs");
    let target = dir.join("sysfs-knob");
    fs::write(&target, "changed").unwrap();

    let input = command_input_for_dir(&dir, false);
    let journal_path = input.journal_path.clone().unwrap();
    let audit_path = input.audit_path.clone().unwrap();
    let history_path = input.history_path.clone().unwrap();

    write_controller_journal_applied(
        &journal_path,
        "experiment-1",
        "sysfs-restore:test",
        RollbackToken::SysfsRestore {
            path: target.clone(),
            original_value: "original".to_owned(),
        },
    )
    .unwrap();

    let outcome = restore_known_autotune_actions(input).unwrap();

    assert_eq!(outcome.status, AutotuneRestoreStatus::Restored);
    assert_eq!(outcome.restored_actions, 1);
    assert!(outcome.messages.iter().any(|message| {
        message.contains("restored_items=1") && message.contains("skipped_items=0")
    }));
    assert_eq!(fs::read_to_string(&target).unwrap(), "original");
    assert!(read_controller_journal(&journal_path).unwrap().is_clean());

    let audit = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
    assert_eq!(audit.len(), 1);
    assert!(audit[0].success);
    assert_eq!(audit[0].command, "autotune emergency restore");

    let history = crate::autotune::history::read_autotune_history_events(&history_path).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].phase, ControllerPhase::Cooldown);
    assert_eq!(history[0].decision.decision, "restored");
    assert!(history[0].rollback_performed);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn keeping_journal_restores_file_and_cleans_journal() {
    let dir = temp_dir("keeping-sysfs");
    let target = dir.join("sysfs-knob");
    fs::write(&target, "changed").unwrap();

    let input = command_input_for_dir(&dir, false);
    let journal_path = input.journal_path.clone().unwrap();
    let record = ControllerJournalRecord::for_phase(
        ControllerJournalState::Keeping,
        "experiment-live",
        "sysfs-restore:test",
        Some(RollbackToken::SysfsRestore {
            path: target.clone(),
            original_value: "original".to_owned(),
        }),
    )
    .with_restore_command("stutter autotune restore");
    write_controller_journal_record(&journal_path, &record).unwrap();

    let outcome = restore_known_autotune_actions(input).unwrap();

    assert_eq!(outcome.status, AutotuneRestoreStatus::Restored);
    assert_eq!(outcome.restored_actions, 1);
    assert_eq!(fs::read_to_string(&target).unwrap(), "original");
    assert!(read_controller_journal(&journal_path).unwrap().is_clean());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn sysfs_restore_failure_keeps_journal_and_writes_fault_logs() {
    let dir = temp_dir("sysfs-failure");
    let missing_parent = dir.join("missing");
    let target = missing_parent.join("knob");

    let input = command_input_for_dir(&dir, false);
    let journal_path = input.journal_path.clone().unwrap();
    let audit_path = input.audit_path.clone().unwrap();
    let history_path = input.history_path.clone().unwrap();

    write_controller_journal_applied(
        &journal_path,
        "experiment-1",
        "sysfs-restore:test",
        RollbackToken::SysfsRestore {
            path: target.clone(),
            original_value: "original".to_owned(),
        },
    )
    .unwrap();

    let outcome = restore_known_autotune_actions(input).unwrap();

    assert_eq!(outcome.status, AutotuneRestoreStatus::Faulted);
    assert_eq!(outcome.failed_actions, 1);
    assert!(!read_controller_journal(&journal_path).unwrap().is_clean());

    let audit = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
    assert_eq!(audit.len(), 1);
    assert!(!audit[0].success);

    let history = crate::autotune::history::read_autotune_history_events(&history_path).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].phase, ControllerPhase::Faulted);
    assert_eq!(history[0].decision.decision, "EmergencyRestoreFault");
    assert!(!history[0].rollback_performed);

    fs::remove_dir_all(dir).ok();
}
