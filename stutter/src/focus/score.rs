use super::{
    classify::PriorityBand,
    group_build::is_stable_build_root,
    groups::{FocusGroup, FocusGroupKind, FocusScoreBreakdown},
    process_scan::{is_active_foreground_candidate, is_game_runtime_process},
    snapshot::FocusSnapshot,
};

#[path = "score/evidence.rs"]
mod evidence;
#[path = "score/kinds.rs"]
mod kinds;
#[path = "score/penalties.rs"]
mod penalties;

pub(crate) use self::kinds::{focus_group_kind_for_class, process_focus_score};
use self::{
    evidence::{
        score_browser_class_evidence, score_compile_class_evidence, score_desktop_class_evidence,
        score_game_class_evidence, score_idle_class_evidence, score_media_class_evidence,
        score_recording_class_evidence, score_virtual_machine_class_evidence,
    },
    penalties::{
        browser_group_penalty, compile_group_penalty, desktop_group_penalty, game_group_penalty,
        idle_group_penalty,
    },
};
use crate::process_tree::TaskClass as SystemTaskClass;

pub(crate) fn active_process_count(snapshot: &FocusSnapshot, member_pids: &[u32]) -> usize {
    member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .filter(|process| is_active_foreground_candidate(process))
        .count()
}

pub(crate) fn clamp_score(score: f32) -> f32 {
    score.clamp(0.0, 1.0)
}

pub(crate) fn priority_band_rank(priority_band: PriorityBand) -> u8 {
    match priority_band {
        PriorityBand::Unknown => 0,
        PriorityBand::Background => 1,
        PriorityBand::Throughput => 2,
        PriorityBand::Interactive => 3,
        PriorityBand::ForegroundLatency => 4,
        PriorityBand::CriticalRealtime => 5,
    }
}

pub(crate) fn score_focus_group(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    root_pids: &[u32],
    member_pids: &[u32],
    primary_pid: Option<u32>,
) -> FocusScoreBreakdown {
    let cpu_score = focus_group_cpu_score(snapshot, member_pids);
    let io_score = focus_group_io_score(snapshot, kind, member_pids);
    let interactivity_score = focus_group_interactivity_score(snapshot, member_pids);
    let class_priority_score = focus_group_class_priority_score(snapshot, kind, member_pids);
    let stability_score = focus_group_stability_score(snapshot, kind, root_pids, primary_pid);
    let penalty = focus_group_penalty(snapshot, kind, root_pids, member_pids, primary_pid);

    FocusScoreBreakdown {
        cpu_score,
        io_score,
        interactivity_score,
        class_priority_score,
        stability_score,
        foreground_score: 0.0,
        penalty,
    }
}

pub(crate) fn total_cpu_ticks(snapshot: &FocusSnapshot, member_pids: &[u32]) -> u64 {
    member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.cpu_time_ticks_delta)
        .sum::<u64>()
}

pub(crate) fn focus_group_contains_pid(group: &FocusGroup, pid: u32) -> bool {
    group.root_pids.contains(&pid)
        || group.member_pids.contains(&pid)
        || group.primary_pid == Some(pid)
}

pub(crate) fn focus_group_cpu_score(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let cpu_ticks = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.cpu_time_ticks_delta)
        .sum::<u64>();

    clamp_score(cpu_ticks as f32 / 250.0)
}

pub(crate) fn focus_group_io_score(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    member_pids: &[u32],
) -> f32 {
    let io_bytes = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| {
            process
                .read_bytes_delta
                .saturating_add(process.write_bytes_delta)
        })
        .sum::<u64>();

    let base = clamp_score(io_bytes as f32 / 67_108_864.0);

    if kind == FocusGroupKind::Compile {
        let linker_io_bytes = member_pids
            .iter()
            .filter_map(|pid| snapshot.processes.get(pid))
            .filter(|process| process.classification.class == SystemTaskClass::Linker)
            .map(|process| {
                process
                    .read_bytes_delta
                    .saturating_add(process.write_bytes_delta)
            })
            .sum::<u64>();

        clamp_score(base + (linker_io_bytes as f32 / 33_554_432.0).min(0.20))
    } else {
        base
    }
}

