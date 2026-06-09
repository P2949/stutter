use std::{collections::BTreeMap, time::Instant};

use aya::maps::{HashMap as AyaHashMap, MapData};
use log::info;
use stutter_core::ids::Tid;

use crate::{
    metrics::{self, TaskStatsMap},
    process_tree::{TargetDiffAction, TargetSnapshotInput, TaskInfo, TaskMap as ProcessTaskMap},
    recorder::TreeEvent,
};

// ARCH: The active task map is updated directly by TaskTracker; unused speculative
// refresh-plan abstractions were removed to keep target selection behavior explicit.

pub struct RefreshInput<'a> {
    pub target_snapshot_input: TargetSnapshotInput<'a>,
    pub max_tasks: usize,
    pub tree_events: &'a mut Vec<TreeEvent>,
    pub target_pid_map: &'a mut AyaHashMap<MapData, u32, u8>,
    pub prev_faults_map: Option<&'a mut AyaHashMap<MapData, u32, [u64; 2]>>,
    pub elapsed_ms: u64,
    pub recording_started: Option<Instant>,
}

pub struct RefreshInternalInput<'a> {
    pub snapshot: crate::process_tree::TargetSnapshot,
    pub max_tasks: usize,
    pub tree_events: &'a mut Vec<TreeEvent>,
    pub elapsed_ms: u64,
    pub recording_started: Option<Instant>,
    pub prev_faults_map: Option<&'a mut AyaHashMap<MapData, u32, [u64; 2]>>,
    pub target_pid_map: &'a mut dyn BpfTaskMap,
}

pub type TaskExeInodesMap = BTreeMap<u32, (Option<u64>, Option<u64>, Option<u64>)>;

#[derive(Default)]
pub struct TaskTracker {
    pub active_targets: ProcessTaskMap,
    pub known_targets: ProcessTaskMap,
    pub stats_by_task: TaskStatsMap,
    pub prev_faults_snapshot: BTreeMap<u32, (u64, u64)>,
    pub task_exe_inodes: TaskExeInodesMap,
    pub cache: crate::process_tree::ProcessCache,
}

pub trait BpfTaskMap: Send {
    fn insert(&mut self, k: u32, v: u8, f: u64) -> anyhow::Result<()>;
    fn remove(&mut self, k: &u32) -> anyhow::Result<()>;
}

impl BpfTaskMap for AyaHashMap<MapData, u32, u8> {
    fn insert(&mut self, k: u32, v: u8, f: u64) -> anyhow::Result<()> {
        AyaHashMap::insert(self, k, v, f).map_err(|e| anyhow::anyhow!(e))
    }
    fn remove(&mut self, k: &u32) -> anyhow::Result<()> {
        AyaHashMap::remove(self, k).map_err(|e| anyhow::anyhow!(e))
    }
}

impl TaskTracker {
    pub async fn refresh(
        &mut self,
        mut input: RefreshInput<'_>,
    ) -> anyhow::Result<crate::process_tree::ScanBudgetReport> {
        let snapshot = crate::process_tree::target_snapshot(
            input
                .target_snapshot_input
                .previous_tasks(Some(&self.active_targets)),
        );

        let budget_report = snapshot.budget_report.clone();

        self.refresh_internal(RefreshInternalInput {
            snapshot,
            max_tasks: input.max_tasks,
            tree_events: input.tree_events,
            elapsed_ms: input.elapsed_ms,
            recording_started: input.recording_started,
            prev_faults_map: input.prev_faults_map.as_deref_mut(),
            target_pid_map: input.target_pid_map,
        })
        .await?;

        Ok(budget_report)
    }

