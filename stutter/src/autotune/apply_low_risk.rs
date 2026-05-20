#[cfg(test)]
use std::{path::Path, time::Duration};

#[cfg(test)]
use anyhow::Context;

use crate::{
    actions::cpu_affinity::CpuAffinityProfileAction, autotune::candidate::CandidateAction,
};
#[cfg(test)]
use crate::{
    autotune::{
        candidate::{CandidateDryRunRecord, dry_run_candidates, generate_profile_candidates},
        controller_journal::{
            write_controller_journal_applying_with_metadata, write_controller_journal_clean,
        },
        washout::{WashoutWindowConfig, run_washout_for_action},
    },
    profiles::Profile,
};

mod audit;
#[cfg(test)]
mod executor;
mod model;
#[cfg(test)]
mod target;

pub use audit::action_from_candidate;
#[cfg(test)]
pub(crate) use audit::{
    AuditedRollbackGuard, append_low_risk_history_event,
    apply_cpu_affinity_candidate_with_audit_hooks,
    apply_cpu_affinity_candidate_with_audit_path_for_tests,
    controller_journal_hooks_for_low_risk_action,
    controller_journal_metadata_for_cpu_affinity_action,
};

pub fn ensure_low_risk_action_allowed(
    action_kind: &str,
    safety_class: &crate::actions::SafetyClass,
) -> anyhow::Result<()> {
    audit::ensure_low_risk_action_allowed(action_kind, safety_class)
}
#[cfg(test)]
pub use executor::{
    CpuAffinityLowRiskExecutor, LowRiskActionExecutor, executor_for_low_risk_candidate,
    run_apply_low_risk_candidate, run_apply_low_risk_with_executor,
};
pub type AuditedCandidateApplyOutcome = model::AuditedCandidateApplyOutcome;
#[cfg(test)]
pub use model::{ApplyLowRiskOutcome, ApplyLowRiskPlan};
#[cfg(test)]
pub use target::{resolve_one_target_tree_pid, resolve_one_target_tree_pid_at};

pub fn apply_cpu_affinity_candidate_with_audit(
    candidate_name: String,
    action: &CpuAffinityProfileAction,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    audit::apply_cpu_affinity_candidate_with_audit(candidate_name, action)
}

pub fn apply_candidate_with_audit(
    candidate: CandidateAction,
) -> anyhow::Result<AuditedCandidateApplyOutcome> {
    let (candidate_name, action) = action_from_candidate(candidate)?;
    apply_cpu_affinity_candidate_with_audit(candidate_name, &action)
}

fn unsupported_low_risk_candidate<T>(candidate: &CandidateAction) -> anyhow::Result<T> {
    anyhow::bail!(
        "apply-low-risk supports CPU-affinity profile actions only; candidate '{}' action_kind={} safety={:?} required_mode={}",
        candidate.candidate_name(),
        candidate.action_kind(),
        candidate.safety_class(),
        crate::daemon_policy::DaemonMode::ApplyMediumRisk
    )
}

#[cfg(test)]
pub fn select_first_eligible_low_risk_candidate(
    candidates: &[CandidateAction],
    records: &[CandidateDryRunRecord],
) -> anyhow::Result<CandidateAction> {
    for record in records {
        ensure_low_risk_action_allowed("cpu_affinity_profile", &record.safety_class)?;
        if !record.eligible {
            continue;
        }

        if let Some(candidate) = candidates
            .iter()
            .find(|candidate| candidate.profile_name() == record.candidate_name)
        {
            return Ok(candidate.clone());
        }
    }

    anyhow::bail!("no eligible ReversibleLowRisk CPU affinity profile candidate found")
}

#[cfg(test)]
pub fn plan_apply_low_risk_from_profiles(
    tree_pid: u32,
    profiles_path: &Path,
    profiles: &[Profile],
    duration: Duration,
) -> anyhow::Result<ApplyLowRiskPlan> {
    let candidates = generate_profile_candidates(profiles, tree_pid, None);
    if candidates.is_empty() {
        anyhow::bail!("no CPU affinity profile candidates were generated");
    }

    let records = dry_run_candidates(&candidates);
    let candidate = select_first_eligible_low_risk_candidate(&candidates, &records)?;
    let dry_run_record = records
        .into_iter()
        .find(|record| record.candidate_name == candidate.profile_name())
        .context("selected candidate did not have a matching dry-run record")?;

    Ok(ApplyLowRiskPlan {
        tree_pid,
        profiles_path: profiles_path.to_path_buf(),
        candidate,
        dry_run_record,
        duration,
    })
}

#[cfg(test)]
pub fn resolve_low_risk_experiment_with_active_profile_state<
    E: crate::autotune::resolution::ExperimentRollbackExecutor + ?Sized,
>(
    experiment: &mut crate::autotune::experiment::ActiveExperiment,
    result: &crate::autotune::comparison::ExperimentResult,
    rollback_executor: &mut E,
    active_profile_state: &mut crate::autotune::kept::ActiveProfileState,
    now_unix_nanos: u128,
) -> anyhow::Result<crate::autotune::resolution::ExperimentResolution> {
    crate::autotune::resolution::resolve_experiment_with_active_profile_state(
        experiment,
        result,
        rollback_executor,
        active_profile_state,
        now_unix_nanos,
    )
}

