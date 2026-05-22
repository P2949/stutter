//! Experiment planning and readiness helpers for apply-low-risk.
//!
//! Owns candidate selection from profile dry-runs, comparison/resolution wrappers, and readiness
//! checks. It must not resolve procfs targets, run audited actions, or dispatch CLI commands.

use std::{path::Path, time::Duration};

use anyhow::Context;

use super::{ApplyLowRiskPlan, ensure_low_risk_action_allowed};
use crate::{
    autotune::planning::{
        candidate::CandidateAction,
        dry_run::{CandidateDryRunRecord, dry_run_candidates},
        profile_candidates::generate_profile_candidates,
    },
    profiles::Profile,
};

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

pub fn resolve_low_risk_experiment<
    E: crate::autotune::resolution::ExperimentRollbackExecutor + ?Sized,
>(
    experiment: &mut crate::autotune::experiment::ActiveExperiment,
    result: &crate::autotune::comparison::ExperimentResult,
    rollback_executor: &mut E,
) -> anyhow::Result<crate::autotune::resolution::ExperimentResolution> {
    crate::autotune::resolution::resolve_experiment(experiment, result, rollback_executor)
}

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

pub fn ensure_candidate_measurement_ready_for_decision(
    measurement_status: &crate::autotune::measurement::CandidateMeasurementWindowStatus,
) -> anyhow::Result<crate::autotune::experiment::WindowScore> {
    crate::autotune::measurement::ensure_candidate_measurement_ready_for_decision(
        measurement_status,
    )
}

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
