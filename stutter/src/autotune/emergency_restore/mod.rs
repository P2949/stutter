mod types;
mod handler;
mod executors;
mod audit;
mod manual_command;
mod helpers;
#[cfg(test)]
mod tests;

pub use types::*;
pub use handler::default_autotune_rollback_registry;
use handler::*;
pub use executors::*;
use audit::*;
pub use manual_command::*;

use std::path::Path;
use anyhow::Context;
use crate::actions::*;
use crate::autotune::controller_journal::*;
use crate::autotune::history::*;

pub fn autotune_restore_command(input: AutotuneRestoreCommandInput) -> anyhow::Result<()> {
    let outcome = restore_known_autotune_actions(input)?;

    for message in &outcome.messages {
        println!("{message}");
    }

    if outcome.status == AutotuneRestoreStatus::Faulted {
        anyhow::bail!(
            "autotune emergency restore failed: restored_actions={} failed_actions={} skipped_actions={}",
            outcome.restored_actions,
            outcome.failed_actions,
            outcome.skipped_actions
        );
    }

    Ok(())
}

pub fn restore_known_autotune_actions(
    input: AutotuneRestoreCommandInput,
) -> anyhow::Result<AutotuneRestoreOutcome> {
    let journal_path = input
        .journal_path
        .unwrap_or_else(default_controller_journal_path);
    let audit_path = input
        .audit_path
        .unwrap_or_else(crate::audit::default_audit_log_path);
    let history_path = input
        .history_path
        .unwrap_or_else(default_autotune_history_path);

    let record = read_controller_journal(&journal_path)?;

    match record.state() {
        ControllerJournalState::Clean
        | ControllerJournalState::Planned
        | ControllerJournalState::Preflighted => Ok(AutotuneRestoreOutcome {
            status: AutotuneRestoreStatus::Clean,
            restored_actions: 0,
            failed_actions: 0,
            skipped_actions: 0,
            restored_records: 0,
            skipped_missing: 0,
            skipped_identity_mismatch: 0,
            failed_records: 0,
            messages: vec![format!(
                "autotune restore: no active autotune action in {}",
                journal_path.display()
            )],
        }),
        ControllerJournalState::Reverted => {
            if !input.dry_run {
                write_controller_journal_clean(&journal_path).with_context(|| {
                    format!(
                        "failed to clear reverted controller journal {}",
                        journal_path.display()
                    )
                })?;
            }
            Ok(AutotuneRestoreOutcome {
                status: AutotuneRestoreStatus::Clean,
                restored_actions: 0,
                failed_actions: 0,
                skipped_actions: 0,
                restored_records: 0,
                skipped_missing: 0,
                skipped_identity_mismatch: 0,
                failed_records: 0,
                messages: vec![format!(
                    "autotune restore: no active autotune action in {}",
                    journal_path.display()
                )],
            })
        }
        ControllerJournalState::Applying => {
            let (experiment_id, action_id) = journal_experiment_action(&record);
            Ok(AutotuneRestoreOutcome {
                status: AutotuneRestoreStatus::ApplyingWithoutRollbackToken,
                restored_actions: 0,
                failed_actions: 0,
                skipped_actions: 1,
                restored_records: 0,
                skipped_missing: 0,
                skipped_identity_mismatch: 0,
                failed_records: 0,
                messages: vec![format!(
                    "autotune restore: journal is applying without rollback_token experiment_id={} action_id={}; no automatic restore is possible",
                    experiment_id, action_id
                )],
            })
        }
        ControllerJournalState::Applied
        | ControllerJournalState::Verifying
        | ControllerJournalState::Measuring
        | ControllerJournalState::Keeping
        | ControllerJournalState::Reverting
        | ControllerJournalState::Faulted => {
            let (experiment_id, action_id) = journal_experiment_action(&record);
            if let Some(rollback_token) = record.rollback_token() {
                restore_applied_journal_record(RestoreAppliedJournalInput {
                    journal_path: &journal_path,
                    audit_path: &audit_path,
                    history_path: &history_path,
                    experiment_id: &experiment_id,
                    action_id: &action_id,
                    rollback_token,
                    dry_run: input.dry_run,
                })
            } else {
                Ok(AutotuneRestoreOutcome {
                    status: AutotuneRestoreStatus::ApplyingWithoutRollbackToken,
                    restored_actions: 0,
                    failed_actions: 0,
                    skipped_actions: 1,
                    restored_records: 0,
                    skipped_missing: 0,
                    skipped_identity_mismatch: 0,
                    failed_records: 0,
                    messages: vec![format!(
                        "autotune restore: journal is {} without rollback_token experiment_id={} action_id={}; no automatic restore is possible",
                        record.state().as_str(),
                        experiment_id,
                        action_id
                    )],
                })
            }
        }
    }
}

