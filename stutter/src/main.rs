mod cli;
mod ebpf_loader;
mod metadata;
mod metrics;
mod process_tree;
mod recorder;
mod report;

#[cfg(test)]
mod regression_tests;

use std::{collections::BTreeMap, future, time::Instant};

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

async fn run_monitor(config: Config) -> anyhow::Result<()> {
    let recording = prepare_recording(&config)?;
    let mut loaded = ebpf_loader::load_and_attach()?;

    let mut active_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let mut known_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let mut stats_by_task: BTreeMap<u32, metrics::TaskStats> = BTreeMap::new();
    let mut interval_records: Vec<IntervalRecord> = Vec::new();
    let mut tree_events: Vec<TreeEvent> = Vec::new();
    let mut spike_events = recording.as_ref().map(|_| SpikeEventBuffer::default());
    let recording_monotonic_start_ns = recording.as_ref().and_then(|run| run.monotonic_start_ns);

    let started = Instant::now();

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

    let duration_future = async {
        if let Some(max_duration) = config.max_duration {
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
        })?;
    }

    info!("exiting stop_reason={stop_reason}");
    Ok(())
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

    let snapshot = process_tree::target_snapshot(&config.target_pids, &config.tree_pids);
    let desired_tasks = snapshot.tasks;

    if desired_tasks.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many target tasks after tree/thread expansion: got {}, but TARGET_PIDS supports at most {}; try a narrower --tree-pid root or increase both userspace TARGET_PIDS_MAX and the eBPF TARGET_PIDS max entries together",
            desired_tasks.len(),
            TARGET_PIDS_MAX
        );
    }

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
        && stats.comm == task_info.comm
        && stats.class == task_info.class
}
