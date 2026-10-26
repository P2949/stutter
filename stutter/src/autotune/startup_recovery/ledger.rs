//! Startup recovery of kept actions from the durable kept-action ledger.
use super::{
    RecoverySource, StartupRecoveryConfig, StartupRecoveryOutcome, StartupRecoveryRollbackExecutor,
    recover_applied_journal_record,
};
use crate::autotune::kept_ledger::{kept_ledger_path, read_kept_ledger};

/// Restore each ledger entry. An unreadable ledger is an error: it must never look empty.
pub(super) fn recover_kept_ledger<E: StartupRecoveryRollbackExecutor + ?Sized>(
    config: &StartupRecoveryConfig,
    executor: &mut E,
) -> anyhow::Result<Vec<StartupRecoveryOutcome>> {
    let ledger = read_kept_ledger(&kept_ledger_path(&config.journal_path))?;
    ledger
        .entries
        .into_iter()
        .map(|entry| {
            recover_applied_journal_record(
                config.clone(),
                executor,
                entry.experiment_id,
                entry.action_id,
                entry.rollback_token,
                RecoverySource::Ledger,
            )
        })
        .collect()
}

/// One outcome for the caller: a fault anywhere wins, then any recovery, then the journal's own.
pub(super) fn combine(
    kept: Vec<StartupRecoveryOutcome>,
    journal: StartupRecoveryOutcome,
) -> StartupRecoveryOutcome {
    if journal.is_faulted() {
        return journal;
    }
    if let Some(faulted) = kept.iter().find(|outcome| outcome.is_faulted()) {
        return faulted.clone();
    }
    if journal != StartupRecoveryOutcome::Clean {
        return journal;
    }
    kept.into_iter().next().unwrap_or(journal)
}
