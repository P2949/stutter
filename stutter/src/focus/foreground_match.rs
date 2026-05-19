use std::collections::BTreeSet;

use super::{
    classify::PriorityBand,
    groups::{FocusGroup, FocusGroupKind, FocusScoreBreakdown},
    process_scan::{is_active_foreground_candidate, is_critical_realtime_process},
    score::focus_group_contains_pid,
    snapshot::{FocusProcess, FocusSnapshot},
};
use crate::process_tree::TaskClass as SystemTaskClass;

pub(super) fn process_name_looks_like_systemd(process: &FocusProcess) -> bool {
    let comm = process.comm.to_ascii_lowercase();
    let cmdline = process.cmdline.to_ascii_lowercase();

    comm == "systemd"
        || comm.starts_with("systemd-")
        || cmdline == "systemd"
        || cmdline.contains("/systemd ")
        || cmdline.contains("/systemd\0")
        || cmdline.ends_with("/systemd")
}

pub(super) fn process_name_looks_like_xwayland(process: &FocusProcess) -> bool {
    let comm = process.comm.to_ascii_lowercase();
    let cmdline = process.cmdline.to_ascii_lowercase();

    comm == "xwayland" || comm == "xwayland.bin" || cmdline.contains("xwayland")
}

pub(super) fn is_foreground_fallback_group(group: &FocusGroup) -> bool {
    group
        .reasons
        .iter()
        .any(|reason| reason == "foreground window PID selected but no known focus group matched")
}

pub(super) fn add_foreground_fallback_group_if_needed(snapshot: &mut FocusSnapshot) {
    let Some(foreground) = snapshot.foreground.as_ref() else {
        return;
    };

    let Some(pid) = foreground.pid else {
        return;
    };

    if snapshot
        .groups
        .iter()
        .any(|group| focus_group_contains_pid(group, pid))
    {
        return;
    }

    let Some(process) = snapshot.processes.get(&pid) else {
        return;
    };

    if !foreground_process_is_safe_auto_target(process) {
        return;
    }

    let member_pids = descendants_of_process(snapshot, pid);
    let confidence = (foreground.confidence * 0.75).clamp(0.0, 1.0);
    let display_name = if process.comm.trim().is_empty() {
        format!("foreground:{pid}")
    } else {
        format!("foreground:{}", process.comm)
    };

    snapshot.groups.push(FocusGroup {
        kind: FocusGroupKind::Unknown,
        root_pids: vec![pid],
        member_pids,
        primary_pid: Some(pid),
        display_name,
        score: confidence,
        score_breakdown: FocusScoreBreakdown {
            cpu_score: 0.0,
            io_score: 0.0,
            interactivity_score: 0.0,
            class_priority_score: 0.0,
            stability_score: 0.0,
            foreground_score: 0.0,
            penalty: 0.0,
        },
        confidence,
        priority_band: process.classification.priority_band,
        reasons: vec![
            "foreground window PID selected but no known focus group matched".to_owned(),
            format!("conservative foreground fallback root pid={pid}"),
        ],
    });
}

fn descendants_of_process(snapshot: &FocusSnapshot, root_pid: u32) -> Vec<u32> {
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

fn is_process_ancestor(snapshot: &FocusSnapshot, ancestor_pid: u32, descendant_pid: u32) -> bool {
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

pub(super) fn foreground_process_is_safe_auto_target(process: &FocusProcess) -> bool {
    if process.pid == 1 {
        return false;
    }

    if process_name_looks_like_systemd(process) {
        return false;
    }

    if process.classification.class == SystemTaskClass::Compositor {
        return false;
    }

    if is_critical_realtime_process(process) {
        return false;
    }

    if process_name_looks_like_xwayland(process) {
        return false;
    }

    true
}

pub(super) fn is_unknown_foreground_like(process: &FocusProcess) -> bool {
    process.classification.class == SystemTaskClass::Unknown
        && is_active_foreground_candidate(process)
        && process.classification.priority_band != PriorityBand::Background
}