    pub async fn refresh_internal(
        &mut self,
        input: RefreshInternalInput<'_>,
    ) -> anyhow::Result<()> {
        let snapshot = input.snapshot;
        let mut prev_faults_map = input.prev_faults_map;
        if snapshot.tasks.len() > input.max_tasks {
            anyhow::bail!(
                "too many target tasks after expansion: got {}, but --max-tasks is {}",
                snapshot.tasks.len(),
                input.max_tasks
            );
        }

        if snapshot.tasks.len() > crate::config::TARGET_PIDS_MAX {
            anyhow::bail!(
                "too many target tasks after expansion: got {}, but TARGET_PIDS supports at most {}",
                snapshot.tasks.len(),
                crate::config::TARGET_PIDS_MAX
            );
        }

        self.handle_replacements(
            &snapshot.tasks,
            input.tree_events,
            &mut prev_faults_map,
            input.elapsed_ms,
            input.recording_started,
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
            let tid = task.task_id();
            let tid_raw = tid.as_u32();
            match action {
                TargetDiffAction::Added => {
                    input.target_pid_map.insert(tid_raw, 1, 0)?;

                    let action_name = target_event_action(task.from_cgroup, "added");
                    info!(
                        "target_{} tid={} pid={} comm={} class={:?}",
                        action_name, tid, task.process_pid, task.comm, task.class
                    );

                    input.tree_events.push(TreeEvent {
                        elapsed_ms: input.elapsed_ms,
                        action: action_name.to_owned(),
                        tid: task.task_id(),
                        process_pid: task.process_id(),
                        process_ppid: task.parent_process_id(),
                        comm: task.comm.clone(),
                        process_comm: task.process_comm.clone(),
                        class: task.class,
                        from_cgroup: task.from_cgroup,
                    });

                    reactivate_or_reset_stats_inner(
                        &mut self.stats_by_task,
                        input.recording_started,
                        tid,
                        &task,
                        input.elapsed_ms,
                    );

                    update_task_exe_info(&mut self.task_exe_inodes, tid_raw, &task);

                    self.active_targets.insert(tid, task.clone());
                    self.known_targets.insert(tid, task);
                }
                TargetDiffAction::Removed => {
                    let action_name = target_event_action(task.from_cgroup, "removed");
                    info!(
                        "target_{} tid={} pid={} comm={} class={:?}",
                        action_name, tid, task.process_pid, task.comm, task.class
                    );

                    input.tree_events.push(TreeEvent {
                        elapsed_ms: input.elapsed_ms,
                        action: action_name.to_owned(),
                        tid: task.task_id(),
                        process_pid: task.process_id(),
                        process_ppid: task.parent_process_id(),
                        comm: task.comm.clone(),
                        process_comm: task.process_comm.clone(),
                        class: task.class,
                        from_cgroup: task.from_cgroup,
                    });

                    if let Some(stats) = self.stats_by_task.get_mut(&tid) {
                        stats.active = false;
                        stats.removed_ms = Some(input.elapsed_ms);
                    }

                    remove_prev_faults_state(
                        &mut prev_faults_map,
                        &mut self.prev_faults_snapshot,
                        tid_raw,
                    );

                    input.target_pid_map.remove(&tid_raw)?;
                    self.active_targets.remove(&tid);
                }
            }
        }

        Ok(())
    }

