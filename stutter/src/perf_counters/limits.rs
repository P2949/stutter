use std::{collections::BTreeSet, fs};

use log::warn;
use stutter_core::ids::Tid;

use crate::{
    metrics::{TaskStats, TaskStatsMap},
    process_tree::{TaskClass, TaskInfo, TaskMap},
};

#[derive(Clone, Debug)]
pub struct CpuPerfConfig {
    pub include_kernel: bool,
    pub max_tasks: usize,
    pub collect_cache_refs: bool,
}

pub(super) fn normalize_config(mut config: CpuPerfConfig) -> CpuPerfConfig {
    if config.max_tasks == 0 {
        config.max_tasks = 1;
    }

    let event_count = event_count(config.collect_cache_refs) as u64;
    if let Some(available) = available_fd_budget() {
        let requested_fds = config.max_tasks as u64 * event_count;
        let half_budget = available / 2;
        if requested_fds > half_budget && half_budget >= event_count {
            let capped = (half_budget / event_count).max(1) as usize;
            warn!(
                "cpu_perf requested up to {} tasks with {} events each ({} fds), but remaining fd budget is about {}; reducing cpu_perf_max_tasks to {}",
                config.max_tasks, event_count, requested_fds, available, capped
            );
            config.max_tasks = config.max_tasks.min(capped);
        } else if requested_fds > available {
            warn!(
                "cpu_perf requested up to {} tasks with {} events each ({} fds), but remaining fd budget is about {}; perf counters may be incomplete",
                config.max_tasks, event_count, requested_fds, available
            );
        }
    }

    config
}

fn available_fd_budget() -> Option<u64> {
    let rlim_cur = crate::syscall::get_nofile_rlimit_cur().ok()?;
    if rlim_cur == libc::RLIM_INFINITY {
        return None;
    }

    let open_count = fs::read_dir("/proc/self/fd").ok()?.count() as u64;
    Some(rlim_cur.saturating_sub(open_count))
}

fn event_count(collect_cache_refs: bool) -> usize {
    if collect_cache_refs { 4 } else { 3 }
}

pub(super) fn select_target_tids(
    active_targets: &TaskMap,
    stats_by_task: &TaskStatsMap,
    max_tasks: usize,
) -> BTreeSet<Tid> {
    let mut candidates = active_targets
        .iter()
        .map(|(tid, info)| (*tid, task_perf_priority(info, stats_by_task.get(tid))))
        .collect::<Vec<_>>();

    candidates.sort_by(|(left_tid, left_priority), (right_tid, right_priority)| {
        right_priority
            .cmp(left_priority)
            .then_with(|| left_tid.cmp(right_tid))
    });

    candidates
        .into_iter()
        .take(max_tasks)
        .map(|(tid, _)| tid)
        .collect()
}

fn task_perf_priority(info: &TaskInfo, stats: Option<&TaskStats>) -> u64 {
    let class_score = match info.class {
        TaskClass::Game => 1_000_000,
        TaskClass::GameScope => 900_000,
        TaskClass::Compositor => 800_000,
        TaskClass::Render => 750_000,
        TaskClass::WineServer => 700_000,
        TaskClass::GameHelper => 600_000,
        TaskClass::Launcher => 200_000,
        TaskClass::Service => 150_000,
        TaskClass::SteamRuntime => 100_000,
        TaskClass::Helper => 50_000,
        TaskClass::Unknown => 10_000,
        _ => 10_000,
    };

    let latency_score = stats
        .map(|stats| stats.session_latency.max_ns.min(1_000_000_000))
        .unwrap_or(0);

    class_score + latency_score
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn task_info(tid: u32, class: TaskClass) -> TaskInfo {
        TaskInfo {
            tid: tid.into(),
            process_pid: tid.into(),
            process_ppid: 0.into(),
            comm: format!("task-{tid}"),
            process_comm: format!("proc-{tid}"),
            process_starttime_ticks: Some(tid as u64),
            task_starttime_ticks: Some(tid as u64),
            exe_dev: None,
            exe_ino: None,
            class,
            sched_policy: None,
            from_cgroup: false,
        }
    }

    #[test]
    fn selects_high_priority_tasks_deterministically() {
        let active_targets = BTreeMap::from([
            (Tid::new(10), task_info(10, TaskClass::Helper)),
            (Tid::new(20), task_info(20, TaskClass::Game)),
            (Tid::new(30), task_info(30, TaskClass::Launcher)),
        ]);
        let selected = select_target_tids(&active_targets, &BTreeMap::new(), 2);

        assert_eq!(selected, BTreeSet::from([Tid::new(20), Tid::new(30)]));
    }

    #[test]
    fn latency_can_lift_spiky_game_helper() {
        let active_targets = BTreeMap::from([
            (Tid::new(10), task_info(10, TaskClass::GameHelper)),
            (Tid::new(20), task_info(20, TaskClass::Launcher)),
        ]);
        let mut stats = TaskStats::new(10, "helper".to_owned(), 0);
        stats.session_latency.max_ns = 900_000_000;
        let stats_by_task = BTreeMap::from([(Tid::new(10), stats)]);

        let selected = select_target_tids(&active_targets, &stats_by_task, 1);

        assert_eq!(selected, BTreeSet::from([Tid::new(10)]));
    }

    #[test]
    fn cap_limits_selected_tasks_with_tid_tiebreaker() {
        let active_targets = BTreeMap::from([
            (Tid::new(30), task_info(30, TaskClass::Game)),
            (Tid::new(10), task_info(10, TaskClass::Game)),
            (Tid::new(20), task_info(20, TaskClass::Game)),
        ]);

        let selected = select_target_tids(&active_targets, &BTreeMap::new(), 2);

        assert_eq!(selected, BTreeSet::from([Tid::new(10), Tid::new(20)]));
    }
}
