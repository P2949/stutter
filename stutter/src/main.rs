mod affinity;
mod cli;
mod ebpf_loader;
mod metadata;
mod metrics;
mod process_tree;
mod profiles;
mod recorder;
mod report;
mod scx;

#[cfg(test)]
mod regression_tests;

use std::{
    collections::BTreeMap,
    future,
    path::{Path, PathBuf},
    time::Instant,
};

use aya::maps::{HashMap as AyaHashMap, MapData};
use cli::{AppCommand, Config, parse_app_command};
use log::{debug, info, warn};
use metrics::{format_latency, print_event, print_interval_summaries, print_session_summaries};
use process_tree::{TargetDiffAction, TaskInfo};
use recorder::{
    FinalizeRecordingInput, IntervalRecord, SpikeEventBuffer, TreeEvent, finalize_recording,
    prepare_recording,
};
use stutter_common::{EVENT_RUNNABLE_LATENCY, SchedulerEvent};
use tokio::{
    signal,
    time::{Duration, MissedTickBehavior, interval, sleep},
};

pub const TARGET_PIDS_MAX: usize = 1024;

struct RefreshTargetTasksInput<'a> {
    config: &'a Config,
    active_targets: &'a mut BTreeMap<u32, TaskInfo>,
    known_targets: &'a mut BTreeMap<u32, TaskInfo>,
    stats_by_task: &'a mut BTreeMap<u32, metrics::TaskStats>,
    tree_events: &'a mut Vec<TreeEvent>,
    target_pid_map: &'a mut AyaHashMap<MapData, u32, u8>,
    elapsed_ms: u128,
    recording_started: Option<Instant>,
}

struct HandleEventInput<'a> {
    event: SchedulerEvent,
    config: &'a Config,
    started: Instant,
    active_targets: &'a BTreeMap<u32, TaskInfo>,
    known_targets: &'a BTreeMap<u32, TaskInfo>,
    stats_by_task: &'a mut BTreeMap<u32, metrics::TaskStats>,
    monotonic_start_ns: Option<u64>,
    spike_events: Option<&'a mut SpikeEventBuffer>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    match parse_app_command()? {
        AppCommand::Monitor(config) => run_monitor(config).await,
        AppCommand::Restore => {
            let path = affinity::default_restore_path();
            let summary = affinity::restore_saved(&path)?;
            println!(
                "restored {} affinity record(s); skipped_dead={}",
                summary.restored, summary.skipped_dead
            );
            Ok(())
        }
        AppCommand::ApplyProfile {
            tree_pid,
            profile,
            force,
            watch,
            keep_applied,
            refresh_ms,
        } => apply_profile_command(tree_pid, profile, force, watch, keep_applied, refresh_ms).await,
        AppCommand::InspectTree { tree_pid } => {
            let rendered = process_tree::render_tree(tree_pid)?;
            print!("{rendered}");
            Ok(())
        }
        AppCommand::Report {
            path,
            json,
            top,
            cluster_window_ms,
        } => report::print_report(&path, json, top, cluster_window_ms),
    }
}

