use super::{Profile, ProfileRule, evaluate::task_info_from_active_snapshot};
use crate::process_tree::{self, TaskInfo, TaskMap};

pub fn profile_matched_task_count_from_snapshots(
    tasks: &[crate::autotune::observation::ActiveTaskSnapshot],
    profile: &Profile,
) -> usize {
    tasks
        .iter()
        .filter(|task| {
            let info = task_info_from_active_snapshot(task);
            profile_matches_task(&info, profile)
        })
        .count()
}

pub fn profile_matched_task_count_for_tree(tree_pid: u32, profile: &Profile) -> usize {
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );
    profile_matched_task_count(&snapshot.tasks, profile)
}

pub fn profile_matched_task_count(tasks: &TaskMap, profile: &Profile) -> usize {
    tasks
        .values()
        .filter(|task| profile_matches_task(task, profile))
        .count()
}

pub fn profile_matches_task(task: &TaskInfo, profile: &Profile) -> bool {
    profile
        .rules
        .iter()
        .any(|rule| profile_rule_matches_task(task, rule))
}

pub fn profile_rule_matches_task(task: &TaskInfo, rule: &ProfileRule) -> bool {
    if !rule.match_class.is_empty() && !rule.match_class.contains(&task.class) {
        return false;
    }

    if !rule.match_comm.is_empty() {
        let comms = [&task.comm, task.process_comm.as_str()];
        return rule
            .match_comm
            .iter()
            .any(|pattern| comms.iter().any(|comm| pattern.matches(comm)));
    }

    true
}

pub(super) fn matching_profile_rule<'a>(
    task: &TaskInfo,
    profile: &'a Profile,
) -> Option<&'a ProfileRule> {
    matching_profile_rule_with_index(task, profile).map(|(_, rule)| rule)
}

pub(super) fn matching_profile_rule_with_index<'a>(
    task: &TaskInfo,
    profile: &'a Profile,
) -> Option<(usize, &'a ProfileRule)> {
    for (index, rule) in profile.rules.iter().enumerate() {
        if !rule.match_class.is_empty() && !rule.match_class.contains(&task.class) {
            continue;
        }

        if !rule.match_comm.is_empty() {
            let comms = [&task.comm, task.process_comm.as_str()];
            let mut comm_match = false;

            for pattern in &rule.match_comm {
                if comms.iter().any(|comm| pattern.matches(comm)) {
                    comm_match = true;
                    break;
                }
            }

            if !comm_match {
                continue;
            }
        }

        return Some((index, rule));
    }

    None
}
