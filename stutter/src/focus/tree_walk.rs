//! Process-family tree helpers for walking ancestors and descendants.
//!
//! Owns:
//! - Ancestor/descendant identification, process family grouping, and cgroup matching.
//!
//! Does not own:
//! - Focus scoring policy, group penalties, or foreground-target resolution.

use std::collections::BTreeSet;

use super::{
    process_scan::contains_game_runtime_text,
    snapshot::{FocusProcess, FocusSnapshot},
};

pub(super) fn descendants_of_process(snapshot: &FocusSnapshot, root_pid: u32) -> Vec<u32> {
    let mut descendants = BTreeSet::new();
    let mut stack = snapshot
        .children_by_parent
        .get(&root_pid)
        .cloned()
        .unwrap_or_default();

    while let Some(pid) = stack.pop() {
        if !snapshot.processes.contains_key(&pid) {
            continue;
        }

        if !descendants.insert(pid) {
            continue;
        }

        if let Some(children) = snapshot.children_by_parent.get(&pid) {
            stack.extend(children.iter().copied());
        }
    }

    descendants.into_iter().collect()
}

pub(super) fn same_process_family(snapshot: &FocusSnapshot, left_pid: u32, right_pid: u32) -> bool {
    if left_pid == right_pid {
        return true;
    }

    if is_process_ancestor(snapshot, left_pid, right_pid) {
        return true;
    }

    if is_process_ancestor(snapshot, right_pid, left_pid) {
        return true;
    }

    let left_parent = snapshot
        .processes
        .get(&left_pid)
        .map(|process| process.ppid);
    let right_parent = snapshot
        .processes
        .get(&right_pid)
        .map(|process| process.ppid);

    matches!((left_parent, right_parent), (Some(left), Some(right)) if left != 0 && left == right)
}

pub(super) fn is_process_ancestor(
    snapshot: &FocusSnapshot,
    ancestor_pid: u32,
    descendant_pid: u32,
) -> bool {
    let mut current = descendant_pid;
    let mut seen = BTreeSet::new();

    while seen.insert(current) {
        let Some(process) = snapshot.processes.get(&current) else {
            return false;
        };

        if process.ppid == ancestor_pid {
            return true;
        }

        if process.ppid == 0 || process.ppid == current {
            return false;
        }

        current = process.ppid;
    }

    false
}

pub(super) fn descendants_of_pid(snapshot: &FocusSnapshot, root_pid: u32) -> BTreeSet<u32> {
    let mut result = BTreeSet::new();
    let mut stack = vec![root_pid];

    while let Some(pid) = stack.pop() {
        if !result.insert(pid) {
            continue;
        }

        if let Some(children) = snapshot.children_by_parent.get(&pid) {
            for child in children.iter().rev() {
                stack.push(*child);
            }
        }
    }

    result
}

pub(super) fn has_ancestor_in_set(
    snapshot: &FocusSnapshot,
    pid: u32,
    pids: &BTreeSet<u32>,
) -> bool {
    let mut current = pid;
    let mut seen = BTreeSet::new();

    while let Some(process) = snapshot.processes.get(&current) {
        if !seen.insert(current) {
            return false;
        }

        let parent = process.ppid;
        if parent == current || parent == 0 {
            return false;
        }

        if pids.contains(&parent) {
            return true;
        }

        current = parent;
    }

    false
}

pub(super) fn process_appears_tied_to_root(
    snapshot: &FocusSnapshot,
    pid: u32,
    root_pid: u32,
) -> bool {
    if pid == root_pid {
        return true;
    }

    let Some(process) = snapshot.processes.get(&pid) else {
        return false;
    };
    let Some(root) = snapshot.processes.get(&root_pid) else {
        return false;
    };

    descendants_of_pid(snapshot, root_pid).contains(&pid)
        || process.ppid == root.ppid
        || same_non_empty_cgroup(process, root)
        || (contains_game_runtime_text(process) && contains_game_runtime_text(root))
}

fn same_non_empty_cgroup(left: &FocusProcess, right: &FocusProcess) -> bool {
    match (&left.cgroup_path, &right.cgroup_path) {
        (Some(left), Some(right)) => {
            !left.as_os_str().is_empty() && !right.as_os_str().is_empty() && left == right
        }
        _ => false,
    }
}
