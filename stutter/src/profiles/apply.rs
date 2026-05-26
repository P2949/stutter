use anyhow::Context;

use super::{
    Profile,
    ioprio::profile_ioprio_policy,
    plan::{ProfileApplyCache, ProfileApplyPlan, planned_profile_apply},
    summary::ProfileApplyResult,
    verify::{preflight_profile_plan, verify_affinity_plan},
    warnings::warn_profile_offline_cpus,
};
use crate::{
    actions::{
        ioprio::IoPrioAction,
        nice::{NiceAction, NicePolicy},
        runner::{ActionRunPolicy, run_audited_action},
    },
    affinity::{self, AffinityRecord},
    process_tree, profile_restore,
};

pub fn profile_uses_priority_actions(profile: &Profile) -> bool {
    profile
        .rules
        .iter()
        .any(|rule| rule.nice.is_some() || rule.ionice.is_some())
}

pub fn apply_profile_to_tree(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
) -> anyhow::Result<Vec<AffinityRecord>> {
    apply_managed_profile_to_tree_with_cache(
        tree_pid,
        profile,
        force_restore_overwrite,
        dry_run,
        None,
    )
    .map(|result| result.affinity_records)
}

pub fn apply_managed_profile_to_tree(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
) -> anyhow::Result<ProfileApplyResult> {
    apply_managed_profile_to_tree_with_cache(
        tree_pid,
        profile,
        force_restore_overwrite,
        dry_run,
        None,
    )
}

pub fn apply_managed_profile_to_tree_cached(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
    cache: &mut ProfileApplyCache,
) -> anyhow::Result<ProfileApplyResult> {
    apply_managed_profile_to_tree_with_cache(
        tree_pid,
        profile,
        force_restore_overwrite,
        dry_run,
        Some(cache),
    )
}

fn apply_managed_profile_to_tree_with_cache(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
    mut cache: Option<&mut ProfileApplyCache>,
) -> anyhow::Result<ProfileApplyResult> {
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );

    if let Err(err) = warn_profile_offline_cpus(profile) {
        log::warn!("profile_online_cpu_check_failed err={err:#}");
    }

    let planned = planned_profile_apply(&snapshot.tasks, profile, cache.as_deref_mut())?;
    if planned.is_empty() {
        return Ok(ProfileApplyResult {
            summary: planned.summary,
            ..ProfileApplyResult::default()
        });
    }

    if dry_run {
        for planned in &planned.affinity_changes {
            log::info!(
                "dry_run: would apply mask {} to TID {}",
                planned.record.applied_mask.to_range_string(),
                planned.record.tid
            );
        }
        return Ok(result_from_plan(&planned));
    }

    preflight_profile_plan(&planned)?;

    let restore_path = profile_restore::default_restore_path();
    let affinity_records = planned.affinity_records();
    profile_restore::save_merged_restore_state(
        &restore_path,
        &affinity_records,
        &planned.nice_records,
        &planned.ionice_records,
        force_restore_overwrite,
    )?;

    let mut result = result_from_plan(&planned);
    result.affinity_records.clear();

    if let Err(err) = apply_profile_plan(&planned, &mut result) {
        if let Err(restore_err) = profile_restore::restore_saved(&restore_path) {
            anyhow::bail!(
                "profile apply failed: {err:#}; emergency restore failed: {restore_err:#}"
            );
        }
        anyhow::bail!("profile apply failed; restore completed: {err:#}");
    }

    if let Err(err) = verify_affinity_plan(&planned) {
        if let Err(restore_err) = profile_restore::restore_saved(&restore_path) {
            anyhow::bail!(
                "profile verify failed: {err:#}; emergency restore failed: {restore_err:#}"
            );
        }
        anyhow::bail!("profile verify failed; restore completed: {err:#}");
    }

    if let Some(cache) = cache {
        for cache_key in &planned.cache_keys {
            cache.known_correct.insert(cache_key.clone());
        }
    }

    result.affinity_records.sort_by_key(|record| record.tid);
    Ok(result)
}

fn apply_profile_plan(
    plan: &ProfileApplyPlan,
    result: &mut ProfileApplyResult,
) -> anyhow::Result<()> {
    for planned in &plan.affinity_changes {
        match affinity::set_affinity(planned.record.tid, &planned.record.applied_mask) {
            Ok(()) => result.affinity_records.push(planned.record.clone()),
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {}
            Err(err) => {
                anyhow::bail!(
                    "failed to set affinity for TID {}: {err}",
                    planned.record.tid
                );
            }
        }
    }

    for (nice, targets) in &plan.nice_groups {
        let action = NiceAction {
            targets: targets.clone(),
            nice: *nice,
            policy: NicePolicy::default(),
        };
        let run_policy = ActionRunPolicy::for_action(
            &action,
            false,
            crate::daemon_policy::ActionSource::ApplyProfileWatch,
        );
        run_audited_action("apply-profile nice", &action, run_policy)
            .with_context(|| format!("failed to apply profile nice={nice}"))?;
    }

    for (ioprio, targets) in &plan.ionice_groups {
        let action = IoPrioAction {
            targets: targets.clone(),
            ioprio: *ioprio,
            policy: profile_ioprio_policy(),
        };
        let run_policy = ActionRunPolicy::for_action(
            &action,
            false,
            crate::daemon_policy::ActionSource::ApplyProfileWatch,
        );
        run_audited_action("apply-profile ionice", &action, run_policy)
            .with_context(|| format!("failed to apply profile I/O priority={}", ioprio.label()))?;
    }

    Ok(())
}

fn result_from_plan(plan: &ProfileApplyPlan) -> ProfileApplyResult {
    let mut affinity_records = plan.affinity_records();
    affinity_records.sort_by_key(|record| record.tid);

    ProfileApplyResult {
        affinity_records,
        nice_records: plan.nice_records.clone(),
        ionice_records: plan.ionice_records.clone(),
        summary: plan.summary.clone(),
    }
}
