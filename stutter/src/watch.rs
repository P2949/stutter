use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use futures_util::future;
use log::{debug, info, warn};
use tokio::{
    signal,
    time::{Instant, MissedTickBehavior, interval, sleep},
};

use crate::cli::Config;

pub const PROFILE_WATCH_VERIFY_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatchProcessState {
    None,
    Waiting,
    Running(u32),
}

impl WatchProcessState {
    pub fn running_pid(&self) -> Option<u32> {
        match self {
            WatchProcessState::Running(pid) => Some(*pid),
            _ => None,
        }
    }

    pub fn should_poll(&self) -> bool {
        matches!(self, WatchProcessState::Waiting | WatchProcessState::None)
    }
}

pub async fn resolve_watch_process(
    config: &Config,
    tree_pids: &mut Vec<u32>,
) -> anyhow::Result<Option<u32>> {
    let Some(pattern) = config.watch_process.clone() else {
        return Ok(None);
    };

    let mut cache = crate::process_tree::ProcessCache::default();
    if let Some(pid) =
        find_process_by_pattern_at_with_cache(Path::new("/proc"), &pattern, &mut cache)
    {
        add_watch_tree_pid(tree_pids, pid);
        return Ok(Some(pid));
    }

    wait_for_watch_process(config, tree_pids)
        .await?
        .ok_or_else(|| anyhow::anyhow!("stopped while waiting for --watch-process {pattern}"))
        .map(Some)
}

pub async fn wait_for_watch_process(
    config: &Config,
    tree_pids: &mut Vec<u32>,
) -> anyhow::Result<Option<u32>> {
    let pattern = config
        .watch_process
        .clone()
        .ok_or_else(|| anyhow::anyhow!("internal error: watch_process missing"))?;

    info!(
        "watch_process_waiting pattern={} persistent={}",
        pattern, config.persistent
    );

    let mut tick = interval(Duration::from_millis(config.watch_poll_ms));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut cache = crate::process_tree::ProcessCache::default();

    let watch_timeout = config.watch_timeout;
    let timeout_future = async move {
        if let Some(timeout) = watch_timeout {
            sleep(timeout).await;
        } else {
            future::pending::<()>().await;
        }
    };
    tokio::pin!(timeout_future);

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                return Ok(None);
            }
            _ = &mut timeout_future => {
                anyhow::bail!(
                    "timed out waiting for --watch-process {pattern} after {}ms",
                    watch_timeout.map(|timeout| timeout.as_millis()).unwrap_or(0)
                );
            }
            _ = tick.tick() => {
                if let Some(pid) = find_process_by_pattern_at_with_cache(
                    Path::new("/proc"),
                    &pattern,
                    &mut cache,
                ) {
                    add_watch_tree_pid(tree_pids, pid);
                    info!("watch_process_found pattern={} pid={}", pattern, pid);
                    return Ok(Some(pid));
                }
            }
        }
    }
}

pub fn add_watch_tree_pid(tree_pids: &mut Vec<u32>, pid: u32) {
    tree_pids.push(pid);
    tree_pids.sort_unstable();
    tree_pids.dedup();
}

pub fn remove_watch_tree_pid(tree_pids: &mut Vec<u32>, pid: u32) {
    tree_pids.retain(|tree_pid| *tree_pid != pid);
}

pub fn capture_tree_root_starttimes(tree_pids: &[u32]) -> BTreeMap<u32, Option<u64>> {
    tree_pids
        .iter()
        .map(|pid| (*pid, process_root_starttime(*pid)))
        .collect()
}

pub fn process_root_starttime(pid: u32) -> Option<u64> {
    crate::process_tree::process_starttime_at(Path::new("/proc"), pid)
}

pub fn tree_root_is_stale(pid: u32, root_starttimes: &BTreeMap<u32, Option<u64>>) -> bool {
    let current = process_root_starttime(pid);
    let expected = root_starttimes.get(&pid).copied().flatten();

    current.is_none() || expected.is_some_and(|expected| current != Some(expected))
}

pub fn remove_stale_tree_roots(
    tree_pids: &mut Vec<u32>,
    root_starttimes: &mut BTreeMap<u32, Option<u64>>,
    watched_pid: Option<u32>,
) -> Vec<u32> {
    let mut removed = Vec::new();

    for pid in tree_pids.clone() {
        if Some(pid) == watched_pid {
            continue;
        }

        if tree_root_is_stale(pid, root_starttimes) {
            removed.push(pid);
            root_starttimes.remove(&pid);
        }
    }

    if !removed.is_empty() {
        tree_pids.retain(|tree_pid| !removed.contains(tree_pid));
    }

    removed
}

#[cfg(test)]
pub fn find_process_by_pattern_at(proc_root: &Path, pattern: &str) -> Option<u32> {
    let mut cache = crate::process_tree::ProcessCache::default();
    find_process_by_pattern_at_with_cache(proc_root, pattern, &mut cache)
}

