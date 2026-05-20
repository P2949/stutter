//! Process tree grouping builder helpers.
//!
//! Owns:
//! - Focus group hierarchy construction, compiler-session resolution, and candidate rankings.
//!
//! Does not own:
//! - Individual process activity scoring, telemetry delta snapshots, or local daemon policy configs.

use std::collections::BTreeSet;

use super::{
    groups::{FocusGroup, FocusGroupKind, make_focus_group},
    score::process_focus_score,
    snapshot::{FocusProcess, FocusSnapshot},
    tree_walk::{descendants_of_pid, has_ancestor_in_set},
};
use crate::process_tree::TaskClass as SystemTaskClass;

pub(super) fn build_tree_groups_for_kind<F>(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
    kind: FocusGroupKind,
    predicate: F,
) -> Vec<FocusGroup>
where
    F: Fn(&FocusProcess) -> bool,
{
    let matching_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| predicate(process))
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    let mut roots = matching_pids
        .iter()
        .copied()
        .filter(|pid| !has_ancestor_in_set(snapshot, *pid, &matching_pids))
        .collect::<Vec<_>>();

    if roots.is_empty() {
        roots = matching_pids.iter().copied().collect::<Vec<_>>();
    }

    roots.sort_by(|left, right| compare_process_preference(snapshot, *right, *left));

    let mut used = BTreeSet::new();
    let mut groups = Vec::new();

    for root_pid in roots {
        if used.contains(&root_pid) {
            continue;
        }

        let member_pids = descendants_of_pid(snapshot, root_pid)
            .into_iter()
            .filter(|pid| !claimed_pids.contains(pid))
            .filter(|pid| matching_pids.contains(pid) || *pid == root_pid)
            .collect::<Vec<_>>();

        if member_pids.is_empty() {
            continue;
        }

        let primary_pid = member_pids
            .iter()
            .copied()
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right));

        if let Some(group) = make_focus_group(
            snapshot,
            kind,
            vec![root_pid],
            member_pids.clone(),
            primary_pid,
            vec![format!("{kind:?} group rooted at stable process tree")],
        ) {
            used.extend(member_pids);
            groups.push(group);
        }
    }

    groups
}

pub(super) fn root_pids_from_members(
    snapshot: &FocusSnapshot,
    member_pids: &BTreeSet<u32>,
) -> Vec<u32> {
    member_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| !member_pids.contains(&process.ppid))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>()
}

pub(super) fn compare_process_preference(
    snapshot: &FocusSnapshot,
    left_pid: u32,
    right_pid: u32,
) -> std::cmp::Ordering {
    let left_score = snapshot
        .processes
        .get(&left_pid)
        .map(process_focus_score)
        .unwrap_or_default();
    let right_score = snapshot
        .processes
        .get(&right_pid)
        .map(process_focus_score)
        .unwrap_or_default();

    left_score
        .partial_cmp(&right_score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| left_pid.cmp(&right_pid))
}

pub(super) fn is_stable_build_root(process: &FocusProcess) -> bool {
    let comm = process.comm.to_ascii_lowercase();
    matches!(
        comm.as_str(),
        "cargo" | "ninja" | "make" | "cmake" | "meson" | "scons"
    ) || process.classification.class == SystemTaskClass::BuildJob
}

pub(super) fn stable_build_root_rank(snapshot: &FocusSnapshot, pid: u32) -> u8 {
    snapshot
        .processes
        .get(&pid)
        .map(|process| {
            if is_stable_build_root(process) {
                3
            } else if process.classification.class == SystemTaskClass::Terminal {
                2
            } else if process.classification.class == SystemTaskClass::Shell {
                1
            } else {
                0
            }
        })
        .unwrap_or(0)
}

pub(super) fn nearest_compile_session_root(snapshot: &FocusSnapshot, pid: u32) -> Option<u32> {
    let mut current = pid;
    let mut nearest_shell_or_terminal = None;
    let process_cgroup = snapshot
        .processes
        .get(&pid)
        .and_then(|process| process.cgroup_path.clone());

    while let Some(process) = snapshot.processes.get(&current) {
        if is_stable_build_root(process) {
            return Some(process.pid);
        }

        if matches!(
            process.classification.class,
            SystemTaskClass::Terminal | SystemTaskClass::Shell
        ) {
            nearest_shell_or_terminal = Some(process.pid);
            break; // Stop at the NEAREST shell/terminal
        }

        let parent = process.ppid;
        if parent == current || parent == 0 {
            break;
        }

        current = parent;
    }

    nearest_shell_or_terminal.or_else(|| {
        process_cgroup.and_then(|cgroup| {
            snapshot
                .processes
                .values()
                .filter(|process| process.cgroup_path.as_ref() == Some(&cgroup))
                .filter(|process| {
                    matches!(
                        process.classification.class,
                        SystemTaskClass::Terminal | SystemTaskClass::Shell
                    )
                })
                .max_by(|left, right| {
                    process_focus_score(left)
                        .partial_cmp(&process_focus_score(right))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|process| process.pid)
        })
    })
}