async fn apply_profile_command(
    tree_pid: u32,
    profile_path: PathBuf,
    force: bool,
    watch: bool,
    keep_applied: bool,
    refresh_ms: u64,
) -> anyhow::Result<()> {
    let profile = profiles::load_first_profile(&profile_path)?;
    let mut cache = profiles::ProfileApplyCache::default();
    let initial_apply = if watch {
        profiles::apply_profile_to_tree_cached(tree_pid, &profile, force, &mut cache)
    } else {
        profiles::apply_profile_to_tree(tree_pid, &profile, force)
    };
    let records = match initial_apply {
        Ok(records) => records,
        Err(err) => {
            if watch
                && !keep_applied
                && let Err(restore_err) = restore_profile_watch_on_exit()
            {
                warn!("profile_watch_restore_after_error_failed err={restore_err:#}");
            }
            return Err(err);
        }
    };
    println!(
        "applied profile affinity to {} task(s); restore with: stutter restore",
        records.len()
    );

    if !watch {
        println!("apply-profile is one-shot; use --watch to keep applying to new threads");
        return Ok(());
    }

    let mut tick = interval(Duration::from_millis(refresh_ms));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.tick().await;

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
                let records = match profiles::apply_profile_to_tree_cached(
                    tree_pid,
                    &profile,
                    false,
                    &mut cache,
                ) {
                    Ok(records) => records,
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

fn restore_profile_watch_on_exit() -> anyhow::Result<()> {
    let path = affinity::default_restore_path();
    if !path.exists() {
        println!("stopped profile watch; no restore file was written");
        return Ok(());
    }

    let summary = affinity::restore_saved(&path)?;
    println!(
        "stopped profile watch; restored {} affinity record(s); skipped_dead={}",
        summary.restored, summary.skipped_dead
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WatchProcessState {
    None,
    Running(u32),
    Waiting,
}

impl WatchProcessState {
    fn running_pid(self) -> Option<u32> {
        match self {
            Self::Running(pid) => Some(pid),
            Self::None | Self::Waiting => None,
        }
    }
}

async fn run_monitor(mut config: Config) -> anyhow::Result<()> {
    let mut watch_state = match resolve_watch_process(&mut config).await? {
        Some(pid) => WatchProcessState::Running(pid),
        None => WatchProcessState::None,
    };
    let had_tree_roots = !config.tree_pids.is_empty();
    let mut tree_root_starttimes = capture_tree_root_starttimes(&config.tree_pids);

    let recording = prepare_recording(&config)?;
    let mut loaded = ebpf_loader::load_and_attach()?;

    let mut active_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let mut known_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let mut stats_by_task: BTreeMap<u32, metrics::TaskStats> = BTreeMap::new();
    let mut interval_records: Vec<IntervalRecord> = Vec::new();
    let mut tree_events: Vec<TreeEvent> = Vec::new();
    let mut spike_events = recording.as_ref().map(|_| SpikeEventBuffer::default());
    let mut scx_tracker = scx::ScxTracker::default();
    let recording_monotonic_start_ns = recording.as_ref().and_then(|run| run.monotonic_start_ns);

    let started = Instant::now();
    scx_tracker.sample(0);

    refresh_target_tasks(RefreshTargetTasksInput {
        config: &config,
        active_targets: &mut active_targets,
        known_targets: &mut known_targets,
        stats_by_task: &mut stats_by_task,
        tree_events: &mut tree_events,
        target_pid_map: &mut loaded.target_pid_map,
        elapsed_ms: started.elapsed().as_millis(),
        recording_started: recording.as_ref().map(|run| run.started_instant),
    })?;

    let mut summary_tick = interval(Duration::from_millis(config.summary_period_ms));
    summary_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    summary_tick.tick().await;

    let mut tree_tick = interval(Duration::from_secs(1));
    tree_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tree_tick.tick().await;

    let mut watch_tick = interval(Duration::from_secs(2));
    watch_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    watch_tick.tick().await;

    let mut scx_tick = interval(Duration::from_secs(1));
    scx_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    scx_tick.tick().await;

    let max_duration = config.max_duration;
    let duration_future = async move {
        if let Some(max_duration) = max_duration {
            sleep(max_duration).await;
        } else {
            future::pending::<()>().await;
        }
    };
    tokio::pin!(duration_future);

    info!(
        "attached target_tasks={} tree_roots={} summary_ms={} spike_threshold={}",
        active_targets.len(),
        config.tree_pids.len(),
        config.summary_period_ms,
        format_latency(config.spike_threshold_ns),
    );

    let stop_reason = loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                break "ctrl_c".to_owned();
            }

            _ = &mut duration_future => {
                break "duration".to_owned();
            }

            _ = summary_tick.tick() => {
                let elapsed_ms = started.elapsed().as_millis();
                print_interval_summaries(
                    &mut stats_by_task,
                    elapsed_ms,
                    &mut interval_records,
                );
            }

            _ = tree_tick.tick(), if !config.tree_pids.is_empty() => {
                if let Some(root_pid) = watch_state.running_pid()
                    && tree_root_is_stale(root_pid, &tree_root_starttimes)
                {
                    remove_watch_tree_pid(&mut config, root_pid);
                    tree_root_starttimes.remove(&root_pid);
                    refresh_target_tasks(RefreshTargetTasksInput {
                        config: &config,
                        active_targets: &mut active_targets,
                        known_targets: &mut known_targets,
                        stats_by_task: &mut stats_by_task,
                        tree_events: &mut tree_events,
                        target_pid_map: &mut loaded.target_pid_map,
                        elapsed_ms: started.elapsed().as_millis(),
                        recording_started: recording.as_ref().map(|run| run.started_instant),
                    })?;

                    if !config.persistent {
                        break "watched_process_exit".to_owned();
                    }

                    watch_state = WatchProcessState::Waiting;
                    info!("watch_process_waiting_for_relaunch");
                    continue;
                }

                let removed_roots = remove_stale_tree_roots(
                    &mut config,
                    &mut tree_root_starttimes,
                    watch_state.running_pid(),
                );
                if !removed_roots.is_empty() {
                    for root in &removed_roots {
                        info!("tree_root_removed pid={root}");
                    }
                    refresh_target_tasks(RefreshTargetTasksInput {
                        config: &config,
                        active_targets: &mut active_targets,
                        known_targets: &mut known_targets,
                        stats_by_task: &mut stats_by_task,
                        tree_events: &mut tree_events,
                        target_pid_map: &mut loaded.target_pid_map,
                        elapsed_ms: started.elapsed().as_millis(),
                        recording_started: recording.as_ref().map(|run| run.started_instant),
                    })?;

                    if had_tree_roots
                        && config.tree_pids.is_empty()
                        && !matches!(watch_state, WatchProcessState::Waiting)
                    {
                        break "tree_root_exit".to_owned();
                    }
                }

                refresh_target_tasks(RefreshTargetTasksInput {
                    config: &config,
                    active_targets: &mut active_targets,
                    known_targets: &mut known_targets,
                    stats_by_task: &mut stats_by_task,
                    tree_events: &mut tree_events,
                    target_pid_map: &mut loaded.target_pid_map,
                    elapsed_ms: started.elapsed().as_millis(),
                    recording_started: recording.as_ref().map(|run| run.started_instant),
                })?;
            }

            _ = watch_tick.tick(), if matches!(watch_state, WatchProcessState::Waiting) => {
                let Some(pattern) = config.watch_process.clone() else {
                    continue;
                };

                // This full /proc scan runs only while waiting for the watched process
                // to appear or relaunch. Once running, the monitor follows the root PID.
                if let Some(pid) = find_process_by_pattern_at(Path::new("/proc"), &pattern) {
                    add_watch_tree_pid(&mut config, pid);
                    tree_root_starttimes.insert(pid, process_root_starttime(pid));
                    watch_state = WatchProcessState::Running(pid);
                    info!("watch_process_relaunched pattern={} pid={}", pattern, pid);

                    refresh_target_tasks(RefreshTargetTasksInput {
                        config: &config,
                        active_targets: &mut active_targets,
                        known_targets: &mut known_targets,
                        stats_by_task: &mut stats_by_task,
                        tree_events: &mut tree_events,
                        target_pid_map: &mut loaded.target_pid_map,
                        elapsed_ms: started.elapsed().as_millis(),
                        recording_started: recording.as_ref().map(|run| run.started_instant),
                    })?;
                }
            }

            _ = scx_tick.tick() => {
                scx_tracker.sample(started.elapsed().as_millis());
            }

            ready = loaded.events.readable_mut() => {
                let mut guard = ready?;
                let rb = guard.get_inner_mut();

                while let Some(item) = rb.next() {
                    let event = unsafe {
                        (item.as_ptr() as *const SchedulerEvent).read_unaligned()
                    };

                    handle_event(HandleEventInput {
                        event,
                        config: &config,
                        started,
                        active_targets: &active_targets,
                        known_targets: &known_targets,
                        stats_by_task: &mut stats_by_task,
                        monotonic_start_ns: recording_monotonic_start_ns,
                        spike_events: spike_events.as_mut(),
                    });
                }

                guard.clear_ready();
            }
        }
    };

    let drop_counters = loaded.snapshot_drop_counters();
    log_drop_counters(&drop_counters);
    print_session_summaries(&mut stats_by_task);

    if let Some(recording) = recording {
        let spike_events_slice = spike_events
            .as_ref()
            .map(SpikeEventBuffer::as_slice)
            .unwrap_or(&[]);
        let spike_events_truncated = spike_events
            .as_ref()
            .map(SpikeEventBuffer::truncated)
            .unwrap_or(false);

        finalize_recording(FinalizeRecordingInput {
            recording: &recording,
            config: &config,
            stop_reason: &stop_reason,
            active_targets: &active_targets,
            stats_by_task: &stats_by_task,
            interval_records: &interval_records,
            tree_events: &tree_events,
            spike_events: spike_events_slice,
            spike_events_truncated,
            scx_events: scx_tracker.events(),
            drop_counters,
        })?;
    }

    if let Some(csv_path) = &config.csv_path {
        recorder::write_interval_csv(csv_path, &interval_records)?;
        println!("wrote interval CSV: {}", csv_path.display());
    }

    info!("exiting stop_reason={stop_reason}");
    Ok(())
}

async fn resolve_watch_process(config: &mut Config) -> anyhow::Result<Option<u32>> {
    let Some(pattern) = config.watch_process.clone() else {
        return Ok(None);
    };

    if let Some(pid) = find_process_by_pattern_at(Path::new("/proc"), &pattern) {
        add_watch_tree_pid(config, pid);
        return Ok(Some(pid));
    }

    wait_for_watch_process(config)
        .await?
        .ok_or_else(|| anyhow::anyhow!("stopped while waiting for --watch-process {pattern}"))
        .map(Some)
}

async fn wait_for_watch_process(config: &mut Config) -> anyhow::Result<Option<u32>> {
    let pattern = config
        .watch_process
        .clone()
        .ok_or_else(|| anyhow::anyhow!("internal error: watch_process missing"))?;

    info!(
        "watch_process_waiting pattern={} persistent={}",
        pattern, config.persistent
    );

    let mut tick = interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = signal::ctrl_c() => {
                return Ok(None);
            }
            _ = tick.tick() => {
                if let Some(pid) = find_process_by_pattern_at(Path::new("/proc"), &pattern) {
                    add_watch_tree_pid(config, pid);
                    info!("watch_process_found pattern={} pid={}", pattern, pid);
                    return Ok(Some(pid));
                }
            }
        }
    }
}

