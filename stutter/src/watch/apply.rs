use std::{path::PathBuf, time::Duration};

use log::{debug, info, warn};
use tokio::{
    signal,
    time::{Instant, MissedTickBehavior, interval},
};

use super::{
    PROFILE_WATCH_VERIFY_MS,
    policy::{force_for_watch_apply, validate_apply_profile_mode, validate_apply_profile_policy},
    restore::restore_profile_watch_on_exit,
};

pub struct ApplyProfileCommandInput {
    pub tree_pid: u32,
    pub profile_path: PathBuf,
    pub force: bool,
    pub dry_run: bool,
    pub allow_medium_risk: bool,
    pub watch: bool,
    pub keep_applied: bool,
    pub refresh_ms: u64,
    pub enforce: bool,
}

pub async fn apply_profile_command(input: ApplyProfileCommandInput) -> anyhow::Result<()> {
    let ApplyProfileCommandInput {
        tree_pid,
        profile_path,
        force,
        dry_run,
        allow_medium_risk,
        watch,
        keep_applied,
        refresh_ms,
        enforce,
    } = input;
    let profile = crate::profiles::load_first_profile(&profile_path)?;
    validate_apply_profile_mode(dry_run, watch)?;
    let persistent_effect = watch && keep_applied;
    let policy = validate_apply_profile_policy(
        &profile,
        tree_pid,
        force,
        dry_run,
        allow_medium_risk,
        persistent_effect,
        crate::daemon_policy::ActionSource::Cli,
    )?;
    let mut cache = crate::profiles::ProfileApplyCache::default();

    if !watch {
        if dry_run {
            let (apply_result, _) = apply_profile_to_tree_cached_blocking(
                tree_pid,
                profile,
                force,
                true,
                cache,
                policy.clone(),
                false,
            )
            .await?;

            print_profile_dry_run_result(&apply_result);
            println!(
                "apply-profile dry-run did not change live affinity, nice, ionice, audit state, or restore state"
            );
            println!("apply-profile is one-shot; use --watch to keep applying to new threads");
            return Ok(());
        }

        let action = crate::actions::cpu_affinity::CpuAffinityProfileAction {
            tree_pid,
            profile,
            force_restore_overwrite: force,
        };
        let result = tokio::task::spawn_blocking(move || {
            let run_policy = crate::actions::runner::ActionRunPolicy {
                policy,
                context: crate::daemon_policy::DaemonPolicyContext::default(),
                max_affected_tasks: None,
                max_total_duration: None,
                dry_run: false,
            };
            crate::actions::runner::run_audited_action("apply-profile", &action, run_policy)
        })
        .await
        .map_err(|err| anyhow::anyhow!("profile apply worker failed: {err}"))??;

        println!(
            "applied profile to {} task(s); restore with: stutter restore",
            result.state.affected_tasks
        );
        println!("apply-profile is one-shot; use --watch to keep applying to new threads");
        return Ok(());
    }

    let (apply_result, updated_cache) = match apply_profile_to_tree_cached_blocking(
        tree_pid,
        profile.clone(),
        force_for_watch_apply(true, force),
        dry_run,
        cache,
        policy.clone(),
        persistent_effect,
    )
    .await
    {
        Ok(res) => res,
        Err(err) => {
            if !keep_applied && let Err(restore_err) = restore_profile_watch_on_exit() {
                warn!("profile_watch_restore_after_error_failed err={restore_err:#}");
            }
            return Err(err);
        }
    };
    cache = updated_cache;

    println!(
        "applied profile to {} task(s); restore with: stutter restore",
        apply_result.affected_tasks()
    );
    crate::audit::audit_or_warn(&crate::audit::AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "apply-profile --watch".to_owned(),
        action_id: Some(format!("cpu-affinity-profile:{}", profile.name)),
        safety_class: Some(
            if crate::profiles::profile_uses_priority_actions(&profile) {
                crate::actions::SafetyClass::ReversibleMediumRisk
            } else {
                crate::actions::SafetyClass::ReversibleLowRisk
            },
        ),
        dry_run,
        success: true,
        affected_tasks: apply_result.affected_tasks(),
        restore_path: Some(crate::profile_restore::default_restore_path()),
        action_phase: None,
        error_category: None,
        message: format!(
            "initial profile application completed affinity={} nice={} ionice={}",
            apply_result.affinity_records.len(),
            apply_result.nice_records.len(),
            apply_result.ionice_records.len()
        ),
    });

    let mut tick = interval(Duration::from_millis(refresh_ms));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.tick().await;
    let verify_interval = Duration::from_millis(PROFILE_WATCH_VERIFY_MS);
    let mut next_verify = Instant::now() + verify_interval;

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                if keep_applied {
                    println!("stopped profile watch; restore with: stutter restore");
                } else {
                    restore_profile_watch_on_exit()?;
                }
                return Ok(());
            }
            _ = tick.tick() => {
                if enforce || Instant::now() >= next_verify {
                    cache.clear();
                    next_verify = Instant::now() + verify_interval;
                    debug!("profile_watch_cache_invalidated_for_full_verify enforce={enforce}");
                }

                let result = apply_profile_to_tree_cached_blocking(
                    tree_pid,
                    profile.clone(),
                    force_for_watch_apply(false, force),
                    dry_run,
                    cache,
                    policy.clone(),
                    persistent_effect,
                )
                .await;

                let apply_result = match result {
                    Ok((apply_result, updated_cache)) => {
                        cache = updated_cache;
                        apply_result
                    }
                    Err(err) => {
                        if !keep_applied
                            && let Err(restore_err) = restore_profile_watch_on_exit()
                        {
                            warn!("profile_watch_restore_after_error_failed err={restore_err:#}");
                        }
                        return Err(err);
                    }
                };

                if apply_result.affected_tasks() > 0 {
                    info!(
                        "profile_watch_applied tasks={} affinity={} nice={} ionice={}",
                        apply_result.affected_tasks(),
                        apply_result.affinity_records.len(),
                        apply_result.nice_records.len(),
                        apply_result.ionice_records.len()
                    );
                }
            }
        }
    }
}

