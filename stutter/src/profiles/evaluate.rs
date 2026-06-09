use stutter_core::ids::{Pid, Tid};

use super::{Profile, matching::matching_profile_rule_with_index};
use crate::process_tree::TaskInfo;

pub struct ProfileEvaluationInput<'a> {
    pub profile: &'a Profile,
    pub active_tasks: &'a [crate::autotune::observation::ActiveTaskSnapshot],
    pub topology: Option<&'a crate::topology::TopologyModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileTaskPlan {
    pub tid: Tid,
    pub process_pid: Pid,
    pub comm: String,
    pub class: crate::process_tree::TaskClass,
    pub requested_mask: String,
    pub matched_rule_index: usize,
    pub matched_rule_name: Option<String>,
}

pub fn evaluate_profile_for_tasks(input: ProfileEvaluationInput<'_>) -> Vec<ProfileTaskPlan> {
    let _topology = input.topology;

    input
        .active_tasks
        .iter()
        .filter_map(|task| {
            let info = task_info_from_active_snapshot(task);
            let (rule_index, rule) = matching_profile_rule_with_index(&info, input.profile)?;
            let requested_mask = rule.affinity.as_ref()?.to_range_string();

            Some(ProfileTaskPlan {
                tid: task.tid,
                process_pid: task.process_pid,
                comm: task.comm.clone(),
                class: task.class,
                requested_mask,
                matched_rule_index: rule_index,
                matched_rule_name: None,
            })
        })
        .collect()
}

pub(super) fn task_info_from_active_snapshot(
    task: &crate::autotune::observation::ActiveTaskSnapshot,
) -> TaskInfo {
    TaskInfo {
        tid: task.tid,
        process_pid: task.process_pid,
        process_ppid: 0.into(),
        comm: task.comm.clone(),
        process_comm: task.comm.clone(),
        process_starttime_ticks: task.process_starttime_ticks,
        task_starttime_ticks: task.task_starttime_ticks,
        exe_dev: None,
        exe_ino: None,
        class: task.class,
        sched_policy: None,
        from_cgroup: task.cgroup_path.is_some(),
    }
}