#[cfg(test)]
pub fn resolve_low_risk_experiment<
    E: crate::autotune::resolution::ExperimentRollbackExecutor + ?Sized,
>(
    experiment: &mut crate::autotune::experiment::ActiveExperiment,
    result: &crate::autotune::comparison::ExperimentResult,
    rollback_executor: &mut E,
) -> anyhow::Result<crate::autotune::resolution::ExperimentResolution> {
    crate::autotune::resolution::resolve_experiment(experiment, result, rollback_executor)
}

#[cfg(test)]
pub fn compare_low_risk_experiment(
    baseline: &crate::autotune::experiment::WindowScore,
    candidate: &crate::autotune::experiment::WindowScore,
    data_quality: crate::autotune::comparison::ExperimentDataQuality,
    target_disappeared: bool,
) -> crate::autotune::comparison::ExperimentResult {
    crate::autotune::comparison::compare_experiment(
        crate::autotune::comparison::ExperimentComparisonInput {
            baseline,
            candidate,
            data_quality,
            target_disappeared,
        },
    )
}

#[cfg(test)]
pub fn ensure_candidate_measurement_ready_for_decision(
    measurement_status: &crate::autotune::measurement::CandidateMeasurementWindowStatus,
) -> anyhow::Result<crate::autotune::experiment::WindowScore> {
    crate::autotune::measurement::ensure_candidate_measurement_ready_for_decision(
        measurement_status,
    )
}

#[cfg(test)]
pub fn ensure_baseline_ready_for_apply(
    baseline_status: &crate::autotune::baseline::BaselineWindowStatus,
) -> anyhow::Result<crate::autotune::experiment::WindowScore> {
    match baseline_status {
        crate::autotune::baseline::BaselineWindowStatus::Ready { score } => Ok(score.clone()),
        crate::autotune::baseline::BaselineWindowStatus::Collecting { reasons, .. } => {
            anyhow::bail!(
                "baseline window is not ready; action blocked: {}",
                reasons.join("; ")
            )
        }
    }
}

#[cfg(test)]
pub async fn apply_low_risk_command(
    input: &crate::autotune::commands::live::AutotuneCommandInput,
) -> anyhow::Result<ApplyLowRiskOutcome> {
    let tree_pid = resolve_one_target_tree_pid(input.tree_pid, input.watch_process.as_deref())?;

    let profiles_path = input
        .profiles
        .as_deref()
        .context("apply-low-risk requires --profiles")?;

    let profiles = crate::profiles::load_profiles(profiles_path)?;
    if profiles.is_empty() {
        anyhow::bail!(
            "profile file {} did not contain [[profile]]",
            profiles_path.display()
        );
    }

    let duration = Duration::from_secs(input.duration_seconds.unwrap_or(30));
    let plan = plan_apply_low_risk_from_profiles(tree_pid, profiles_path, &profiles, duration)?;

    let (candidate_name, action) = action_from_candidate(plan.candidate)?;
    let experiment_id = format!("apply-low-risk:{}", candidate_name);
    let action_id = format!("cpu-affinity-profile:{}", candidate_name);
    let journal_path = crate::autotune::controller_journal::default_controller_journal_path();

    write_controller_journal_applying_with_metadata(
        &journal_path,
        &experiment_id,
        &action_id,
        controller_journal_metadata_for_cpu_affinity_action(
            &candidate_name,
            &action,
            None,
            "pending_apply",
        ),
    )
    .with_context(|| {
        format!(
            "failed to write applying controller journal for autotune candidate '{}'",
            candidate_name
        )
    })?;

    let audited = apply_cpu_affinity_candidate_with_audit_hooks(
        candidate_name.clone(),
        &action,
        controller_journal_hooks_for_low_risk_action(
            &journal_path,
            &experiment_id,
            &action_id,
            &candidate_name,
            &action,
        ),
    )?;
    let _affected_tasks = audited.affected_tasks;
    let mut guard = AuditedRollbackGuard::new(&action, audited.rollback.clone());

    run_washout_for_action(
        &action,
        action.tree_pid,
        WashoutWindowConfig::default()
            .with_washout(input.washout_seconds, input.washout_verify_interval_ms),
    )
    .await
    .with_context(|| format!("washout failed for autotune candidate '{}'", candidate_name))?;

    if !plan.duration.is_zero() {
        tokio::time::sleep(plan.duration).await;
    }

    guard.rollback_now().with_context(|| {
        format!(
            "rollback failed for autotune candidate '{}'",
            audited.candidate_name
        )
    })?;

    write_controller_journal_clean(&journal_path).with_context(|| {
        format!(
            "failed to write clean controller journal after rolling back autotune candidate '{}'",
            audited.candidate_name
        )
    })?;

    Ok(ApplyLowRiskOutcome {
        candidate_name: audited.candidate_name,
        action_kind: audited.action_kind,
        affected_tasks: audited.affected_tasks,
        safety_class: audited.safety_class,
        rollback_performed: guard.rollback_performed(),
    })
}

#[cfg(test)]
#[path = "apply_low_risk_tests/mod.rs"]
mod tests;