fn add_watch_tree_pid(config: &mut Config, pid: u32) {
    config.tree_pids.push(pid);
    config.tree_pids.sort_unstable();
    config.tree_pids.dedup();
}

fn remove_watch_tree_pid(config: &mut Config, pid: u32) {
    config.tree_pids.retain(|tree_pid| *tree_pid != pid);
}

fn capture_tree_root_starttimes(tree_pids: &[u32]) -> BTreeMap<u32, Option<u64>> {
    tree_pids
        .iter()
        .map(|pid| (*pid, process_root_starttime(*pid)))
        .collect()
}

fn process_root_starttime(pid: u32) -> Option<u64> {
    process_tree::process_starttime_at(Path::new("/proc"), pid)
}

fn tree_root_is_stale(pid: u32, root_starttimes: &BTreeMap<u32, Option<u64>>) -> bool {
    let current = process_root_starttime(pid);
    let expected = root_starttimes.get(&pid).copied().flatten();

    current.is_none() || expected.is_some_and(|expected| current != Some(expected))
}

fn remove_stale_tree_roots(
    config: &mut Config,
    root_starttimes: &mut BTreeMap<u32, Option<u64>>,
    watched_pid: Option<u32>,
) -> Vec<u32> {
    let mut removed = Vec::new();

    for pid in config.tree_pids.clone() {
        if Some(pid) == watched_pid {
            continue;
        }
        if tree_root_is_stale(pid, root_starttimes) {
            removed.push(pid);
            root_starttimes.remove(&pid);
        }
    }

    if !removed.is_empty() {
        config
            .tree_pids
            .retain(|tree_pid| !removed.contains(tree_pid));
    }

    removed
}

