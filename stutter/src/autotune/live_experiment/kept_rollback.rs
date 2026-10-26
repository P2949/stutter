//! Durable bookkeeping for kept actions: each is written to the kept-action ledger before it counts
//! as kept, and released (rolled back, then retired) when the workload or the controller ends.
use super::{
    journal::{controller_journal_path, write_controller_journal_phase_for_live_experiment},
    *,
};
use crate::{
    audit::{AuditEvent, append_audit_event_to_path},
    autotune::{
        controller_journal::{
            ControllerJournalState, read_controller_journal, write_controller_journal_clean,
        },
        kept::ActiveProfileState,
        kept_ledger::{KeptLedgerEntry, kept_ledger_path, record_kept, retire_kept},
        objective::ObjectiveSignals,
        observation::AutotuneObservation,
    },
};

/// What a release did. A failed action stays kept, in memory and in the ledger.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct KeptRelease {
    pub(crate) restored: usize,
    pub(crate) failed: usize,
}

impl LiveExperimentManager {
    /// Make a keep durable: the ledger entry first, then the journal's Keeping phase. The journal
    /// only ever names the experiment in flight, so the ledger is what keeps an earlier kept
    /// action recoverable once the next experiment starts. A failure leaves nothing half-kept.
    pub(super) fn commit_keep_durably(
        input: &LiveExperimentManagerInput<'_>,
        experiment: &LiveExperiment,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<()> {
        Self::record_kept_durably(input, experiment, observation.now_unix_nanos)?;
        write_controller_journal_phase_for_live_experiment(
            input,
            experiment,
            observation,
            ControllerJournalState::Keeping,
            "kept_pending_manual_restore",
        )
        .inspect_err(|_| Self::retire_kept_durably(input, &experiment.experiment_id))
    }

    fn record_kept_durably(
        input: &LiveExperimentManagerInput<'_>,
        experiment: &LiveExperiment,
        kept_unix_nanos: u128,
    ) -> anyhow::Result<()> {
        if input.simulate_action_effects && input.controller_journal_path.is_none() {
            return Ok(());
        }
        record_kept(
            &kept_ledger_path(&controller_journal_path(input)),
            KeptLedgerEntry {
                experiment_id: experiment.experiment_id.clone(),
                action_id: experiment.action_id(),
                candidate: experiment.candidate.profile_name().to_owned(),
                rollback_token: experiment.rollback.clone(),
                mode: Some(experiment.mode),
                safety_class: Some(experiment.safety_class.clone()),
                kept_unix_nanos,
            },
        )
    }

    pub(super) fn retire_kept_durably(
        input: &LiveExperimentManagerInput<'_>,
        experiment_id: &crate::autotune::experiment::ExperimentId,
    ) {
        if let Err(error) = retire_kept(
            &kept_ledger_path(&controller_journal_path(input)),
            experiment_id,
        ) {
            log::warn!("kept_ledger_retire_failed experiment_id={experiment_id}: {error:#}");
        }
    }

    /// Roll back every kept action through the same executor an experiment uses. A failure keeps
    /// the action (and its ledger entry) so nothing applied is ever forgotten.
    pub(crate) fn release_kept_actions_with_executor<E: LiveExperimentActionExecutor + ?Sized>(
        input: &LiveExperimentManagerInput<'_>,
        profile: &mut ActiveProfileState,
        observation: &AutotuneObservation,
        reason: &str,
        executor: &mut E,
    ) -> KeptRelease {
        let mut release = KeptRelease::default();
        let groups: Vec<_> = profile.kept_actions().map(|(group, _)| *group).collect();
        for group in groups {
            let Some(kept) = profile.kept_actions.get(group).cloned() else {
                continue;
            };
            let experiment = LiveExperiment {
                experiment_id: kept.experiment_id.clone(),
                candidate: kept.candidate.clone(),
                safety_class: kept.candidate.safety_class(),
                mode: input.mode,
                baseline_score: kept.baseline_score.clone(),
                baseline_signals: ObjectiveSignals::default(),
                baseline_active_config: None,
                applied_unix_nanos: kept.kept_unix_nanos,
                washout_until_unix_nanos: kept.kept_unix_nanos,
                measure_until_unix_nanos: kept.kept_unix_nanos,
                rollback: kept.rollback.clone(),
            };
            // Simulated effects never reached the host, so there is nothing to restore.
            let result = if input.simulate_action_effects {
                Ok(None)
            } else {
                executor.rollback_candidate(input, &experiment, observation)
            };
            let success = result.is_ok();
            let message = match &result {
                Ok(_) => format!("kept action released: {reason}"),
                Err(error) => format!("kept action release failed: {reason}: {error:#}"),
            };
            write_release_audit(input, &experiment, success, message);
            if success {
                release.restored += 1;
                Self::retire_kept_durably(input, &kept.experiment_id);
                clear_journal_if_it_names(input, &kept.rollback);
                profile.kept_actions.remove(group);
            } else {
                release.failed += 1;
            }
        }
        release
    }
}

/// The journal keeps a `Keeping` record for the latest kept action; once that action is restored
/// the record would only make startup recovery restore it a second time.
fn clear_journal_if_it_names(
    input: &LiveExperimentManagerInput<'_>,
    token: &crate::actions::RollbackToken,
) {
    let path = controller_journal_path(input);
    let Ok(record) = read_controller_journal(&path) else {
        return;
    };
    if record.state() == ControllerJournalState::Keeping
        && record.rollback_token() == Some(token)
        && let Err(error) = write_controller_journal_clean(&path)
    {
        log::warn!("controller_journal_clear_failed after kept release: {error:#}");
    }
}

fn write_release_audit(
    input: &LiveExperimentManagerInput<'_>,
    experiment: &LiveExperiment,
    success: bool,
    message: String,
) {
    let path = input
        .audit_path
        .clone()
        .unwrap_or_else(crate::audit::default_audit_log_path);
    let event = AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "autotune-kept-release".to_owned(),
        action_id: Some(experiment.action_id()),
        safety_class: Some(experiment.safety_class.clone()),
        dry_run: false,
        success,
        affected_tasks: experiment.rollback.affected_tasks(),
        restore_path: experiment.rollback.restore_path().cloned(),
        action_phase: None,
        error_category: None,
        operation: Some(experiment.candidate.operation_kind()),
        message,
    };
    if let Err(error) = append_audit_event_to_path(&path, &event) {
        log::warn!("kept_release_audit_failed: {error:#}");
    }
}
