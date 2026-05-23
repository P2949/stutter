use std::collections::BTreeSet;

use super::{
    group_build::{
        compare_process_preference, is_stable_build_root, nearest_compile_session_root,
        root_pids_from_members, stable_build_root_rank,
    },
    groups::{FocusGroup, FocusGroupKind, make_focus_group},
    process_scan::{
        is_active_foreground_candidate, is_browser_class, is_compile_class, is_game_class,
        is_game_runtime_process, is_non_service_interactive_class,
    },
    score::{focus_group_kind_for_class, process_focus_score},
    snapshot::FocusSnapshot,
    tree_walk::{descendants_of_pid, has_ancestor_in_set, process_appears_tied_to_root},
};
use crate::process_tree::TaskClass as SystemTaskClass;

pub(crate) fn build_game_group(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Option<FocusGroup> {
    let game_like_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| {
            is_game_class(process.classification.class) || is_game_runtime_process(process)
        })
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if game_like_pids.is_empty() {
        return None;
    }

    let root_pid = game_like_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| process.classification.class == SystemTaskClass::GameScope)
                .unwrap_or(false)
        })
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        .or_else(|| {
            game_like_pids
                .iter()
                .copied()
                .filter(|pid| {
                    snapshot
                        .processes
                        .get(pid)
                        .map(|process| process.classification.class == SystemTaskClass::Game)
                        .unwrap_or(false)
                })
                .filter(|pid| !has_ancestor_in_set(snapshot, *pid, &game_like_pids))
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })
        .or_else(|| {
            game_like_pids
                .iter()
                .copied()
                .filter(|pid| {
                    snapshot
                        .processes
                        .get(pid)
                        .map(|process| {
                            is_game_runtime_process(process)
                                && descendants_of_pid(snapshot, process.pid).iter().any(
                                    |child_pid| {
                                        snapshot
                                            .processes
                                            .get(child_pid)
                                            .map(|child| {
                                                child.classification.class == SystemTaskClass::Game
                                                    || child.classification.class
                                                        == SystemTaskClass::GameRenderThread
                                                    || child.classification.class
                                                        == SystemTaskClass::GameWorkerThread
                                            })
                                            .unwrap_or(false)
                                    },
                                )
                        })
                        .unwrap_or(false)
                })
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })
        .or_else(|| {
            game_like_pids
                .iter()
                .copied()
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })?;

    let mut member_pids = descendants_of_pid(snapshot, root_pid)
        .into_iter()
        .filter(|pid| !claimed_pids.contains(pid))
        .collect::<BTreeSet<_>>();

    for process in snapshot.processes.values() {
        if claimed_pids.contains(&process.pid) {
            continue;
        }

        if process.classification.class == SystemTaskClass::WineServer
            && process_appears_tied_to_root(snapshot, process.pid, root_pid)
        {
            member_pids.insert(process.pid);
        }
    }

    for pid in &game_like_pids {
        if !claimed_pids.contains(pid) && process_appears_tied_to_root(snapshot, *pid, root_pid) {
            member_pids.insert(*pid);
        }
    }

    make_focus_group(
        snapshot,
        FocusGroupKind::Game,
        vec![root_pid],
        member_pids.into_iter().collect(),
        Some(root_pid),
        vec![
            "game group selected from gamescope/game/runtime roots".to_owned(),
            "wineserver is included only when tied to the same parent/session/cgroup/runtime"
                .to_owned(),
        ],
    )
}

pub(crate) fn build_browser_groups(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Vec<FocusGroup> {
    let browser_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| is_browser_class(process.classification.class))
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    let mut roots = browser_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| {
                    process.classification.class == SystemTaskClass::BrowserForeground
                        || !has_ancestor_in_set(snapshot, *pid, &browser_pids)
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    roots.sort_by(|left, right| compare_process_preference(snapshot, *right, *left));

    let mut used = BTreeSet::new();
    let mut groups = Vec::new();

    for root_pid in roots {
        if used.contains(&root_pid) {
            continue;
        }

        let mut member_pids = descendants_of_pid(snapshot, root_pid)
            .into_iter()
            .filter(|pid| !claimed_pids.contains(pid))
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(|process| is_browser_class(process.classification.class))
                    .unwrap_or(false)
            })
            .collect::<BTreeSet<_>>();

        member_pids.insert(root_pid);

        if member_pids.is_empty() {
            continue;
        }

        let primary_pid = member_pids
            .iter()
            .copied()
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(|process| {
                        process.classification.class == SystemTaskClass::BrowserForeground
                    })
                    .unwrap_or(false)
            })
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
            .or(Some(root_pid));

        if let Some(group) = make_focus_group(
            snapshot,
            FocusGroupKind::Browser,
            vec![root_pid],
            member_pids.iter().copied().collect(),
            primary_pid,
            vec![
                "browser group rooted at browser parent process".to_owned(),
                "renderer/GPU/network descendants are included under the browser parent".to_owned(),
            ],
        ) {
            used.extend(member_pids);
            groups.push(group);
        }
    }

    groups
}