fn find_process_by_pattern_at(proc_root: &Path, pattern: &str) -> Option<u32> {
    process_tree::scan_processes_at(proc_root)
        .into_iter()
        .filter_map(|(pid, process)| {
            let score = process_match_score(pattern, &process.comm, &process.cmdline)?;
            Some((score, pid))
        })
        .max_by_key(|(score, pid)| (*score, *pid))
        .map(|(_, pid)| pid)
}

fn process_match_score(pattern: &str, comm: &str, cmdline: &str) -> Option<u8> {
    if comm == pattern {
        return Some(3);
    }

    if cmdline_executable_basename(cmdline).as_deref() == Some(pattern) {
        return Some(2);
    }

    (comm.contains(pattern) || cmdline.contains(pattern)).then_some(1)
}

fn cmdline_executable_basename(cmdline: &str) -> Option<String> {
    let executable = cmdline.split_whitespace().next()?;
    let executable = executable.replace('\\', "/");
    PathBuf::from(executable)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

fn handle_event(input: HandleEventInput<'_>) {
    let HandleEventInput {
        event,
        config,
        started,
        active_targets,
        known_targets,
        stats_by_task,
        monotonic_start_ns,
        spike_events,
    } = input;

    match event.kind {
        EVENT_RUNNABLE_LATENCY => {
            let comm = metrics::comm_to_string(&event.comm);
            let elapsed_ms = started.elapsed().as_millis();

            let task_info = active_targets
                .get(&event.pid)
                .or_else(|| known_targets.get(&event.pid));

            let stats = stats_by_task
                .entry(event.pid)
                .or_insert_with(|| metrics::TaskStats::new(event.pid, comm.clone(), elapsed_ms));

            if should_replace_unknown_comm(&stats.comm, &comm) {
                stats.comm = comm.clone();
            }

            if let Some(task_info) = task_info {
                stats.apply_task_info(task_info);
                stats.active = active_targets.contains_key(&event.pid);
            }

            stats.record(&event, config.spike_threshold_ns, elapsed_ms);

            if event.latency_ns >= config.spike_threshold_ns
                && let Some(spike_events) = spike_events
            {
                spike_events.push(recorder::SpikeEvent::from_task_stats(
                    monotonic_start_ns,
                    stats,
                    &event,
                ));
            }

            if config.verbose {
                print_event(&event, &comm, "sample");
            } else if event.latency_ns >= config.spike_threshold_ns {
                print_event(&event, &comm, "spike");
            }
        }
        other => {
            let comm = metrics::comm_to_string(&event.comm);
            info!(
                "unknown_event kind={} task={} cpu={} comm={}",
                other, event.pid, event.cpu, comm
            );
        }
    }
}

fn refresh_target_tasks(input: RefreshTargetTasksInput<'_>) -> anyhow::Result<()> {
    let RefreshTargetTasksInput {
        config,
        active_targets,
        known_targets,
        stats_by_task,
        tree_events,
        target_pid_map,
        elapsed_ms,
        recording_started,
    } = input;

    let snapshot = process_tree::target_snapshot_with_filters(
        &config.target_pids,
        &config.tree_pids,
        &config.task_filters,
    );
    let desired_tasks = snapshot.tasks;

    if desired_tasks.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many target tasks after tree/thread expansion: got {}, but TARGET_PIDS supports at most {}; try a narrower --tree-pid root or increase both userspace TARGET_PIDS_MAX and the eBPF TARGET_PIDS max entries together",
            desired_tasks.len(),
            TARGET_PIDS_MAX
        );
    }

    handle_same_tid_replacements(
        active_targets,
        &desired_tasks,
        known_targets,
        stats_by_task,
        tree_events,
        elapsed_ms,
        recording_started,
    );

    for diff in process_tree::diff_tasks(active_targets, &desired_tasks) {
        match diff.action {
            TargetDiffAction::Added => {
                let tid = diff.task.tid;
                target_pid_map.insert(tid, 1, 0).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to insert TID {tid} into TARGET_PIDS during tree refresh \
                         (map full? eBPF TARGET_PIDS max_entries is {TARGET_PIDS_MAX}): {e}"
                    )
                })?;

                known_targets.insert(tid, diff.task.clone());

                if let Some(started) = recording_started {
                    tree_events.push(TreeEvent::from_task(started, "added", &diff.task));
                }

                reactivate_or_reset_stats(stats_by_task, tid, &diff.task, elapsed_ms);

                info!(
                    "tree_target_added tid={} process_pid={} ppid={} comm={} class={}",
                    tid,
                    diff.task.process_pid,
                    diff.task.process_ppid,
                    diff.task.comm,
                    diff.task.class
                );
            }
            TargetDiffAction::Removed => {
                let tid = diff.task.tid;

                match target_pid_map.remove(&tid) {
                    Ok(()) => info!("tree_target_removed tid={tid} class={}", diff.task.class),
                    Err(e) => warn!("tree_target_remove_failed tid={tid} err={e}"),
                }

                if let Some(started) = recording_started {
                    tree_events.push(TreeEvent::from_task(started, "removed", &diff.task));
                }

                if let Some(stats) = stats_by_task.get_mut(&tid) {
                    stats.active = false;
                    stats.removed_ms =
                        recording_started.map(|started| started.elapsed().as_millis());
                }
            }
        }
    }

    *active_targets = desired_tasks;

    debug!(
        "tree_refresh_complete active_tasks={} manual_roots={} tree_roots={} process_count={}",
        active_targets.len(),
        config.target_pids.len(),
        config.tree_pids.len(),
        snapshot.process_roots.len(),
    );

    Ok(())
}

