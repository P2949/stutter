use std::{
    collections::BTreeMap,
    path::Path,
    time::Instant,
};

use aya::maps::{HashMap as AyaHashMap, MapData};
use log::info;

use crate::{
    cli::Config,
    metrics,
    process_tree::{TargetDiffAction, TaskInfo},
    recorder::TreeEvent,
};

pub type TaskExeInodesMap = BTreeMap<u32, (Option<u64>, Option<u64>, Option<u64>)>;

#[derive(Default)]
pub struct TaskTracker {
    pub active_targets: BTreeMap<u32, TaskInfo>,
    pub known_targets: BTreeMap<u32, TaskInfo>,
    pub stats_by_task: BTreeMap<u32, metrics::TaskStats>,
    pub prev_faults_snapshot: BTreeMap<u32, (u64, u64)>,
    pub task_exe_inodes: TaskExeInodesMap,
    pub cache: crate::process_tree::ProcessCache,
}

impl TaskTracker {
    pub async fn refresh(
        &mut self,
        config: &Config,
        tree_events: &mut Vec<TreeEvent>,
        target_pid_map: &mut AyaHashMap<MapData, u32, u8>,
        mut prev_faults_map: Option<&mut AyaHashMap<MapData, u32, [u64; 2]>>,
        elapsed_ms: u128,
        recording_started: Option<Instant>,
    ) -> anyhow::Result<()> {
        let snapshot = crate::process_tree::target_snapshot_filtered_at_with_options(
            crate::process_tree::TargetSnapshotInput {
                proc_root: Path::new("/proc"),
                manual_pids: &config.target_pids,
                tree_pids: &config.tree_pids,
                cgroup_path: config.cgroupv2.as_deref(),
                exclude_tree_pids: &config.exclude_tree_pids,
                filters: &config.task_filters,
                keep_missing_pid: config.keep_missing_pid,
                cache: &mut self.cache,
                previous_tasks: Some(&self.active_targets),
            },
        );

        self.handle_replacements(
            &snapshot.tasks,
            tree_events,
            &mut prev_faults_map,
            elapsed_ms,
            recording_started,
        );

        let diffs: Vec<(TargetDiffAction, TaskInfo)> = {
            let active_snapshot = self.active_targets.clone();
            crate::process_tree::diff_tasks_ref(&active_snapshot, &snapshot.tasks)
                .into_iter()
                .map(|d| (d.action, d.task.clone()))
                .collect()
        };

        if diffs.is_empty() {
            return Ok(());
        }

        for (action, task) in diffs {
            let tid = task.tid;
            match action {
                TargetDiffAction::Added => {
                    let action_name = target_event_action(task.from_cgroup, "added");
                    info!(
                        "target_{} tid={} pid={} comm={} class={:?}",
                        action_name, tid, task.process_pid, task.comm, task.class
                    );

                    tree_events.push(TreeEvent {
                        elapsed_ms,
                        action: action_name.to_owned(),
                        tid,
                        process_pid: task.process_pid,
                        process_ppid: task.process_ppid,
                        comm: task.comm.clone(),
                        process_comm: task.process_comm.clone(),
                        class: task.class,
                        from_cgroup: task.from_cgroup,
                    });

                    reactivate_or_reset_stats_inner(
                        &mut self.stats_by_task,
                        recording_started,
                        tid,
                        &task,
                        elapsed_ms,
                    );

                    update_task_exe_info(&mut self.task_exe_inodes, tid, &task);

                    target_pid_map.insert(tid, 1, 0)?;
                    self.active_targets.insert(tid, task);
                }
                TargetDiffAction::Removed => {
                    let action_name = target_event_action(task.from_cgroup, "removed");
                    info!(
                        "target_{} tid={} pid={} comm={} class={:?}",
                        action_name, tid, task.process_pid, task.comm, task.class
                    );

                    tree_events.push(TreeEvent {
                        elapsed_ms,
                        action: action_name.to_owned(),
                        tid,
                        process_pid: task.process_pid,
                        process_ppid: task.process_ppid,
                        comm: task.comm.clone(),
                        process_comm: task.process_comm.clone(),
                        class: task.class,
                        from_cgroup: task.from_cgroup,
                    });

                    if let Some(stats) = self.stats_by_task.get_mut(&tid) {
                        stats.active = false;
                        stats.removed_ms = Some(elapsed_ms);
                    }

                    remove_prev_faults_state(
                        &mut prev_faults_map,
                        &mut self.prev_faults_snapshot,
                        tid,
                    );

                    let _ = target_pid_map.remove(&tid);
                    self.active_targets.remove(&tid);
                }
            }
        }

        Ok(())
    }

