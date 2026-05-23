use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{
    classify::PriorityBand,
    group_build::{build_tree_groups_for_kind, compare_process_preference},
    group_candidates::{
        build_browser_groups, build_compile_groups, build_desktop_group, build_fallback_group,
        build_game_group, build_idle_group,
    },
    process_scan::display_name_for_group,
    safety::{
        is_critical_realtime_process, is_too_broad_system_service_group,
        is_unknown_foreground_like, safety_warning_reason,
    },
    score::{clamp_score, focus_group_confidence, priority_band_rank, score_focus_group},
    snapshot::FocusSnapshot,
};
use crate::{autotune::state::SituationKind, process_tree::TaskClass as SystemTaskClass};

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
