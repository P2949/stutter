use std::{collections::BTreeMap, fs, os::unix::fs::MetadataExt, path::Path};

use super::*;
use crate::{
    autotune::observation::WorkloadIdentity, focus::FocusGroupKind, process_tree::TaskMap,
};

pub(crate) fn workload_identity_from_runtime_context(
    proc_root: &Path,
    root_pid: Option<u32>,
    focus_kind: Option<FocusGroupKind>,
    active_tasks: &TaskMap,
) -> Option<WorkloadIdentity> {
    let root_pid = root_pid?;
    let root_task = active_tasks
        .values()
        .find(|task| task.process_id() == root_pid || task.task_id() == root_pid);
    let process_starttime_ticks = root_task
        .and_then(|task| task.process_starttime_ticks)
        .or_else(|| crate::process_tree::process_starttime_at(proc_root, root_pid));
    let (exe_dev, exe_ino) = root_task
        .and_then(|task| Some((task.exe_dev?, task.exe_ino?)))
        .map(|(dev, ino)| (Some(dev), Some(ino)))
        .unwrap_or_else(|| exe_identity_for_pid(proc_root, root_pid));
    let cgroup_path = read_proc_cgroup_path(proc_root, root_pid);
    let class_distribution = class_distribution_from_tasks(active_tasks);
    let stable_hash = crate::daemon::state::daemon_profile_stable_hash([
        root_pid.to_string().as_str(),
        process_starttime_ticks
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
            .as_str(),
        exe_dev
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
            .as_str(),
        exe_ino
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
            .as_str(),
        cgroup_path.as_deref().unwrap_or(""),
        &format!("{focus_kind:?}"),
    ]);

    Some(WorkloadIdentity {
        root_pid,
        process_starttime_ticks,
        exe_dev,
        exe_ino,
        cgroup_path,
        focus_kind,
        class_distribution,
        stable_hash,
    })
}

pub(crate) fn exe_identity_for_pid(proc_root: &Path, pid: u32) -> (Option<u64>, Option<u64>) {
    fs::metadata(proc_root.join(pid.to_string()).join("exe"))
        .map(|metadata| (Some(metadata.dev()), Some(metadata.ino())))
        .unwrap_or((None, None))
}
pub(crate) fn class_distribution_from_tasks(active_tasks: &TaskMap) -> BTreeMap<String, usize> {
    let mut classes = BTreeMap::new();
    for task in active_tasks.values() {
        *classes.entry(task.class.as_str().to_owned()).or_insert(0) += 1;
    }
    classes
}
