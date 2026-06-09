use super::{Profile, ProfileRule, evaluate::task_info_from_active_snapshot};
use crate::process_tree::{self, TaskInfo, TaskMap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuleMatchEvidence {
    pub matched_class: bool,
    pub rule_had_match_class: bool,
    pub rule_had_match_comm: bool,
    pub comm_hits: Vec<CommPatternHit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommPatternHit {
    pub field: CommField,
    pub pattern_index: usize,
    pub pattern: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommField {
    TaskComm,
    ProcessComm,
}

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
    profile_rule_match_evidence(task, rule).is_some()
}

pub(crate) fn profile_rule_match_evidence(
    task: &TaskInfo,
    rule: &ProfileRule,
) -> Option<RuleMatchEvidence> {
    let rule_had_match_class = !rule.match_class.is_empty();
    let rule_had_match_comm = !rule.match_comm.is_empty();
    let matched_class = rule_had_match_class && rule.match_class.contains(&task.class);

    if rule_had_match_class && !matched_class {
        return None;
    }

    let mut comm_hits = Vec::new();
    if rule_had_match_comm {
        for (pattern_index, pattern) in rule.match_comm.iter().enumerate() {
            if pattern.matches(&task.comm) {
                comm_hits.push(CommPatternHit {
                    field: CommField::TaskComm,
                    pattern_index,
                    pattern: pattern.raw().to_owned(),
                    value: task.comm.clone(),
                });
            }
            if pattern.matches(&task.process_comm) {
                comm_hits.push(CommPatternHit {
                    field: CommField::ProcessComm,
                    pattern_index,
                    pattern: pattern.raw().to_owned(),
                    value: task.process_comm.clone(),
                });
            }
        }

        if comm_hits.is_empty() {
            return None;
        }
    }

    Some(RuleMatchEvidence {
        matched_class,
        rule_had_match_class,
        rule_had_match_comm,
        comm_hits,
    })
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
    matching_profile_rule_with_evidence(task, profile).map(|(index, rule, _)| (index, rule))
}

pub(crate) fn matching_profile_rule_with_evidence<'a>(
    task: &TaskInfo,
    profile: &'a Profile,
) -> Option<(usize, &'a ProfileRule, RuleMatchEvidence)> {
    profile.rules.iter().enumerate().find_map(|(index, rule)| {
        profile_rule_match_evidence(task, rule).map(|evidence| (index, rule, evidence))
    })
}
