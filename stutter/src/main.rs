mod cli;
mod ebpf_loader;
mod metadata;
mod metrics;
mod process_tree;
mod recorder;
mod report;

use std::{collections::BTreeMap, future, time::Instant};

use aya::maps::{HashMap as AyaHashMap, MapData};
use cli::{AppCommand, Config, parse_app_command};
use log::{debug, info, warn};
use metrics::{format_latency, print_event, print_interval_summaries, print_session_summaries};
use process_tree::{TargetDiffAction, TaskInfo};
use recorder::{
    FinalizeRecordingInput, IntervalRecord, SpikeEvent, TreeEvent, finalize_recording,
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
    let mut spike_events = recording.as_ref().map(|_| Vec::<SpikeEvent>::new());

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

                    handle_event(
                        event,
                        &config,
                        started,
                        &active_targets,
                        &known_targets,
                        &mut stats_by_task,
                        spike_events.as_mut(),
                    );
                }

                guard.clear_ready();
            }
        }
    };

    print_session_summaries(&mut stats_by_task);

    if let Some(recording) = recording {
        finalize_recording(FinalizeRecordingInput {
            recording: &recording,
            config: &config,
            stop_reason: &stop_reason,
            active_targets: &active_targets,
            stats_by_task: &stats_by_task,
            interval_records: &interval_records,
            tree_events: &tree_events,
            spike_events: spike_events.as_deref().unwrap_or(&[]),
        })?;
    }

    info!("exiting stop_reason={stop_reason}");
    Ok(())
}