pub async fn apply_profile_to_tree_blocking(
    tree_pid: u32,
    profile: crate::profiles::Profile,
    force: bool,
    dry_run: bool,
    _enforce: bool,
    policy: crate::daemon_policy::DaemonPolicy,
    persistent_effect: bool,
) -> anyhow::Result<Vec<crate::affinity::AffinityRecord>> {
    tokio::task::spawn_blocking(move || {
        let action = crate::actions::cpu_affinity::CpuAffinityProfileAction {
            tree_pid,
            profile,
            force_restore_overwrite: force,
        };
        let cache = crate::profiles::ProfileApplyCache::default();
        action
            .apply_cached_with_policy(&policy, dry_run, cache, persistent_effect)
            .map(|(result, _cache)| result.affinity_records)
    })
    .await
    .map_err(|err| anyhow::anyhow!("profile apply worker failed: {err}"))?
}

pub async fn apply_profile_to_tree_cached_blocking(
    tree_pid: u32,
    profile: crate::profiles::Profile,
    force: bool,
    dry_run: bool,
    cache: crate::profiles::ProfileApplyCache,
    policy: crate::daemon_policy::DaemonPolicy,
    persistent_effect: bool,
) -> anyhow::Result<(
    crate::profiles::ProfileApplyResult,
    crate::profiles::ProfileApplyCache,
)> {
    tokio::task::spawn_blocking(move || {
        let action = crate::actions::cpu_affinity::CpuAffinityProfileAction {
            tree_pid,
            profile,
            force_restore_overwrite: force,
        };
        action.apply_cached_with_policy(&policy, dry_run, cache, persistent_effect)
    })
    .await
    .map_err(|err| anyhow::anyhow!("profile apply worker failed: {err}"))?
}

fn print_profile_dry_run_result(result: &crate::profiles::ProfileApplyResult) {
    println!("profile dry-run:");
    println!("  checked_tasks={}", result.summary.checked_tasks);
    println!("  pending_affinity={}", result.summary.pending_affinity);
    println!("  pending_nice={}", result.summary.pending_nice);
    println!("  pending_ionice={}", result.summary.pending_ionice);
    println!("  total_pending_tasks={}", result.summary.pending_changes);

    if result.affinity_records.is_empty() {
        println!("  affinity_changes=[]");
    } else {
        println!("  affinity_changes:");
        for record in &result.affinity_records {
            println!(
                "    tid={} process_pid={:?} original_mask={} proposed_mask={}",
                record.tid,
                record.process_pid,
                record.original_mask.to_range_string(),
                record.applied_mask.to_range_string()
            );
        }
    }

    if result.nice_records.is_empty() {
        println!("  nice_changes=[]");
    } else {
        println!("  nice_changes:");
        for record in &result.nice_records {
            println!(
                "    tid={} process_pid={:?} comm={} original_nice={} proposed_nice={}",
                record.tid,
                record.process_pid,
                record.comm.as_deref().unwrap_or("<unknown>"),
                record.original_nice,
                record.applied_nice
            );
        }
    }

    if result.ionice_records.is_empty() {
        println!("  ionice_changes=[]");
    } else {
        println!("  ionice_changes:");
        for record in &result.ionice_records {
            println!(
                "    tid={} process_pid={:?} comm={} original_ioprio={} proposed_ioprio={}",
                record.tid,
                record.process_pid,
                record.comm.as_deref().unwrap_or("<unknown>"),
                record.original_ioprio,
                record.applied_ioprio
            );
        }
    }
}
