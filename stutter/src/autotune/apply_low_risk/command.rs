//! Test-only apply-low-risk command orchestration.
//!
//! Owns command input validation, profile loading, controller-journal setup, washout, and rollback
//! sequencing. It must not own lower-level audit execution, target selection, or experiment planning.

use std::time::Duration;

use anyhow::Context;

use super::{
    ApplyLowRiskOutcome, AuditedRollbackGuard, action_from_candidate,
    apply_cpu_affinity_candidate_with_audit_hooks, controller_journal_hooks_for_low_risk_action,
    controller_journal_metadata_for_cpu_affinity_action, plan_apply_low_risk_from_profiles,
    resolve_one_target_tree_pid,
};
use crate::autotune::{
    controller_journal::{
        write_controller_journal_applying_with_metadata, write_controller_journal_clean,
    },
    washout::{WashoutWindowConfig, run_washout_for_action},
};

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
        crate::autotune::experiment::ExperimentId::try_new(&experiment_id)?,
        crate::actions::ActionId::try_new(&action_id)?,
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
        action.tree_pid.into(),
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