fn should_replace_unknown_comm(current: &str, incoming: &str) -> bool {
    (current == "?" || current.is_empty()) && incoming != "?" && !incoming.is_empty()
}

fn handle_same_tid_replacements(
    active_targets: &BTreeMap<u32, TaskInfo>,
    desired_tasks: &BTreeMap<u32, TaskInfo>,
    known_targets: &mut BTreeMap<u32, TaskInfo>,
    stats_by_task: &mut BTreeMap<u32, metrics::TaskStats>,
    tree_events: &mut Vec<TreeEvent>,
    elapsed_ms: u128,
    recording_started: Option<Instant>,
) {
    for (tid, desired_task) in desired_tasks {
        let Some(active_task) = active_targets.get(tid) else {
            continue;
        };

        if same_task_info(active_task, desired_task) {
            continue;
        }

        known_targets.insert(*tid, desired_task.clone());
        reset_stats_for_task_change(stats_by_task, *tid, desired_task, elapsed_ms);

        if let Some(started) = recording_started {
            tree_events.push(TreeEvent::from_task(started, "replaced", desired_task));
        }

        warn!(
            "tree_target_replaced tid={} old_process_pid={} old_comm={} old_class={} new_process_pid={} new_comm={} new_class={}",
            tid,
            active_task.process_pid,
            active_task.comm,
            active_task.class,
            desired_task.process_pid,
            desired_task.comm,
            desired_task.class,
        );
    }
}