pub(crate) fn focus_group_interactivity_score(
    snapshot: &FocusSnapshot,
    member_pids: &[u32],
) -> f32 {
    let ctxt_switches = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| {
            process
                .voluntary_ctxt_switches_delta
                .saturating_add(process.nonvoluntary_ctxt_switches_delta)
        })
        .sum::<u64>();

    clamp_score(ctxt_switches as f32 / 250.0)
}

pub(crate) fn focus_group_class_priority_score(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    member_pids: &[u32],
) -> f32 {
    match kind {
        FocusGroupKind::Game => score_game_class_evidence(snapshot, member_pids),
        FocusGroupKind::Browser => score_browser_class_evidence(snapshot, member_pids),
        FocusGroupKind::Compile => score_compile_class_evidence(snapshot, member_pids),
        FocusGroupKind::Media => score_media_class_evidence(snapshot, member_pids),
        FocusGroupKind::Recording => score_recording_class_evidence(snapshot, member_pids),
        FocusGroupKind::VirtualMachine => {
            score_virtual_machine_class_evidence(snapshot, member_pids)
        }
        FocusGroupKind::Desktop => score_desktop_class_evidence(snapshot, member_pids),
        FocusGroupKind::Idle => score_idle_class_evidence(snapshot, member_pids),
        FocusGroupKind::Unknown => 0.05,
    }
}

pub(crate) fn focus_group_stability_score(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    root_pids: &[u32],
    primary_pid: Option<u32>,
) -> f32 {
    let mut score = 0.0_f32;

    for root_pid in root_pids {
        if let Some(root) = snapshot.processes.get(root_pid) {
            match kind {
                FocusGroupKind::Game => {
                    if root.classification.class == SystemTaskClass::GameScope
                        || root.classification.class == SystemTaskClass::Game
                        || is_game_runtime_process(root)
                    {
                        score += 0.20;
                    }
                }
                FocusGroupKind::Browser => {
                    if root.classification.class == SystemTaskClass::BrowserForeground {
                        score += 0.25;
                    }
                }
                FocusGroupKind::Compile => {
                    if is_stable_build_root(root) {
                        score += 0.30;
                    } else if matches!(
                        root.classification.class,
                        SystemTaskClass::Terminal | SystemTaskClass::Shell
                    ) {
                        score += 0.15;
                    }
                }
                FocusGroupKind::Media
                | FocusGroupKind::Recording
                | FocusGroupKind::VirtualMachine => {
                    if Some(root.pid) == primary_pid {
                        score += 0.15;
                    }
                }
                FocusGroupKind::Desktop => {
                    if root.classification.class == SystemTaskClass::Compositor {
                        score += 0.10;
                    }
                }
                FocusGroupKind::Idle | FocusGroupKind::Unknown => {}
            }
        }
    }

    clamp_score(score)
}

pub(crate) fn focus_group_penalty(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    root_pids: &[u32],
    member_pids: &[u32],
    primary_pid: Option<u32>,
) -> f32 {
    match kind {
        FocusGroupKind::Game => game_group_penalty(snapshot, root_pids, member_pids),
        FocusGroupKind::Browser => browser_group_penalty(snapshot, member_pids),
        FocusGroupKind::Compile => compile_group_penalty(snapshot, member_pids),
        FocusGroupKind::Idle => idle_group_penalty(snapshot, member_pids),
        FocusGroupKind::Desktop => desktop_group_penalty(snapshot, primary_pid),
        FocusGroupKind::Media
        | FocusGroupKind::Recording
        | FocusGroupKind::VirtualMachine
        | FocusGroupKind::Unknown => 0.0,
    }
}

pub(crate) fn focus_group_confidence(
    snapshot: &FocusSnapshot,
    member_pids: &[u32],
    breakdown: &FocusScoreBreakdown,
) -> f32 {
    let max_class_confidence = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.classification.confidence)
        .fold(0.0_f32, f32::max);

    let activity_evidence =
        clamp_score(breakdown.cpu_score + breakdown.io_score + breakdown.interactivity_score);

    let mut confidence = clamp_score(
        (max_class_confidence * 0.45)
            + (activity_evidence * 0.40)
            + (breakdown.stability_score * 0.25)
            - (breakdown.penalty * 0.50),
    );

    if activity_evidence < 0.05 {
        confidence = confidence.min(0.55);
    } else if activity_evidence < 0.20 {
        confidence = confidence.min(0.75);
    }

    confidence
}