fn journal_experiment_action(record: &ControllerJournalRecord) -> (String, String) {
    let state = record.state().as_str();
    (
        record
            .experiment_id
            .clone()
            .unwrap_or_else(|| format!("{state}-unknown-experiment")),
        record
            .action_id
            .clone()
            .unwrap_or_else(|| format!("{state}-unknown-action")),
    )
}

struct RestoreAppliedJournalInput<'a> {
    journal_path: &'a Path,
    audit_path: &'a Path,
    history_path: &'a Path,
    experiment_id: &'a str,
    action_id: &'a str,
    rollback_token: &'a RollbackToken,
    dry_run: bool,
}

fn restore_applied_journal_record(
    input: RestoreAppliedJournalInput<'_>,
) -> anyhow::Result<AutotuneRestoreOutcome> {
    let RestoreAppliedJournalInput {
        journal_path,
        audit_path,
        history_path,
        experiment_id,
        action_id,
        rollback_token,
        dry_run,
    } = input;
    let registry = default_autotune_rollback_registry();
    let manual_command = manual_restore_command_for_token(rollback_token);
    let rollback_kind = rollback_token_kind(rollback_token);

    if dry_run {
        let preview = registry.preview_token(rollback_token)?;
        return Ok(AutotuneRestoreOutcome {
            status: AutotuneRestoreStatus::DryRun,
            restored_actions: 0,
            failed_actions: 0,
            skipped_actions: 1,
            restored_records: 0,
            skipped_missing: 0,
            skipped_identity_mismatch: 0,
            failed_records: 0,
            messages: vec![
                format!(
                    "autotune restore dry-run: would restore experiment_id={} action_id={} rollback_kind={} affected_tasks={} manual_restore_command=\"{}\"",
                    experiment_id, action_id, rollback_kind, preview.affected_tasks, manual_command
                ),
                preview.message,
            ],
        });
    }

    match registry.restore_token(rollback_token) {
        Ok(result) => {
            let summary = rollback_restore_summary_from_registry_result(rollback_token, result);
            if summary.failed_items > 0 {
                write_emergency_restore_audit_event(
                    audit_path,
                    action_id,
                    rollback_token,
                    false,
                    summary.restored_items,
                    format!(
                        "autotune emergency restore incomplete experiment_id={} action_id={} rollback_kind={} restored_items={} skipped_items={} failed_items={} manual_restore_command=\"{}\"{}",
                        experiment_id,
                        action_id,
                        summary.rollback_kind,
                        summary.restored_items,
                        summary.skipped_items,
                        summary.failed_items,
                        manual_command,
                        render_summary_messages(&summary.messages)
                    ),
                )?;

                write_emergency_restore_history_event(EmergencyRestoreHistoryEventInput {
                    history_path,
                    phase: ControllerPhase::Faulted,
                    decision: "EmergencyRestoreFault",
                    experiment_id,
                    action_id,
                    rollback_token,
                    rollback_performed: false,
                    reason: format!(
                        "autotune emergency restore incomplete rollback_kind={} restored_items={} skipped_items={} failed_items={} manual_restore_command=\"{}\"",
                        summary.rollback_kind,
                        summary.restored_items,
                        summary.skipped_items,
                        summary.failed_items,
                        manual_command
                    ),
                })?;

                return Ok(AutotuneRestoreOutcome {
                    status: AutotuneRestoreStatus::Faulted,
                    restored_actions: 0,
                    failed_actions: 1,
                    skipped_actions: 0,
                    restored_records: summary.restored_items,
                    skipped_missing: summary.skipped_missing,
                    skipped_identity_mismatch: summary.skipped_identity_mismatch,
                    failed_records: summary.failed_items,
                    messages: vec![format!(
                        "autotune restore failed: experiment_id={} action_id={} rollback_kind={} restored_items={} skipped_items={} failed_items={}",
                        experiment_id,
                        action_id,
                        summary.rollback_kind,
                        summary.restored_items,
                        summary.skipped_items,
                        summary.failed_items
                    )],
                });
            }

            write_controller_journal_clean(journal_path).with_context(|| {
                format!(
                    "failed to clear controller journal after emergency restore {}",
                    journal_path.display()
                )
            })?;

            write_emergency_restore_audit_event(
                audit_path,
                action_id,
                rollback_token,
                true,
                summary.restored_items,
                format!(
                    "autotune emergency restore succeeded experiment_id={} action_id={} rollback_kind={} restored_items={} skipped_items={} manual_restore_command=\"{}\"{}",
                    experiment_id,
                    action_id,
                    summary.rollback_kind,
                    summary.restored_items,
                    summary.skipped_items,
                    manual_command,
                    render_summary_messages(&summary.messages)
                ),
            )?;

            write_emergency_restore_history_event(EmergencyRestoreHistoryEventInput {
                history_path,
                phase: ControllerPhase::Cooldown,
                decision: "restored",
                experiment_id,
                action_id,
                rollback_token,
                rollback_performed: true,
                reason: format!(
                    "autotune emergency restore succeeded rollback_kind={} restored_items={} skipped_items={}",
                    summary.rollback_kind, summary.restored_items, summary.skipped_items
                ),
            })?;

            Ok(AutotuneRestoreOutcome {
                status: AutotuneRestoreStatus::Restored,
                restored_actions: 1,
                failed_actions: 0,
                skipped_actions: 0,
                restored_records: summary.restored_items,
                skipped_missing: summary.skipped_missing,
                skipped_identity_mismatch: summary.skipped_identity_mismatch,
                failed_records: summary.failed_items,
                messages: vec![format!(
                    "autotune restore: restored experiment_id={} action_id={} rollback_kind={} restored_items={} skipped_items={}",
                    experiment_id,
                    action_id,
                    summary.rollback_kind,
                    summary.restored_items,
                    summary.skipped_items
                )],
            })
        }
        Err(err) => {
            let reason = format!("{err:#}");
            write_emergency_restore_audit_event(
                audit_path,
                action_id,
                rollback_token,
                false,
                0,
                format!(
                    "autotune emergency restore failed experiment_id={} action_id={} rollback_kind={} error={} manual_restore_command=\"{}\"",
                    experiment_id, action_id, rollback_kind, reason, manual_command
                ),
            )?;

            write_emergency_restore_history_event(EmergencyRestoreHistoryEventInput {
                history_path,
                phase: ControllerPhase::Faulted,
                decision: "EmergencyRestoreFault",
                experiment_id,
                action_id,
                rollback_token,
                rollback_performed: false,
                reason: format!(
                    "autotune emergency restore failed rollback_kind={} error={} manual_restore_command=\"{}\"",
                    rollback_kind, reason, manual_command
                ),
            })?;

            Ok(AutotuneRestoreOutcome {
                status: AutotuneRestoreStatus::Faulted,
                restored_actions: 0,
                failed_actions: 1,
                skipped_actions: 0,
                restored_records: 0,
                skipped_missing: 0,
                skipped_identity_mismatch: 0,
                failed_records: 1,
                messages: vec![
                    format!(
                        "autotune restore failed: experiment_id={} action_id={} rollback_kind={} error={}",
                        experiment_id, action_id, rollback_kind, reason
                    ),
                    format!("manual_restore_command=\"{}\"", manual_command),
                ],
            })
        }
    }
}