pub fn find_process_by_pattern_at_with_cache(
    proc_root: &Path,
    pattern: &str,
    cache: &mut crate::process_tree::ProcessCache,
) -> Option<u32> {
    let pattern_lower = normalize_process_match_text(pattern);

    let budget = crate::process_tree::ScanBudget::default_proc_scan();
    let mut budget_report = crate::process_tree::ScanBudgetReport::default();

    crate::process_tree::scan_processes_at(proc_root, cache, &budget, &mut budget_report)
        .into_iter()
        .filter_map(|(pid, process)| {
            let score =
                process_match_score(pattern, &pattern_lower, &process.comm, &process.cmdline)?;
            Some((score, pid))
        })
        .max_by_key(|(score, pid)| (*score, *pid))
        .map(|(_, pid)| pid)
}

pub fn process_match_score(
    pattern: &str,
    pattern_lower: &str,
    comm: &str,
    cmdline: &str,
) -> Option<u8> {
    if comm == pattern {
        return Some(5);
    }

    let comm_lower = normalize_process_match_text(comm);
    if comm_lower == pattern_lower {
        return Some(4);
    }

    let cmdline_lower = normalize_process_match_text(cmdline);
    let exe_basename_lower = cmdline_executable_basename_lower(cmdline);
    if exe_basename_lower.as_deref() == Some(pattern_lower) {
        return Some(3);
    }

    if comm_lower.contains(pattern_lower) {
        return Some(2);
    }

    if cmdline_lower.contains(pattern_lower) {
        return Some(1);
    }

    None
}

pub fn normalize_process_match_text(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}