fn reset_stats_for_task_change(
    stats_by_task: &mut BTreeMap<u32, metrics::TaskStats>,
    tid: u32,
    task_info: &TaskInfo,
    elapsed_ms: u128,
) {
    let mut stats = metrics::TaskStats::new(tid, task_info.comm.clone(), elapsed_ms);
    stats.apply_task_info(task_info);
    stats.active = true;
    stats_by_task.insert(tid, stats);
}

fn reactivate_or_reset_stats(
    stats_by_task: &mut BTreeMap<u32, metrics::TaskStats>,
    tid: u32,
    task_info: &TaskInfo,
    elapsed_ms: u128,
) {
    let Some(stats) = stats_by_task.get_mut(&tid) else {
        return;
    };

    if stats.removed_ms.is_some() && !same_logical_task(stats, task_info) {
        *stats = metrics::TaskStats::new(tid, task_info.comm.clone(), elapsed_ms);
    }

    stats.apply_task_info(task_info);
    stats.active = true;
    stats.removed_ms = None;
}

fn same_logical_task(stats: &metrics::TaskStats, task_info: &TaskInfo) -> bool {
    stats.process_pid == Some(task_info.process_pid)
        && stats.process_comm == task_info.process_comm
        && stats.process_starttime_ticks == task_info.process_starttime_ticks
        && stats.task_starttime_ticks == task_info.task_starttime_ticks
        && stats.comm == task_info.comm
        && stats.class == task_info.class
}

fn same_task_info(left: &TaskInfo, right: &TaskInfo) -> bool {
    left.tid == right.tid
        && left.process_pid == right.process_pid
        && left.process_ppid == right.process_ppid
        && left.comm == right.comm
        && left.process_comm == right.process_comm
        && left.process_starttime_ticks == right.process_starttime_ticks
        && left.task_starttime_ticks == right.task_starttime_ticks
        && left.class == right.class
}

fn log_drop_counters(drop_counters: &ebpf_loader::DropCountersSnapshot) {
    if drop_counters.total() == 0 {
        debug!("ebpf_drop_counters total=0");
        return;
    }

    warn!(
        "ebpf_drop_counters total={} wakeup_times_insert_failed={} ringbuf_reserve_failed={}",
        drop_counters.total(),
        drop_counters.wakeup_times_insert_failed,
        drop_counters.ringbuf_reserve_failed,
    );
}
