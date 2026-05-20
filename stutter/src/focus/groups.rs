use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{
    classify::PriorityBand,
    foreground_match::{
        add_foreground_fallback_group_if_needed, is_foreground_fallback_group,
        process_name_looks_like_xwayland,
    },
    group_build::{
        build_tree_groups_for_kind, compare_process_preference, is_stable_build_root,
        nearest_compile_session_root, root_pids_from_members, stable_build_root_rank,
    },
    process_scan::{
        display_name_for_group, is_active_foreground_candidate, is_browser_class, is_compile_class,
        is_game_class, is_game_runtime_process, is_non_service_interactive_class,
    },
    safety::{
        is_critical_realtime_process, is_too_broad_system_service_group,
        is_unknown_foreground_like, process_name_looks_like_systemd, safety_warning_reason,
    },
    score::*,
    snapshot::FocusSnapshot,
    tree_walk::{
        descendants_of_pid, has_ancestor_in_set, process_appears_tied_to_root, same_process_family,
    },
};
use crate::{
    autotune::state::SituationKind, config::FocusSource, process_tree::TaskClass as SystemTaskClass,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FocusGroupKind {
    Game,
    Browser,
    Compile,
    Media,
    Recording,
    VirtualMachine,
    Desktop,
    Idle,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FocusScoreBreakdown {
    pub cpu_score: f32,
    pub io_score: f32,
    pub interactivity_score: f32,
    pub class_priority_score: f32,
    pub stability_score: f32,
    #[serde(default)]
    pub foreground_score: f32,
    pub penalty: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SafetyWarning {
    CriticalRealtimePresent { pid: u32, comm: String },
    CompositorInFocusGroup { pid: u32, comm: String },
    UnknownForegroundLike { pid: u32, comm: String },
    TooBroadSystemServiceGroup { root_pids: Vec<u32> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusGroup {
    pub kind: FocusGroupKind,
    pub root_pids: Vec<u32>,
    pub member_pids: Vec<u32>,
    pub primary_pid: Option<u32>,
    pub display_name: String,
    pub score: f32,
    pub score_breakdown: FocusScoreBreakdown,
    pub confidence: f32,
    pub priority_band: PriorityBand,
    pub reasons: Vec<String>,
}

pub(crate) fn build_focus_groups(snapshot: &FocusSnapshot) -> Vec<FocusGroup> {
    let mut claimed_pids = BTreeSet::new();
    let mut groups = Vec::new();

    if let Some(group) = build_game_group(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_browser_groups(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_compile_groups(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in
        build_tree_groups_for_kind(snapshot, &claimed_pids, FocusGroupKind::Media, |process| {
            process.classification.class == SystemTaskClass::Media
        })
    {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_tree_groups_for_kind(
        snapshot,
        &claimed_pids,
        FocusGroupKind::Recording,
        |process| process.classification.class == SystemTaskClass::Recorder,
    ) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_tree_groups_for_kind(
        snapshot,
        &claimed_pids,
        FocusGroupKind::VirtualMachine,
        |process| process.classification.class == SystemTaskClass::VirtualMachine,
    ) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    if let Some(group) = build_desktop_group(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    if let Some(group) = build_idle_group(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    if let Some(group) = build_fallback_group(snapshot, &claimed_pids) {
        groups.push(group);
    }

    groups.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                priority_band_rank(right.priority_band).cmp(&priority_band_rank(left.priority_band))
            })
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    groups
}

pub(crate) fn make_focus_group(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    mut root_pids: Vec<u32>,
    mut member_pids: Vec<u32>,
    primary_pid: Option<u32>,
    mut reasons: Vec<String>,
) -> Option<FocusGroup> {
    root_pids.sort_unstable();
    root_pids.dedup();
    member_pids.sort_unstable();
    member_pids.dedup();

    if member_pids.is_empty() {
        return None;
    }

    let primary_pid = primary_pid.or_else(|| {
        member_pids
            .iter()
            .copied()
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
    });

    let score_breakdown = score_focus_group(snapshot, kind, &root_pids, &member_pids, primary_pid);
    let score = clamp_score(
        score_breakdown.cpu_score
            + score_breakdown.io_score
            + score_breakdown.interactivity_score
            + score_breakdown.class_priority_score
            + score_breakdown.stability_score
            - score_breakdown.penalty,
    );

    let confidence = focus_group_confidence(snapshot, &member_pids, &score_breakdown);

    let priority_band = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.classification.priority_band)
        .max_by_key(|band| priority_band_rank(*band))
        .unwrap_or(PriorityBand::Unknown);

    let display_name = display_name_for_group(
        kind,
        primary_pid.and_then(|pid| snapshot.processes.get(&pid)),
    );

    if let Some(primary_pid) = primary_pid
        && let Some(primary) = snapshot.processes.get(&primary_pid)
    {
        reasons.push(format!(
            "primary pid={} comm='{}' class={:?}",
            primary.pid, primary.comm, primary.classification.class
        ));
    }

    let mut group = FocusGroup {
        kind,
        root_pids,
        member_pids,
        primary_pid,
        display_name,
        score,
        score_breakdown,
        confidence,
        priority_band,
        reasons,
    };

    let safety_warnings = safety_warnings_for_group(&group, snapshot);
    group
        .reasons
        .extend(safety_warnings.iter().map(safety_warning_reason));

    Some(group)
}

pub fn safety_warnings_for_group(
    group: &FocusGroup,
    snapshot: &FocusSnapshot,
) -> Vec<SafetyWarning> {
    let mut warnings = Vec::new();

    for pid in &group.member_pids {
        let Some(process) = snapshot.processes.get(pid) else {
            continue;
        };

        if is_critical_realtime_process(process) {
            warnings.push(SafetyWarning::CriticalRealtimePresent {
                pid: process.pid,
                comm: process.comm.clone(),
            });
        }

        if process.classification.class == SystemTaskClass::Compositor {
            warnings.push(SafetyWarning::CompositorInFocusGroup {
                pid: process.pid,
                comm: process.comm.clone(),
            });
        }

        if is_unknown_foreground_like(process) {
            warnings.push(SafetyWarning::UnknownForegroundLike {
                pid: process.pid,
                comm: process.comm.clone(),
            });
        }
    }

    if is_too_broad_system_service_group(group, snapshot) {
        warnings.push(SafetyWarning::TooBroadSystemServiceGroup {
            root_pids: group.root_pids.clone(),
        });
    }

    warnings
}

pub fn situation_for_group(group: &FocusGroup) -> SituationKind {
    match group.kind {
        FocusGroupKind::Game => SituationKind::GameFocused,
        FocusGroupKind::Browser => SituationKind::BrowserFocused,
        FocusGroupKind::Compile => SituationKind::CompileLoad,
        FocusGroupKind::Media => SituationKind::MediaPlayback,
        FocusGroupKind::Recording => SituationKind::Recording,
        FocusGroupKind::VirtualMachine => SituationKind::VirtualMachineLoad,
        FocusGroupKind::Idle => SituationKind::Idle,
        FocusGroupKind::Desktop | FocusGroupKind::Unknown => SituationKind::Unknown,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ForegroundFocusReadiness {
    Ready { pid: u32 },
    NoForeground,
    UnavailableStatus,
    StaleSnapshot,
    MissingPid,
    MissingProcess { pid: u32 },
    UnsafeProcess { pid: u32, reason: &'static str },
}

pub(crate) fn apply_foreground_source_mode_to_snapshot(
    snapshot: &mut FocusSnapshot,
    source_mode: FocusSource,
) {
    if source_mode == FocusSource::Heuristic {
        return;
    }

    match foreground_focus_readiness(snapshot) {
        ForegroundFocusReadiness::Ready { pid } => {
            let _ = pid;
        }
        ForegroundFocusReadiness::UnsafeProcess { pid, reason } => {
            let _ = (pid, reason);
            if source_mode == FocusSource::Foreground {
                snapshot.groups.clear();
            }
            return;
        }
        ForegroundFocusReadiness::MissingProcess { pid } => {
            let _ = pid;
            if source_mode == FocusSource::Foreground {
                snapshot.groups.clear();
            }
            return;
        }
        ForegroundFocusReadiness::MissingPid
        | ForegroundFocusReadiness::StaleSnapshot
        | ForegroundFocusReadiness::UnavailableStatus
        | ForegroundFocusReadiness::NoForeground => {
            if source_mode == FocusSource::Foreground {
                snapshot.groups.clear();
            }
            return;
        }
    }

    add_foreground_fallback_group_if_needed(snapshot);

    let mut groups = std::mem::take(&mut snapshot.groups);

    for group in groups.iter_mut() {
        let foreground_score = foreground_score_for_group(group, snapshot);
        group.score_breakdown.foreground_score = foreground_score;

        if foreground_score > 0.0 {
            group.score = foreground_aware_total_score(&group.score_breakdown);
            if group.confidence < foreground_score && !is_foreground_fallback_group(group) {
                group.confidence = foreground_score;
            }
            group
                .reasons
                .push(format!("foreground-window score {:.2}", foreground_score));
        } else if source_mode == FocusSource::Hybrid {
            group.score = foreground_aware_total_score(&group.score_breakdown);
        }
    }

    if source_mode == FocusSource::Foreground {
        groups.retain(|group| group.score_breakdown.foreground_score > 0.0);
    }

    groups.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                priority_band_rank(right.priority_band).cmp(&priority_band_rank(left.priority_band))
            })
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    snapshot.groups = groups;
}

pub(crate) fn foreground_focus_readiness(snapshot: &FocusSnapshot) -> ForegroundFocusReadiness {
    let Some(foreground) = snapshot.foreground.as_ref() else {
        return ForegroundFocusReadiness::NoForeground;
    };

    if foreground.status != crate::foreground::ForegroundProviderStatus::Available {
        return ForegroundFocusReadiness::UnavailableStatus;
    }

    if foreground.stale_ms.is_some() {
        return ForegroundFocusReadiness::StaleSnapshot;
    }

    if foreground.confidence <= 0.0 {
        return ForegroundFocusReadiness::UnavailableStatus;
    }

    let Some(pid) = foreground.pid else {
        return ForegroundFocusReadiness::MissingPid;
    };

    let Some(process) = snapshot.processes.get(&pid) else {
        return ForegroundFocusReadiness::MissingProcess { pid };
    };

    if process.pid == 1 {
        return ForegroundFocusReadiness::UnsafeProcess {
            pid,
            reason: "pid one",
        };
    }

    if process_name_looks_like_systemd(process) {
        return ForegroundFocusReadiness::UnsafeProcess {
            pid,
            reason: "systemd",
        };
    }

    if process.classification.class == SystemTaskClass::Compositor {
        return ForegroundFocusReadiness::UnsafeProcess {
            pid,
            reason: "compositor",
        };
    }

    if is_critical_realtime_process(process) {
        return ForegroundFocusReadiness::UnsafeProcess {
            pid,
            reason: "critical realtime",
        };
    }

    if process_name_looks_like_xwayland(process) {
        return ForegroundFocusReadiness::UnsafeProcess {
            pid,
            reason: "xwayland",
        };
    }

    ForegroundFocusReadiness::Ready { pid }
}

pub(crate) fn foreground_aware_total_score(breakdown: &FocusScoreBreakdown) -> f32 {
    breakdown.cpu_score * 0.25
        + breakdown.io_score * 0.10
        + breakdown.interactivity_score * 0.15
        + breakdown.class_priority_score * 0.20
        + breakdown.stability_score * 0.10
        + breakdown.foreground_score * 0.35
        - breakdown.penalty
}

pub(crate) fn foreground_score_for_group(group: &FocusGroup, snapshot: &FocusSnapshot) -> f32 {
    let Some(fg) = snapshot.foreground.as_ref() else {
        return 0.0;
    };

    let Some(pid) = fg.pid else {
        return 0.0;
    };

    if group.member_pids.contains(&pid) {
        return 1.0 * fg.confidence;
    }

    if group.root_pids.contains(&pid) {
        return 0.95 * fg.confidence;
    }

    if group
        .member_pids
        .iter()
        .any(|member| same_process_family(snapshot, *member, pid))
    {
        return 0.75 * fg.confidence;
    }

    0.0
}

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
