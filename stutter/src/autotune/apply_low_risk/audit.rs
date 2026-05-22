//! Audit-backed application helpers for low-risk autotune candidates.
//!
//! Owns candidate-to-action conversion, action safety gating, audited apply orchestration, and
//! test-only controller-journal/history helpers. It must not own target selection, experiment
//! scoring, or CLI command dispatch.

#[cfg(test)]
use std::path::Path;

use anyhow::Context;

use super::{model::AuditedCandidateApplyOutcome, unsupported_low_risk_candidate};
#[cfg(test)]
use crate::actions::RollbackToken;
#[cfg(test)]
use crate::actions::runner::run_audited_action_with_audit_path;
#[cfg(test)]
use crate::autotune::controller_journal::{
    ControllerJournalActionMetadata, journal_process_identity,
    write_controller_journal_applied_with_metadata, write_controller_journal_clean,
};
use crate::{
    actions::{
        SafetyClass, TuningAction,
        cpu_affinity::CpuAffinityProfileAction,
        runner::{
            ActionHooks, ActionRunPolicy, AuditedActionResult, run_audited_action_with_hooks,
        },
    },
    autotune::planning::candidate::CandidateAction,
};

#[cfg(test)]
pub struct AuditedRollbackGuard<'a, A: TuningAction + ?Sized> {
    action: &'a A,
    token: Option<RollbackToken>,
    rollback_performed: bool,
}

#[cfg(test)]
impl<'a, A: TuningAction + ?Sized> AuditedRollbackGuard<'a, A> {
    pub fn new(action: &'a A, token: RollbackToken) -> Self {
        Self {
            action,
            token: Some(token),
            rollback_performed: false,
        }
    }

    pub fn rollback_now(&mut self) -> anyhow::Result<()> {
        if let Some(token) = self.token.take() {
            self.action.rollback(&token)?;
            self.rollback_performed = true;
        }
        Ok(())
    }

    pub fn rollback_performed(&self) -> bool {
        self.rollback_performed
    }
}

#[cfg(test)]
impl<A: TuningAction + ?Sized> Drop for AuditedRollbackGuard<'_, A> {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            if let Err(err) = self.action.rollback(&token) {
                log::error!("autotune_candidate_rollback_failed error={err:#}");
            } else {
                self.rollback_performed = true;
            }
        }
    }
}

pub fn action_from_candidate(
    candidate: CandidateAction,
) -> anyhow::Result<(String, CpuAffinityProfileAction)> {
    match candidate {
        CandidateAction::CpuAffinityProfile { plan } => Ok((
            plan.profile_name,
            CpuAffinityProfileAction {
                tree_pid: plan.tree_pid,
                profile: plan.profile,
                force_restore_overwrite: false,
            },
        )),
        other => unsupported_low_risk_candidate(&other),
    }
}

#[cfg(test)]
pub fn append_low_risk_history_event(
    path: &std::path::Path,
    event: &crate::autotune::history::AutotuneHistoryEvent,
) -> anyhow::Result<()> {
    crate::autotune::history::append_autotune_history_event(path, event)
}

pub fn apply_cpu_affinity_candidate_with_audit(
    candidate_name: String,
    action: &CpuAffinityProfileAction,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    apply_cpu_affinity_candidate_with_audit_hooks(candidate_name, action, ActionHooks::none())
}

pub(crate) fn apply_cpu_affinity_candidate_with_audit_hooks(
    candidate_name: String,
    action: &CpuAffinityProfileAction,
    hooks: ActionHooks<'_>,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    ensure_low_risk_action_allowed("cpu_affinity_profile", &action.safety_class())?;

    let run_policy =
        ActionRunPolicy::apply_low_risk(crate::daemon_policy::ActionSource::AutotuneRuntime, false);
    let AuditedActionResult {
        state, rollback, ..
    } = run_audited_action_with_hooks("autotune candidate", action, run_policy, hooks)
        .with_context(|| {
            format!(
                "audited apply failed for autotune candidate '{}'",
                candidate_name
            )
        })?;

    let rollback = rollback.with_context(|| {
        format!(
            "audited apply for autotune candidate '{}' succeeded without rollback token",
            candidate_name
        )
    })?;

    Ok(AuditedCandidateApplyOutcome {
        candidate_name,
        action_kind: "cpu_affinity_profile".to_owned(),
        affected_tasks: rollback.affected_tasks(),
        safety_class: action.safety_class(),
        state,
        rollback,
    })
}