    pub fn handle_replacements(
        &mut self,
        desired_tasks: &ProcessTaskMap,
        tree_events: &mut Vec<TreeEvent>,
        prev_faults_map: &mut Option<&mut AyaHashMap<MapData, u32, [u64; 2]>>,
        elapsed_ms: u64,
        recording_started: Option<Instant>,
    ) {
        for (tid, desired) in desired_tasks {
            if let Some(active) = self.active_targets.get(tid)
                && !crate::process_tree::same_logical_task(active, desired)
            {
                let tid_raw = tid.as_u32();
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
                    tid: desired.task_id(),
                    process_pid: desired.process_id(),
                    process_ppid: desired.parent_process_id(),
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

                update_task_exe_info(&mut self.task_exe_inodes, tid_raw, desired);

                remove_prev_faults_state(prev_faults_map, &mut self.prev_faults_snapshot, tid_raw);

                self.active_targets.insert(*tid, desired.clone());
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
    stats_by_task: &mut TaskStatsMap,
    recording_started: Option<Instant>,
    tid: Tid,
    task_info: &TaskInfo,
    elapsed_ms: u64,
) {
    let mut stats = metrics::TaskStats::new(tid.as_u32(), task_info.comm.clone(), elapsed_ms);
    stats.apply_task_info(task_info);
    if let Some(_started) = recording_started {
        stats.recording_started(elapsed_ms);
    }
    stats_by_task.insert(tid, stats);
}

pub fn reactivate_or_reset_stats_inner(
    stats_by_task: &mut TaskStatsMap,
    recording_started: Option<Instant>,
    tid: Tid,
    task_info: &TaskInfo,
    elapsed_ms: u64,
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
    crate::process_tree::same_logical_task(left, right)
        && left.comm == right.comm
        && left.process_comm == right.process_comm
        && left.class == right.class
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::model::MonitorConfig,
        process_tree::{TargetSnapshot, TaskClass},
    };

    #[tokio::test]
    async fn test_refresh_exceeds_max_tasks() {
        let mut tracker = TaskTracker::default();
        let mut config = MonitorConfig::default();
        config.target.max_tasks = 1;

        let mut tasks = BTreeMap::new();
        tasks.insert(
            1.into(),
            task_info(1, 100, "proc1", "task1", TaskClass::Game),
        );
        tasks.insert(
            2.into(),
            task_info(2, 100, "proc1", "task2", TaskClass::Game),
        );

        let snapshot = TargetSnapshot {
            tasks,
            process_roots: std::collections::BTreeSet::from([100.into()]),
            budget_report: crate::process_tree::ScanBudgetReport::default(),
        };

        struct MockMap;
        impl BpfTaskMap for MockMap {
            fn insert(&mut self, _: u32, _: u8, _: u64) -> anyhow::Result<()> {
                Ok(())
            }
            fn remove(&mut self, _: &u32) -> anyhow::Result<()> {
                Ok(())
            }
        }
        let mut mock_map = MockMap;

        let mut tree_events = Vec::new();
        let result = tracker
            .refresh_internal(RefreshInternalInput {
                snapshot,
                max_tasks: config.target.max_tasks,
                tree_events: &mut tree_events,
                elapsed_ms: 0,
                recording_started: None,
                prev_faults_map: None,
                target_pid_map: &mut mock_map,
            })
            .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("too many target tasks")
        );
        assert!(tracker.active_targets.is_empty());
        assert!(tracker.stats_by_task.is_empty());
        assert!(tree_events.is_empty());
    }

    fn task_info(
        tid: u32,
        process_pid: u32,
        process_comm: &str,
        comm: &str,
        class: TaskClass,
    ) -> TaskInfo {
        TaskInfo {
            tid: tid.into(),
            process_pid: process_pid.into(),
            process_ppid: 1.into(),
            comm: comm.into(),
            process_comm: process_comm.into(),
            process_starttime_ticks: Some(u64::from(process_pid) * 10),
            task_starttime_ticks: Some(u64::from(tid) * 10),
            exe_dev: None,
            exe_ino: None,
            class,
            sched_policy: None,
            from_cgroup: false,
        }
    }

    #[test]
    fn test_same_logical_task() {
        let mut t1 = task_info(1, 100, "proc1", "task1", TaskClass::Game);
        let mut t2 = t1.clone();

        // same pid + same starttimes => same
        assert!(crate::process_tree::same_logical_task(&t1, &t2));

        // different pid => not same
        t2.process_pid = 101.into();
        assert!(!crate::process_tree::same_logical_task(&t1, &t2));
        t2.process_pid = 100.into();

        // same pid + all starttimes None + different exe inode => not same
        t1.process_starttime_ticks = None;
        t1.task_starttime_ticks = None;
        t2.process_starttime_ticks = None;
        t2.task_starttime_ticks = None;
        t1.exe_dev = Some(1);
        t1.exe_ino = Some(10);
        t2.exe_dev = Some(1);
        t2.exe_ino = Some(11);
        assert!(!crate::process_tree::same_logical_task(&t1, &t2));

        // same pid + missing starttimes + same exe inode + same comm => same
        t2.exe_ino = Some(10);
        assert!(crate::process_tree::same_logical_task(&t1, &t2));

        // same pid + missing starttimes + same exe inode + different comm => not same
        t2.comm = "task2".to_owned();
        assert!(!crate::process_tree::same_logical_task(&t1, &t2));
    }

    #[test]
    fn test_handle_replacements() {
        let mut tracker = TaskTracker::default();
        let tid = 7;

        // 1. active_targets contains tid=7 comm="old" starttime=10
        let mut old_task = task_info(tid, 100, "old_proc", "old", TaskClass::Game);
        old_task.task_starttime_ticks = Some(10);
        tracker.active_targets.insert(tid.into(), old_task.clone());
        tracker.known_targets.insert(tid.into(), old_task.clone());
        tracker.stats_by_task.insert(
            tid.into(),
            metrics::TaskStats::new(tid, "old".to_string(), 0),
        );

        // 2. desired snapshot contains tid=7 comm="new" starttime=20
        let mut new_task = task_info(tid, 101, "new_proc", "new", TaskClass::Game);
        new_task.task_starttime_ticks = Some(20);
        let mut desired = BTreeMap::new();
        desired.insert(tid.into(), new_task.clone());

        // 3. call handle_replacements
        let mut tree_events = Vec::new();
        tracker.handle_replacements(&desired, &mut tree_events, &mut None, 100, None);

        // 4. assert active_targets[7].comm == "new"
        assert_eq!(tracker.active_targets.get(&tid).unwrap().comm, "new");
        assert_eq!(tracker.active_targets.get(&tid).unwrap().process_pid, 101);

        // 5. assert known_targets[7].comm == "new"
        assert_eq!(tracker.known_targets.get(&tid).unwrap().comm, "new");

        // 6. assert stats_by_task[7].comm == "new"
        assert_eq!(tracker.stats_by_task.get(&tid).unwrap().comm, "new");
    }
}