pub fn cmdline_executable_basename_lower(cmdline: &str) -> Option<String> {
    let executable = cmdline.split_whitespace().next()?;
    let executable = normalize_process_match_text(executable);

    PathBuf::from(executable)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

pub fn force_for_watch_apply(initial: bool, user_force: bool) -> bool {
    initial && user_force
}

pub struct ApplyProfileCommandInput {
    pub tree_pid: u32,
    pub profile_path: PathBuf,
    pub force: bool,
    pub dry_run: bool,
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
        watch,
        keep_applied,
        refresh_ms,
        enforce,
    } = input;
    let profile = crate::profiles::load_first_profile(&profile_path)?;
    let mut cache = crate::profiles::ProfileApplyCache::default();

    if !watch {
        let action = crate::actions::cpu_affinity::CpuAffinityProfileAction {
            tree_pid,
            profile,
            force_restore_overwrite: force,
        };
        let result = tokio::task::spawn_blocking(move || {
            crate::actions::runner::run_audited_action("apply-profile", &action, dry_run)
        })
        .await
        .map_err(|err| anyhow::anyhow!("profile apply worker failed: {err}"))??;

        println!(
            "applied profile affinity to {} task(s); restore with: stutter restore",
            result.state.affected_tasks
        );
        println!("apply-profile is one-shot; use --watch to keep applying to new threads");
        return Ok(());
    }

    let (records, updated_cache) = match apply_profile_to_tree_cached_blocking(
        tree_pid,
        profile.clone(),
        force_for_watch_apply(true, force),
        dry_run,
        cache,
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
        "applied profile affinity to {} task(s); restore with: stutter restore",
        records.len()
    );
    crate::audit::audit_or_warn(&crate::audit::AuditEvent {
        schema_version: 1,
        unix_nanos: crate::audit::unix_nanos_now(),
        command: "apply-profile --watch".to_owned(),
        action_id: Some(format!("cpu-affinity-profile:{}", profile.name)),
        safety_class: Some(crate::actions::SafetyClass::ReversibleLowRisk),
        dry_run,
        success: true,
        affected_tasks: records.len(),
        restore_path: Some(crate::affinity::default_restore_path()),
        message: "initial CPU affinity profile application completed".to_owned(),
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
                )
                .await;

                let records = match result {
                    Ok((records, updated_cache)) => {
                        cache = updated_cache;
                        records
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

                if !records.is_empty() {
                    info!("profile_watch_applied tasks={}", records.len());
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
) -> anyhow::Result<Vec<crate::affinity::AffinityRecord>> {
    tokio::task::spawn_blocking(move || {
        // Enforce is handled by the caller clearing the cache in watch mode.
        // Blocking one-shot always verifies.
        let action = crate::actions::cpu_affinity::CpuAffinityProfileAction {
            tree_pid,
            profile,
            force_restore_overwrite: force,
        };
        action.apply_records(dry_run)
    })
    .await
    .map_err(|err| anyhow::anyhow!("profile apply worker failed: {err}"))?
}

pub async fn apply_profile_to_tree_cached_blocking(
    tree_pid: u32,
    profile: crate::profiles::Profile,
    force: bool,
    dry_run: bool,
    mut cache: crate::profiles::ProfileApplyCache,
) -> anyhow::Result<(
    Vec<crate::affinity::AffinityRecord>,
    crate::profiles::ProfileApplyCache,
)> {
    tokio::task::spawn_blocking(move || {
        crate::profiles::apply_profile_to_tree_cached(
            tree_pid, &profile, force, dry_run, &mut cache,
        )
        .map(|records| (records, cache))
    })
    .await
    .map_err(|err| anyhow::anyhow!("profile apply worker failed: {err}"))?
}

pub fn restore_profile_watch_on_exit() -> anyhow::Result<()> {
    let path = crate::affinity::default_restore_path();
    if !path.exists() {
        println!("stopped profile watch; no restore file was written");
        return Ok(());
    }

    match crate::affinity::restore_saved(&path) {
        Ok(summary) => {
            crate::audit::audit_or_warn(&crate::audit::AuditEvent {
                schema_version: 1,
                unix_nanos: crate::audit::unix_nanos_now(),
                command: "apply-profile --watch restore".to_owned(),
                action_id: Some("cpu-affinity-restore".to_owned()),
                safety_class: Some(crate::actions::SafetyClass::ReversibleLowRisk),
                dry_run: false,
                success: true,
                affected_tasks: summary.restored,
                restore_path: Some(path.clone()),
                message: format!(
                    "watch restore completed restored={} skipped_dead={} skipped_identity_mismatch={} legacy_unverified={}",
                    summary.restored,
                    summary.skipped_dead,
                    summary.skipped_identity_mismatch,
                    summary.legacy_unverified
                ),
            });
            println!(
                "stopped profile watch; restored {} affinity record(s); skipped_dead={} skipped_identity_mismatch={} legacy_unverified={}",
                summary.restored,
                summary.skipped_dead,
                summary.skipped_identity_mismatch,
                summary.legacy_unverified
            );
        }
        Err(err) => {
            crate::audit::audit_or_warn(&crate::audit::AuditEvent {
                schema_version: 1,
                unix_nanos: crate::audit::unix_nanos_now(),
                command: "apply-profile --watch restore".to_owned(),
                action_id: Some("cpu-affinity-restore".to_owned()),
                safety_class: Some(crate::actions::SafetyClass::ReversibleLowRisk),
                dry_run: false,
                success: false,
                affected_tasks: 0,
                restore_path: Some(path.clone()),
                message: format!("restore failed: {err:#}"),
            });
            return Err(err);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_for_watch_apply_only_uses_user_force_on_initial_apply() {
        assert!(force_for_watch_apply(true, true));
        assert!(!force_for_watch_apply(false, true));
        assert!(!force_for_watch_apply(true, false));
        assert!(!force_for_watch_apply(false, false));
    }

    #[test]
    fn test_watch_process_should_poll() {
        assert!(WatchProcessState::None.should_poll());
        assert!(WatchProcessState::Waiting.should_poll());
        assert!(!WatchProcessState::Running(123).should_poll());
    }

    #[test]
    fn test_process_match_score() {
        let p = "my-game";
        let pl = "my-game";

        // Score 5: Exact comm match
        assert_eq!(process_match_score(p, pl, "my-game", ""), Some(5));

        // Score 4: Case-insensitive comm match
        assert_eq!(process_match_score(p, pl, "MY-GAME", ""), Some(4));

        // Score 3: Executable basename match
        assert_eq!(
            process_match_score(p, pl, "other", "/usr/bin/my-game"),
            Some(3)
        );
        assert_eq!(
            process_match_score(p, pl, "other", "C:\\Games\\my-game"),
            Some(3)
        );

        // Score 2: comm substring match
        assert_eq!(process_match_score(p, pl, "super-my-game-pro", ""), Some(2));

        // Score 1: cmdline substring match
        assert_eq!(
            process_match_score(p, pl, "other", "--game=my-game"),
            Some(1)
        );

        // None: No match
        assert_eq!(process_match_score(p, pl, "other", "--foo"), None);
    }

    #[test]
    fn test_find_process_selection_priority() {
        let dir = std::env::temp_dir().join(format!("stutter-watch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // PID 100: comm contains pattern (Score 2)
        let pid100 = dir.join("100");
        std::fs::create_dir_all(&pid100).unwrap();
        std::fs::write(pid100.join("status"), "Name:\tmy-game-helper\nPPid:\t1\n").unwrap();
        std::fs::write(pid100.join("cmdline"), "helper\0").unwrap();
        std::fs::write(
            pid100.join("stat"),
            "100 (my-game-helper) S 1 100 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        )
        .unwrap();

        // PID 200: only cmdline contains pattern (Score 1)
        let pid200 = dir.join("200");
        std::fs::create_dir_all(&pid200).unwrap();
        std::fs::write(pid200.join("status"), "Name:\tother\nPPid:\t1\n").unwrap();
        std::fs::write(pid200.join("cmdline"), "other\0--match=my-game\0").unwrap();
        std::fs::write(
            pid200.join("stat"),
            "200 (other) S 1 200 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        )
        .unwrap();

        // Even though 200 has a higher PID, 100 should be selected because Score 2 > Score 1.
        let mut cache = crate::process_tree::ProcessCache::default();
        let selected = find_process_by_pattern_at_with_cache(&dir, "my-game", &mut cache);
        assert_eq!(selected, Some(100));

        std::fs::remove_dir_all(dir).ok();
    }
}
