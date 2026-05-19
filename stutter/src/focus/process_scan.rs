use std::collections::BTreeSet;

use super::{
    classify::{PriorityBand, ProcessIdentity},
    groups::{FocusGroup, FocusGroupKind, SafetyWarning, make_focus_group},
    score::{process_focus_score, total_cpu_ticks},
    snapshot::{FocusProcess, FocusSnapshot},
};
use crate::process_tree::TaskClass as SystemTaskClass;

pub(super) fn is_critical_realtime_process(process: &FocusProcess) -> bool {
    matches!(
        process.classification.class,
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input
    ) || process.classification.priority_band == PriorityBand::CriticalRealtime
}

pub(super) fn is_too_broad_system_service_group(
    group: &FocusGroup,
    snapshot: &FocusSnapshot,
) -> bool {
    if group.kind != FocusGroupKind::Idle {
        return false;
    }

    let root_is_system_service = group.root_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(is_system_service_root)
            .unwrap_or(false)
    });

    let all_members_are_service_like = !group.member_pids.is_empty()
        && group.member_pids.iter().all(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| {
                    matches!(
                        process.classification.class,
                        SystemTaskClass::Service
                            | SystemTaskClass::StorageDaemon
                            | SystemTaskClass::NetworkDaemon
                            | SystemTaskClass::KernelThread
                            | SystemTaskClass::IrqThread
                    )
                })
                .unwrap_or(false)
        });

    root_is_system_service && (group.member_pids.len() >= 4 || all_members_are_service_like)
}

fn is_system_service_root(process: &FocusProcess) -> bool {
    if !matches!(
        process.classification.class,
        SystemTaskClass::Service
            | SystemTaskClass::StorageDaemon
            | SystemTaskClass::NetworkDaemon
            | SystemTaskClass::IrqThread
            | SystemTaskClass::KernelThread
    ) {
        return false;
    }

    let comm = process.comm.to_ascii_lowercase();
    let cmdline = process.cmdline.to_ascii_lowercase();
    comm == "systemd"
        || comm == "dbus-daemon"
        || comm == "networkmanager"
        || comm == "udisksd"
        || comm == "sshd"
        || comm.starts_with("systemd-")
        || cmdline.contains("/lib/systemd/")
        || cmdline.contains("/usr/lib/systemd/")
}

pub(super) fn safety_warning_reason(warning: &SafetyWarning) -> String {
    match warning {
        SafetyWarning::CriticalRealtimePresent { pid, comm } => format!(
            "safety: critical realtime/input process present pid={} comm='{}'; never lower or deprioritize this task",
            pid, comm
        ),
        SafetyWarning::CompositorInFocusGroup { pid, comm } => format!(
            "safety: compositor process present pid={} comm='{}'; compositor is foreground latency context, not disposable background load",
            pid, comm
        ),
        SafetyWarning::UnknownForegroundLike { pid, comm } => format!(
            "safety: unknown active foreground-like process present pid={} comm='{}'; keep it Interactive/Unknown rather than Background",
            pid, comm
        ),
        SafetyWarning::TooBroadSystemServiceGroup { root_pids } => format!(
            "safety: broad service/system tree roots {:?}; do not select this as a mutation target",
            root_pids
        ),
    }
}

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

pub(super) fn contains_game_runtime_text(process: &FocusProcess) -> bool {
    let text = process_identity_text(process);
    text.contains("steamapps")
        || text.contains("pressure-vessel")
        || text.contains("proton")
        || text.contains("wineserver")
}

pub(super) fn is_game_runtime_process(process: &FocusProcess) -> bool {
    let text = process_identity_text(process);
    text.contains("pressure-vessel")
        || text.contains("steam-runtime")
        || text.contains("proton")
        || text.contains("steamapps")
}

pub(super) fn is_game_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::Game
            | SystemTaskClass::GameRenderThread
            | SystemTaskClass::GameWorkerThread
            | SystemTaskClass::WineServer
            | SystemTaskClass::GameScope
    )
}

pub(super) fn is_browser_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserBackground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
    )
}

pub(super) fn is_compile_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::BuildJob
            | SystemTaskClass::Compiler
            | SystemTaskClass::Linker
            | SystemTaskClass::Indexer
            | SystemTaskClass::PackageManager
    )
}

pub(super) fn is_non_service_interactive_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::AudioRealtime
            | SystemTaskClass::Input
            | SystemTaskClass::Game
            | SystemTaskClass::GameRenderThread
            | SystemTaskClass::GameWorkerThread
            | SystemTaskClass::WineServer
            | SystemTaskClass::GameScope
            | SystemTaskClass::Compositor
            | SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
            | SystemTaskClass::Editor
            | SystemTaskClass::Terminal
            | SystemTaskClass::Shell
            | SystemTaskClass::Media
            | SystemTaskClass::Recorder
            | SystemTaskClass::VirtualMachine
    )
}

