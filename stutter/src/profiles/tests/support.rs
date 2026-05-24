use super::super::*;

pub(super) fn test_task(tid: u32, class: TaskClass, comm: &str) -> TaskInfo {
    TaskInfo {
        tid: tid.into(),
        process_pid: tid.into(),
        process_ppid: 1.into(),
        comm: comm.into(),
        process_comm: "process".into(),
        process_starttime_ticks: Some(u64::from(tid) * 10),
        task_starttime_ticks: Some(u64::from(tid) * 10 + 1),
        exe_dev: None,
        exe_ino: None,
        class,
        sched_policy: None,
        from_cgroup: false,
    }
}

pub(super) fn active_task_from_task(
    task: &TaskInfo,
) -> crate::autotune::observation::ActiveTaskSnapshot {
    crate::autotune::observation::ActiveTaskSnapshot {
        tid: task.task_id().as_u32(),
        process_pid: task.process_id().as_u32(),
        comm: task.comm.clone(),
        class: task.class,
        process_starttime_ticks: task.process_starttime_ticks,
        task_starttime_ticks: task.task_starttime_ticks,
        cgroup_path: None,
    }
}