    pub fn handle_replacements(
        &mut self,
        desired_tasks: &BTreeMap<u32, TaskInfo>,
        tree_events: &mut Vec<TreeEvent>,
        prev_faults_map: &mut Option<&mut AyaHashMap<MapData, u32, [u64; 2]>>,
        elapsed_ms: u128,
        recording_started: Option<Instant>,
    ) {
        for (tid, desired) in desired_tasks {
            if let Some(active) = self.active_targets.get(tid)
                && !same_logical_task(active, desired)
            {
                info!(
                    "target_replaced tid={} old_pid={} new_pid={} old_comm={} new_comm={} old_class={:?} new_class={:?}",
                    tid,
                    active.process_pid,
                    desired.process_pid,
                    active.comm,
                    desired.comm,
                    active.class,
                    desired.class
                );

                tree_events.push(TreeEvent {
                    elapsed_ms,
                    action: "replaced".to_owned(),
                    tid: *tid,
                    process_pid: desired.process_pid,
                    process_ppid: desired.process_ppid,
                    comm: desired.comm.clone(),
                    process_comm: desired.process_comm.clone(),
                    class: desired.class,
                    from_cgroup: desired.from_cgroup,
                });

                reset_stats_for_task_change(
                    &mut self.stats_by_task,
                    recording_started,
                    *tid,
                    desired,
                    elapsed_ms,
                );

                update_task_exe_info(&mut self.task_exe_inodes, *tid, desired);

                remove_prev_faults_state(prev_faults_map, &mut self.prev_faults_snapshot, *tid);

                self.known_targets.insert(*tid, desired.clone());
            }
        }
    }
}


pub fn remove_prev_faults_state(
    prev_faults_map: &mut Option<&mut AyaHashMap<MapData, u32, [u64; 2]>>,
    prev_faults_snapshot: &mut BTreeMap<u32, (u64, u64)>,
    tid: u32,
) {
    if let Some(m) = prev_faults_map {
        let _ = m.remove(&tid);
    }
    prev_faults_snapshot.remove(&tid);
}

pub fn reset_stats_for_task_change(
    stats_by_task: &mut BTreeMap<u32, metrics::TaskStats>,
    recording_started: Option<Instant>,
    tid: u32,
    task_info: &TaskInfo,
    elapsed_ms: u128,
) {
    let mut stats = metrics::TaskStats::new(tid, task_info.comm.clone(), elapsed_ms);
    stats.apply_task_info(task_info);
    if let Some(_started) = recording_started {
        stats.recording_started(elapsed_ms);
    }
    stats_by_task.insert(tid, stats);
}

pub fn reactivate_or_reset_stats_inner(
    stats_by_task: &mut BTreeMap<u32, metrics::TaskStats>,
    recording_started: Option<Instant>,
    tid: u32,
    task_info: &TaskInfo,
    elapsed_ms: u128,
) {
    if let Some(stats) = stats_by_task.get_mut(&tid)
        && same_task_info(task_info, &stats.task_info())
    {
        stats.active = true;
        stats.removed_ms = None;
        return;
    }

    reset_stats_for_task_change(stats_by_task, recording_started, tid, task_info, elapsed_ms);
}

pub fn same_logical_task(left: &TaskInfo, right: &TaskInfo) -> bool {
    left.process_pid == right.process_pid
        && left.task_starttime_ticks == right.task_starttime_ticks
        && left.process_starttime_ticks == right.process_starttime_ticks
}

pub fn update_task_exe_info(
    task_exe_inodes: &mut TaskExeInodesMap,
    tid: u32,
    task_info: &TaskInfo,
) {
    task_exe_inodes.insert(
        tid,
        (
            task_info.exe_dev,
            task_info.exe_ino,
            task_info.process_starttime_ticks,
        ),
    );
}

pub fn same_task_info(left: &TaskInfo, right: &TaskInfo) -> bool {
    left.process_pid == right.process_pid
        && left.comm == right.comm
        && left.process_comm == right.process_comm
        && left.class == right.class
        && left.task_starttime_ticks == right.task_starttime_ticks
        && left.process_starttime_ticks == right.process_starttime_ticks
}

pub fn target_event_action(from_cgroup: bool, action: &'static str) -> &'static str {
    if from_cgroup {
        match action {
            "added" => "cgroup_added",
            "removed" => "cgroup_removed",
            _ => action,
        }
    } else {
        action
    }
}

pub fn should_replace_unknown_comm(current: &str, incoming: &str) -> bool {
    (current == "?" || current.is_empty()) && incoming != "?" && !incoming.is_empty()
}