pub(super) fn is_active_foreground_candidate(process: &FocusProcess) -> bool {
    process.cpu_time_ticks_delta > 0
        || process.read_bytes_delta > 0
        || process.write_bytes_delta > 0
        || process.voluntary_ctxt_switches_delta > 0
        || process.nonvoluntary_ctxt_switches_delta > 0
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

pub(super) fn process_identity_text(process: &FocusProcess) -> String {
    let cgroup_path = process
        .cgroup_path
        .as_ref()
        .map(|path| path.to_string_lossy())
        .unwrap_or_default();

    format!(
        "{} {} {}",
        process.comm.to_ascii_lowercase(),
        process.cmdline.to_ascii_lowercase(),
        cgroup_path.to_ascii_lowercase()
    )
}

pub(super) fn low_to_moderate_activity_bonus(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let cpu_ticks = total_cpu_ticks(snapshot, member_pids);
    if cpu_ticks == 0 {
        0.0
    } else if cpu_ticks <= 150 {
        0.25
    } else {
        0.15
    }
}

pub(super) fn game_group_penalty(
    snapshot: &FocusSnapshot,
    root_pids: &[u32],
    member_pids: &[u32],
) -> f32 {
    let root_is_launcher_only = root_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                let text = process_identity_text(process);
                text.contains("steam")
                    && !text.contains("steamapps")
                    && !text.contains("pressure-vessel")
                    && !text.contains("proton")
            })
            .unwrap_or(false)
    });

    let active_game_child_count = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| {
            process.classification.class == SystemTaskClass::Game
                || process.classification.class == SystemTaskClass::GameRenderThread
                || process.classification.class == SystemTaskClass::GameWorkerThread
        })
        .filter(|process| is_active_foreground_candidate(process))
        .count();

    if root_is_launcher_only && active_game_child_count == 0 {
        0.45
    } else if total_cpu_ticks(snapshot, member_pids) < 5 && active_game_child_count == 0 {
        0.20
    } else {
        0.0
    }
}

pub(super) fn browser_group_penalty(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let idle_renderer_count = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| process.classification.class == SystemTaskClass::BrowserRenderer)
        .filter(|process| !is_active_foreground_candidate(process))
        .count();

    let active_child_count = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| process.classification.class != SystemTaskClass::BrowserForeground)
        .filter(|process| is_active_foreground_candidate(process))
        .count();

    if idle_renderer_count > active_child_count.saturating_mul(2).saturating_add(2) {
        ((idle_renderer_count - active_child_count) as f32 * 0.04).min(0.25)
    } else {
        0.0
    }
}

pub(super) fn compile_group_penalty(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let has_stable_build_root = member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(is_stable_build_root)
            .unwrap_or(false)
    });

    let active_compiler_or_linker_count = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| {
            matches!(
                process.classification.class,
                SystemTaskClass::Compiler | SystemTaskClass::Linker
            ) && is_active_foreground_candidate(process)
        })
        .count();

    let indexer_only = member_pids.iter().all(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Indexer)
            .unwrap_or(false)
    });

    if indexer_only {
        0.55
    } else if !has_stable_build_root && active_compiler_or_linker_count == 0 {
        0.35
    } else {
        0.0
    }
}

pub(super) fn idle_group_penalty(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    if total_cpu_ticks(snapshot, member_pids) == 0 {
        0.20
    } else {
        0.10
    }
}

pub(super) fn desktop_group_penalty(snapshot: &FocusSnapshot, primary_pid: Option<u32>) -> f32 {
    let Some(primary_pid) = primary_pid else {
        return 0.10;
    };

    let Some(primary) = snapshot.processes.get(&primary_pid) else {
        return 0.10;
    };

    if primary.classification.class == SystemTaskClass::Compositor
        && !is_active_foreground_candidate(primary)
    {
        0.20
    } else {
        0.0
    }
}

pub(super) fn display_name_for_group(
    kind: FocusGroupKind,
    primary: Option<&FocusProcess>,
) -> String {
    if let Some(primary) = primary
        && !primary.comm.is_empty()
    {
        return primary.comm.clone();
    }

    match kind {
        FocusGroupKind::Game => "Game".to_owned(),
        FocusGroupKind::Browser => "Browser".to_owned(),
        FocusGroupKind::Compile => "Compile".to_owned(),
        FocusGroupKind::Media => "Media".to_owned(),
        FocusGroupKind::Recording => "Recording".to_owned(),
        FocusGroupKind::VirtualMachine => "VirtualMachine".to_owned(),
        FocusGroupKind::Desktop => "Desktop".to_owned(),
        FocusGroupKind::Idle => "Idle".to_owned(),
        FocusGroupKind::Unknown => "Unknown".to_owned(),
    }
}

#[cfg(test)]
pub(super) fn try_community_rules_classification(
    reasons: &mut Vec<String>,
    identity: &ProcessIdentity<'_>,
    cgroup_path: &str,
) -> Option<(SystemTaskClass, f32)> {
    if let Some(hit) = crate::community_rules::classify_process_identity(
        &crate::community_rules::CommunityProcessIdentity {
            thread_comm: identity.comm,
            process_comm: identity.comm,
            cmdline: identity.cmdline,
            exe_path: identity.exe_path.unwrap_or_default(),
            cgroup_path,
        },
    ) && let Some(class) = system_class_for_community_task_class(hit.class)
    {
        reasons.push(hit.reason);
        return Some((class, hit.confidence));
    }
    None
}

#[cfg(not(test))]
pub(super) fn try_community_rules_classification(
    _reasons: &mut Vec<String>,
    _identity: &ProcessIdentity<'_>,
    _cgroup_path: &str,
) -> Option<(SystemTaskClass, f32)> {
    None
}

#[cfg(test)]
fn system_class_for_community_task_class(
    class: crate::process_tree::TaskClass,
) -> Option<SystemTaskClass> {
    match class {
        crate::process_tree::TaskClass::Game => Some(SystemTaskClass::Game),
        _ => None,
    }
}