#[cfg(test)]
pub fn apply_cpu_affinity_candidate_with_audit_path_for_tests(
    candidate_name: String,
    action: &CpuAffinityProfileAction,
    audit_path: &std::path::Path,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    ensure_low_risk_action_allowed("cpu_affinity_profile", &action.safety_class())?;

    let run_policy =
        ActionRunPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test, false);
    let AuditedActionResult {
        state, rollback, ..
    } = run_audited_action_with_audit_path("autotune candidate", action, run_policy, audit_path)
        .with_context(|| {
            format!(
                "audited apply failed for autotune candidate '{}'",
                candidate_name
            )
        })?;

    let rollback = rollback.with_context(|| {
        format!(
            "audited apply for autotune candidate '{}' succeeded without rollback token",
            candidate_name
        )
    })?;

    Ok(AuditedCandidateApplyOutcome {
        candidate_name,
        action_kind: "cpu_affinity_profile".to_owned(),
        affected_tasks: rollback.affected_tasks(),
        safety_class: action.safety_class(),
        state,
        rollback,
    })
}

pub fn ensure_low_risk_action_allowed(
    action_kind: &str,
    safety_class: &SafetyClass,
) -> anyhow::Result<()> {
    if action_kind != "cpu_affinity_profile" {
        anyhow::bail!(
            "apply-low-risk currently supports CPU-affinity profile actions only; blocked action_kind={}",
            action_kind
        );
    }

    if *safety_class != SafetyClass::ReversibleLowRisk {
        anyhow::bail!(
            "apply-low-risk currently supports ReversibleLowRisk CPU-affinity profile actions only; blocked safety_class={:?}",
            safety_class
        );
    }

    Ok(())
}

#[cfg(test)]
pub(crate) fn controller_journal_metadata_for_cpu_affinity_action(
    candidate_name: &str,
    action: &CpuAffinityProfileAction,
    active_task_count: Option<usize>,
    verify_result: &'static str,
) -> ControllerJournalActionMetadata {
    let starttime_ticks =
        crate::process_tree::process_starttime_at(Path::new("/proc"), action.tree_pid);

    ControllerJournalActionMetadata::default()
        .with_candidate(candidate_name.to_owned())
        .with_workload_identity(journal_process_identity(
            action.tree_pid,
            starttime_ticks,
            None,
        ))
        .with_target_identity(journal_process_identity(
            action.tree_pid,
            starttime_ticks,
            active_task_count,
        ))
        .with_restore_command("stutter autotune restore")
        .with_verify_result(verify_result)
        .with_mode(crate::daemon_policy::DaemonMode::ApplyLowRisk)
        .with_safety_class(action.safety_class())
}

#[cfg(test)]
pub(crate) fn controller_journal_hooks_for_low_risk_action<'a>(
    journal_path: &'a Path,
    experiment_id: &'a str,
    action_id: &'a str,
    candidate_name: &'a str,
    action: &'a CpuAffinityProfileAction,
) -> ActionHooks<'a> {
    ActionHooks::after_apply(move |rollback| {
        write_controller_journal_applied_with_metadata(
            journal_path,
            experiment_id,
            action_id,
            rollback.clone(),
            controller_journal_metadata_for_cpu_affinity_action(
                candidate_name,
                action,
                Some(rollback.affected_tasks()),
                "applied_pending_verify",
            ),
        )
        .with_context(|| {
            format!(
                "failed to write applied controller journal for autotune candidate '{}'",
                candidate_name
            )
        })?;

        Ok(())
    })
    .with_after_rollback(move |_rollback| {
        write_controller_journal_clean(journal_path).with_context(|| {
            format!(
                "failed to write clean controller journal after automatic rollback for autotune candidate '{}'",
                candidate_name
            )
        })?;

        Ok(())
    })
}
