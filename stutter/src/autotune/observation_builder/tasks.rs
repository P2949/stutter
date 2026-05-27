use std::{fs, path::Path};

use crate::{
    autotune::observation::{
        ActiveTaskSnapshot, ProtectedTask, is_protected_task_class, protected_task_reason,
    },
    process_tree::TaskMap,
};

pub(crate) fn protected_tasks_from_active_tasks(active_tasks: &TaskMap) -> Vec<ProtectedTask> {
    active_tasks
        .values()
        .filter(|task| is_protected_task_class(task.class))
        .map(|task| ProtectedTask {
            tid: task.task_id(),
            process_pid: task.process_id(),
            comm: task.comm.clone(),
            class: task.class,
            reason: protected_task_reason(task.class).to_owned(),
        })
        .collect()
}

pub(crate) fn active_task_snapshots_from_active_tasks(
    proc_root: &Path,
    active_tasks: &TaskMap,
) -> Vec<ActiveTaskSnapshot> {
    active_tasks
        .values()
        .map(|task| ActiveTaskSnapshot {
            tid: task.task_id(),
            process_pid: task.process_id(),
            comm: task.comm.clone(),
            class: task.class,
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            cgroup_path: read_proc_cgroup_path(proc_root, task.task_id().as_u32()),
        })
        .collect()
}
pub(crate) fn read_proc_cgroup_path(proc_root: &Path, pid: u32) -> Option<String> {
    let text = fs::read_to_string(proc_root.join(pid.to_string()).join("cgroup")).ok()?;
    text.lines()
        .filter_map(|line| line.rsplit_once(':').map(|(_, path)| path.trim()))
        .filter(|path| !path.is_empty())
        .max_by_key(|path| path.len())
        .map(str::to_owned)
}
