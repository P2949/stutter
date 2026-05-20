//! Test-only low-risk action executors and rollback harnesses.

use std::time::Duration;

use anyhow::Context;

use super::{
    ensure_low_risk_action_allowed, model::ApplyLowRiskOutcome, unsupported_low_risk_candidate,
};
use crate::{
    actions::{RollbackToken, SafetyClass, TuningAction, cpu_affinity::CpuAffinityProfileAction},
    autotune::{
        candidate::{CandidateAction, CandidateDryRunRecord},
        planning::dry_run::dry_run_record_from_action_state,
    },
};

#[cfg(test)]
pub trait LowRiskActionExecutor {
    fn candidate_name(&self) -> &str;
    fn action_kind(&self) -> &'static str;
    fn safety_class(&self) -> SafetyClass;
    fn dry_run(&mut self) -> anyhow::Result<CandidateDryRunRecord>;
    fn apply(&mut self) -> anyhow::Result<RollbackToken>;
    fn rollback(&mut self, token: &RollbackToken) -> anyhow::Result<()>;
}

#[cfg(test)]
pub struct CpuAffinityLowRiskExecutor {
    candidate_name: String,
    action: CpuAffinityProfileAction,
}

#[cfg(test)]
impl CpuAffinityLowRiskExecutor {
    pub fn from_candidate(candidate: CandidateAction) -> anyhow::Result<Self> {
        match candidate {
            CandidateAction::CpuAffinityProfile { plan } => Ok(Self {
                candidate_name: plan.profile_name,
                action: CpuAffinityProfileAction {
                    tree_pid: plan.tree_pid,
                    profile: plan.profile,
                    force_restore_overwrite: false,
                },
            }),
            other => unsupported_low_risk_candidate(&other),
        }
    }
}

#[cfg(test)]
impl LowRiskActionExecutor for CpuAffinityLowRiskExecutor {
    fn candidate_name(&self) -> &str {
        &self.candidate_name
    }

    fn action_kind(&self) -> &'static str {
        "cpu_affinity_profile"
    }

    fn safety_class(&self) -> SafetyClass {
        self.action.safety_class()
    }

    fn dry_run(&mut self) -> anyhow::Result<CandidateDryRunRecord> {
        let safety_class = self.action.safety_class();
        let state = self.action.dry_run()?;
        Ok(dry_run_record_from_action_state(
            self.candidate_name.clone(),
            safety_class,
            state,
        ))
    }

    fn apply(&mut self) -> anyhow::Result<RollbackToken> {
        Ok(self.action.apply()?)
    }

    fn rollback(&mut self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

#[cfg(test)]
pub fn executor_for_low_risk_candidate(
    candidate: CandidateAction,
) -> anyhow::Result<Box<dyn LowRiskActionExecutor>> {
    Ok(Box::new(CpuAffinityLowRiskExecutor::from_candidate(
        candidate,
    )?))
}

#[cfg(test)]
struct RollbackGuard<'a, E: LowRiskActionExecutor + ?Sized> {
    executor: &'a mut E,
    token: Option<RollbackToken>,
    rollback_performed: bool,
}

#[cfg(test)]
impl<'a, E: LowRiskActionExecutor + ?Sized> RollbackGuard<'a, E> {
    fn new(executor: &'a mut E, token: RollbackToken) -> Self {
        Self {
            executor,
            token: Some(token),
            rollback_performed: false,
        }
    }

    fn rollback_now(&mut self) -> anyhow::Result<()> {
        if let Some(token) = self.token.take() {
            self.executor.rollback(&token)?;
            self.rollback_performed = true;
        }
        Ok(())
    }

    fn rollback_performed(&self) -> bool {
        self.rollback_performed
    }
}

#[cfg(test)]
impl<E: LowRiskActionExecutor + ?Sized> Drop for RollbackGuard<'_, E> {
    fn drop(&mut self) {
        if let Some(token) = self.token.take() {
            if let Err(err) = self.executor.rollback(&token) {
                log::error!(
                    "autotune_apply_low_risk_rollback_failed candidate={} error={err:#}",
                    self.executor.candidate_name()
                );
            } else {
                self.rollback_performed = true;
            }
        }
    }
}

#[cfg(test)]
pub async fn run_apply_low_risk_candidate(
    candidate: CandidateAction,
    duration: Duration,
) -> anyhow::Result<ApplyLowRiskOutcome> {
    let mut executor = executor_for_low_risk_candidate(candidate)?;
    run_apply_low_risk_with_executor(executor.as_mut(), duration).await
}

#[cfg(test)]
pub async fn run_apply_low_risk_with_executor<E: LowRiskActionExecutor + ?Sized>(
    executor: &mut E,
    duration: Duration,
) -> anyhow::Result<ApplyLowRiskOutcome> {
    let candidate_name = executor.candidate_name().to_owned();
    let action_kind = executor.action_kind().to_owned();
    let safety_class = executor.safety_class();

    ensure_low_risk_action_allowed(&action_kind, &safety_class)?;

    let dry_run = executor.dry_run()?;
    if !dry_run.eligible {
        anyhow::bail!(
            "candidate '{}' is not eligible for apply-low-risk: {}",
            candidate_name,
            dry_run
                .reason
                .as_deref()
                .unwrap_or("dry-run did not produce an eligible candidate")
        );
    }

    let token = executor
        .apply()
        .with_context(|| format!("failed to apply low-risk candidate '{}'", candidate_name))?;
    let affected_tasks = token.affected_tasks();

    let mut guard = RollbackGuard::new(executor, token);

    if !duration.is_zero() {
        tokio::time::sleep(duration).await;
    }

    guard
        .rollback_now()
        .with_context(|| format!("failed to rollback low-risk candidate '{}'", candidate_name))?;

    Ok(ApplyLowRiskOutcome {
        candidate_name,
        action_kind,
        affected_tasks,
        safety_class,
        rollback_performed: guard.rollback_performed(),
    })
}