fn handle_event(
    event: SchedulerEvent,
    config: &Config,
    started: Instant,
    active_targets: &BTreeMap<u32, TaskInfo>,
    known_targets: &BTreeMap<u32, TaskInfo>,
    stats_by_task: &mut BTreeMap<u32, metrics::TaskStats>,
    spike_events: Option<&mut Vec<SpikeEvent>>,
) {
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
                spike_events.push(SpikeEvent::from_task_stats(elapsed_ms, stats, &event));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_tree::TaskClass;

    #[test]
    fn reused_tid_with_different_task_resets_stats_after_removal() {
        let mut stats_by_task = BTreeMap::from([(
            42,
            task_stats_with_info(42, 100, "old-game", "old-thread", TaskClass::Game, 10),
        )]);
        {
            let stats = stats_by_task.get_mut(&42).unwrap();
            stats.removed_ms = Some(50);
            stats.session_latency.record(3_000_000);
        }

        let new_task = task_info(42, 200, "new-game", "new-thread", TaskClass::Helper);
        reactivate_or_reset_stats(&mut stats_by_task, 42, &new_task, 77);

        let stats = stats_by_task.get(&42).unwrap();
        assert_eq!(stats.first_seen_ms, 77);
        assert_eq!(stats.last_seen_ms, 77);
        assert_eq!(stats.session_latency.count, 0);
        assert_eq!(stats.process_pid, Some(200));
        assert_eq!(stats.process_comm, "new-game");
        assert_eq!(stats.comm, "new-thread");
        assert_eq!(stats.class, TaskClass::Helper);
        assert!(stats.active);
        assert_eq!(stats.removed_ms, None);
    }

    #[test]
    fn same_reused_tid_reactivates_without_clearing_stats() {
        let mut stats_by_task = BTreeMap::from([(
            42,
            task_stats_with_info(42, 100, "game", "worker", TaskClass::Game, 10),
        )]);
        {
            let stats = stats_by_task.get_mut(&42).unwrap();
            stats.removed_ms = Some(50);
            stats.session_latency.record(3_000_000);
        }

        let same_task = task_info(42, 100, "game", "worker", TaskClass::Game);
        reactivate_or_reset_stats(&mut stats_by_task, 42, &same_task, 77);

        let stats = stats_by_task.get(&42).unwrap();
        assert_eq!(stats.first_seen_ms, 10);
        assert_eq!(stats.session_latency.count, 1);
        assert_eq!(stats.removed_ms, None);
        assert!(stats.active);
    }

    #[test]
    fn event_comm_updates_only_unknown_existing_name() {
        let config = Config {
            target_pids: vec![7],
            tree_pids: vec![],
            summary_period_ms: 1_000,
            spike_threshold_ns: 1_000_000,
            verbose: false,
            recording: None,
            max_duration: None,
        };
        let active_targets = BTreeMap::new();
        let known_targets = BTreeMap::new();
        let mut stats_by_task =
            BTreeMap::from([(7, metrics::TaskStats::new(7, "?".to_owned(), 0))]);

        handle_event(
            scheduler_event(7, "real-name"),
            &config,
            Instant::now(),
            &active_targets,
            &known_targets,
            &mut stats_by_task,
            None,
        );

        assert_eq!(stats_by_task.get(&7).unwrap().comm, "real-name");

        handle_event(
            scheduler_event(7, "later-name"),
            &config,
            Instant::now(),
            &active_targets,
            &known_targets,
            &mut stats_by_task,
            None,
        );

        assert_eq!(stats_by_task.get(&7).unwrap().comm, "real-name");
    }

    #[test]
    fn recording_spike_events_capture_only_threshold_crossing_events() {
        let config = Config {
            target_pids: vec![7],
            tree_pids: vec![],
            summary_period_ms: 1_000,
            spike_threshold_ns: 1_000_000,
            verbose: false,
            recording: None,
            max_duration: None,
        };
        let active_targets = BTreeMap::from([(
            7,
            task_info(7, 77, "KingdomCome.exe", "RenderThread", TaskClass::Game),
        )]);
        let known_targets = BTreeMap::new();
        let mut stats_by_task = BTreeMap::new();
        let mut spike_events = Vec::new();

        handle_event(
            scheduler_event_with_latency(7, "RenderThread", 999_999),
            &config,
            Instant::now(),
            &active_targets,
            &known_targets,
            &mut stats_by_task,
            Some(&mut spike_events),
        );
        assert!(spike_events.is_empty());

        handle_event(
            scheduler_event_with_latency(7, "RenderThread", 1_000_000),
            &config,
            Instant::now(),
            &active_targets,
            &known_targets,
            &mut stats_by_task,
            Some(&mut spike_events),
        );

        assert_eq!(spike_events.len(), 1);
        let spike = &spike_events[0];
        assert_eq!(spike.task, 7);
        assert!(spike.active);
        assert_eq!(spike.class, TaskClass::Game);
        assert_eq!(spike.process_pid, Some(77));
        assert_eq!(spike.process_comm, "KingdomCome.exe");
        assert_eq!(spike.comm, "RenderThread");
        assert_eq!(spike.cpu, 0);
        assert_eq!(spike.prio, 120);
        assert_eq!(spike.latency_ns, 1_000_000);
        assert_eq!(spike.wakeup_ns, 100);
        assert_eq!(spike.switch_ns, 1_000_100);
    }

    fn task_stats_with_info(
        tid: u32,
        process_pid: u32,
        process_comm: &str,
        comm: &str,
        class: TaskClass,
        first_seen_ms: u128,
    ) -> metrics::TaskStats {
        let mut stats = metrics::TaskStats::new(tid, comm.to_owned(), first_seen_ms);
        stats.apply_task_info(&task_info(tid, process_pid, process_comm, comm, class));
        stats
    }

    fn task_info(
        tid: u32,
        process_pid: u32,
        process_comm: &str,
        comm: &str,
        class: TaskClass,
    ) -> TaskInfo {
        TaskInfo {
            tid,
            process_pid,
            process_ppid: 1,
            comm: comm.to_owned(),
            process_comm: process_comm.to_owned(),
            class,
        }
    }

    fn scheduler_event(pid: u32, comm: &str) -> SchedulerEvent {
        scheduler_event_with_latency(pid, comm, 10)
    }

    fn scheduler_event_with_latency(pid: u32, comm: &str, latency_ns: u64) -> SchedulerEvent {
        let mut comm_bytes = [0; 16];
        for (idx, byte) in comm.as_bytes().iter().take(15).enumerate() {
            comm_bytes[idx] = *byte;
        }

        SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            pid,
            cpu: 0,
            prio: 120,
            wakeup_ns: 100,
            switch_ns: 100 + latency_ns,
            latency_ns,
            comm: comm_bytes,
        }
    }
}