pub(crate) fn build_compile_groups(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Vec<FocusGroup> {
    let compile_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| is_compile_class(process.classification.class))
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if compile_pids.is_empty() {
        return Vec::new();
    }

    let mut roots = compile_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(is_stable_build_root)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    if roots.is_empty() {
        roots = compile_pids
            .iter()
            .copied()
            .filter_map(|pid| nearest_compile_session_root(snapshot, pid))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
    }

    if roots.is_empty() {
        roots = compile_pids
            .iter()
            .copied()
            .filter(|pid| !has_ancestor_in_set(snapshot, *pid, &compile_pids))
            .collect::<Vec<_>>();
    }

    if roots.is_empty() {
        roots = compile_pids.iter().copied().collect::<Vec<_>>();
    }

    roots.sort_by(|left, right| {
        stable_build_root_rank(snapshot, *right)
            .cmp(&stable_build_root_rank(snapshot, *left))
            .then_with(|| compare_process_preference(snapshot, *right, *left))
    });

    let mut used = BTreeSet::new();
    let mut groups = Vec::new();

    for root_pid in roots {
        if used.contains(&root_pid) || claimed_pids.contains(&root_pid) {
            continue;
        }

        let mut member_pids = descendants_of_pid(snapshot, root_pid)
            .into_iter()
            .filter(|pid| !claimed_pids.contains(pid))
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(|process| {
                        is_compile_class(process.classification.class)
                            || process.pid == root_pid
                            || process.classification.class == SystemTaskClass::Terminal
                            || process.classification.class == SystemTaskClass::Shell
                    })
                    .unwrap_or(false)
            })
            .collect::<BTreeSet<_>>();

        if compile_pids.contains(&root_pid) {
            member_pids.insert(root_pid);
        }

        if !member_pids.iter().any(|pid| compile_pids.contains(pid)) {
            continue;
        }

        let primary_pid = member_pids
            .iter()
            .copied()
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(is_stable_build_root)
                    .unwrap_or(false)
            })
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
            .or_else(|| {
                member_pids
                    .iter()
                    .copied()
                    .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
            });

        if let Some(group) = make_focus_group(
            snapshot,
            FocusGroupKind::Compile,
            vec![root_pid],
            member_pids.iter().copied().collect(),
            primary_pid,
            vec![
                "compile group prefers stable build roots such as cargo/ninja/make/cmake/meson"
                    .to_owned(),
                "compiler/linker descendants are grouped under the stable build/session root"
                    .to_owned(),
            ],
        ) {
            used.extend(member_pids);
            groups.push(group);
        }
    }

    groups
}

pub(crate) fn build_desktop_group(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Option<FocusGroup> {
    let desktop_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| {
            focus_group_kind_for_class(process.classification.class) == FocusGroupKind::Desktop
        })
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if desktop_pids.is_empty() {
        return None;
    }

    let primary_pid = desktop_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| process.classification.class == SystemTaskClass::Compositor)
                .unwrap_or(false)
        })
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(is_active_foreground_candidate)
                .unwrap_or(false)
        })
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        .or_else(|| {
            desktop_pids
                .iter()
                .copied()
                .filter(|pid| {
                    snapshot
                        .processes
                        .get(pid)
                        .map(is_active_foreground_candidate)
                        .unwrap_or(false)
                })
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })
        .or_else(|| {
            desktop_pids
                .iter()
                .copied()
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        });

    make_focus_group(
        snapshot,
        FocusGroupKind::Desktop,
        root_pids_from_members(snapshot, &desktop_pids),
        desktop_pids.into_iter().collect(),
        primary_pid,
        vec![
            "desktop group is supporting context".to_owned(),
            "compositor becomes primary only when it is an active foreground-latency candidate"
                .to_owned(),
        ],
    )
}

pub(crate) fn build_idle_group(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Option<FocusGroup> {
    let idle_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| {
            focus_group_kind_for_class(process.classification.class) == FocusGroupKind::Idle
        })
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if idle_pids.is_empty() {
        return None;
    }

    let primary_pid = idle_pids
        .iter()
        .copied()
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right));

    make_focus_group(
        snapshot,
        FocusGroupKind::Idle,
        root_pids_from_members(snapshot, &idle_pids),
        idle_pids.into_iter().collect(),
        primary_pid,
        vec!["idle/background service group is never allowed to outrank active foreground work by base score alone".to_owned()],
    )
}

pub(crate) fn build_fallback_group(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Option<FocusGroup> {
    let root_pid = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| is_non_service_interactive_class(process.classification.class))
        .max_by(|left, right| {
            left.cpu_time_ticks_delta
                .cmp(&right.cpu_time_ticks_delta)
                .then_with(|| {
                    process_focus_score(left)
                        .partial_cmp(&process_focus_score(right))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .map(|process| process.pid)?;

    let member_pids = descendants_of_pid(snapshot, root_pid)
        .into_iter()
        .filter(|pid| !claimed_pids.contains(pid))
        .collect::<Vec<_>>();

    if member_pids.is_empty() {
        return None;
    }

    let primary_pid = member_pids
        .iter()
        .copied()
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right));

    make_focus_group(
        snapshot,
        FocusGroupKind::Unknown,
        vec![root_pid],
        member_pids,
        primary_pid,
        vec![
            "fallback selected highest non-service interactive process tree by recent CPU delta"
                .to_owned(),
        ],
    )
}
