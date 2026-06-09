use std::{collections::BTreeMap, path::Path};

pub fn add_watch_tree_pid(tree_pids: &mut Vec<u32>, pid: u32) {
    tree_pids.push(pid);
    tree_pids.sort_unstable();
    tree_pids.dedup();
}

pub fn remove_watch_tree_pid(tree_pids: &mut Vec<u32>, pid: u32) {
    tree_pids.retain(|tree_pid| *tree_pid != pid);
}

pub fn capture_tree_root_starttimes(tree_pids: &[u32]) -> BTreeMap<u32, Option<u64>> {
    tree_pids
        .iter()
        .map(|pid| (*pid, process_root_starttime(*pid)))
        .collect()
}

pub fn process_root_starttime(pid: u32) -> Option<u64> {
    crate::process_tree::process_starttime_at(Path::new("/proc"), pid)
}

pub fn tree_root_is_stale(pid: u32, root_starttimes: &BTreeMap<u32, Option<u64>>) -> bool {
    let current = process_root_starttime(pid);
    let expected = root_starttimes.get(&pid).copied().flatten();

    current.is_none() || expected.is_some_and(|expected| current != Some(expected))
}

pub fn remove_stale_tree_roots(
    tree_pids: &mut Vec<u32>,
    root_starttimes: &mut BTreeMap<u32, Option<u64>>,
    watched_pid: Option<u32>,
) -> Vec<u32> {
    let mut removed = Vec::new();

    for pid in tree_pids.clone() {
        if Some(pid) == watched_pid {
            continue;
        }

        if tree_root_is_stale(pid, root_starttimes) {
            removed.push(pid);
            root_starttimes.remove(&pid);
        }
    }

    if !removed.is_empty() {
        tree_pids.retain(|tree_pid| !removed.contains(tree_pid));
    }

    removed
}
