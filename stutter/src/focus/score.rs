use super::{
    classify::PriorityBand,
    group_build::is_stable_build_root,
    groups::{FocusGroup, FocusGroupKind, FocusScoreBreakdown},
    process_scan::{
        contains_game_runtime_text, is_active_foreground_candidate, is_game_runtime_process,
        process_identity_text,
    },
    snapshot::{FocusProcess, FocusSnapshot},
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

pub(crate) fn score_game_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let mut score = 0.0_f32;

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Game)
            .unwrap_or(false)
    }) {
        score += 0.25;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::GameScope)
            .unwrap_or(false)
    }) {
        score += 0.20;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::WineServer)
            .unwrap_or(false)
    }) {
        score += 0.10;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                process.classification.class == SystemTaskClass::GameRenderThread
                    || process.classification.class == SystemTaskClass::GameWorkerThread
            })
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(contains_game_runtime_text)
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                process_identity_text(process).contains("steamapps/common")
                    || process_identity_text(process).contains("steamapps")
            })
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    clamp_score(score)
}

pub(crate) fn score_browser_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let mut score = 0.0_f32;

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::BrowserForeground)
            .unwrap_or(false)
    }) {
        score += 0.30;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::BrowserRenderer)
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::BrowserGpu)
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::BrowserNetwork)
            .unwrap_or(false)
    }) {
        score += 0.10;
    }

    if active_process_count(snapshot, member_pids) >= 2 {
        score += 0.20;
    }

    clamp_score(score)
}

pub(crate) fn score_compile_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let mut score = 0.0_f32;

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(is_stable_build_root)
            .unwrap_or(false)
    }) {
        score += 0.35;
    }

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

    score += (active_compiler_or_linker_count as f32 * 0.12).min(0.35);

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                process.classification.class == SystemTaskClass::Linker
                    && process
                        .read_bytes_delta
                        .saturating_add(process.write_bytes_delta)
                        > 0
            })
            .unwrap_or(false)
    }) {
        score += 0.15;
    }

    if total_cpu_ticks(snapshot, member_pids) >= 100 {
        score += 0.15;
    }

    clamp_score(score)
}

pub(crate) fn score_media_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let has_media = member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Media)
            .unwrap_or(false)
    });

    if has_media {
        clamp_score(0.35 + low_to_moderate_activity_bonus(snapshot, member_pids))
    } else {
        0.0
    }
}

pub(crate) fn score_recording_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let has_recorder = member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Recorder)
            .unwrap_or(false)
    });

    if has_recorder {
        clamp_score(0.40 + low_to_moderate_activity_bonus(snapshot, member_pids))
    } else {
        0.0
    }
}

pub(crate) fn score_virtual_machine_class_evidence(
    snapshot: &FocusSnapshot,
    member_pids: &[u32],
) -> f32 {
    let has_vm = member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::VirtualMachine)
            .unwrap_or(false)
    });

    if has_vm {
        clamp_score(0.45 + focus_group_cpu_score(snapshot, member_pids) * 0.25)
    } else {
        0.0
    }
}

pub(crate) fn score_desktop_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let mut score = 0.0_f32;

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| process.classification.class == SystemTaskClass::Compositor)
            .unwrap_or(false)
    }) {
        score += 0.25;
    }

    if member_pids.iter().any(|pid| {
        snapshot
            .processes
            .get(pid)
            .map(|process| {
                matches!(
                    process.classification.class,
                    SystemTaskClass::AudioRealtime
                        | SystemTaskClass::Input
                        | SystemTaskClass::Editor
                        | SystemTaskClass::Terminal
                        | SystemTaskClass::Shell
                )
            })
            .unwrap_or(false)
    }) {
        score += 0.20;
    }

    clamp_score(score)
}

pub(crate) fn score_idle_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let has_idle_class = member_pids.iter().any(|pid| {
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

    if has_idle_class { 0.05 } else { 0.0 }
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

pub(crate) fn focus_group_kind_for_class(class: SystemTaskClass) -> FocusGroupKind {
    match class {
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::GameScope => FocusGroupKind::Game,

        SystemTaskClass::BrowserForeground
        | SystemTaskClass::BrowserBackground
        | SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::BrowserNetwork => FocusGroupKind::Browser,

        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::Indexer
        | SystemTaskClass::PackageManager => FocusGroupKind::Compile,

        SystemTaskClass::Media => FocusGroupKind::Media,
        SystemTaskClass::Recorder => FocusGroupKind::Recording,
        SystemTaskClass::VirtualMachine => FocusGroupKind::VirtualMachine,

        SystemTaskClass::Compositor | SystemTaskClass::AudioRealtime | SystemTaskClass::Input => {
            FocusGroupKind::Desktop
        }

        SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Unknown => FocusGroupKind::Unknown,

        SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread
        | SystemTaskClass::Service => FocusGroupKind::Idle,
        _ => FocusGroupKind::Unknown,
    }
}

pub(crate) fn process_focus_score(process: &FocusProcess) -> f32 {
    let class_base = match process.classification.class {
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input => 90.0,
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::GameScope => 80.0,
        SystemTaskClass::Compositor | SystemTaskClass::BrowserForeground => 70.0,
        SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::BrowserNetwork
        | SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Media
        | SystemTaskClass::Recorder
        | SystemTaskClass::VirtualMachine => 50.0,
        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::Indexer
        | SystemTaskClass::PackageManager => 35.0,
        SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread
        | SystemTaskClass::Service
        | SystemTaskClass::BrowserBackground => 15.0,
        SystemTaskClass::Unknown => 0.0,
        _ => 10.0,
    };

    let cpu_score = process.cpu_time_ticks_delta as f32;
    let io_score = (process
        .read_bytes_delta
        .saturating_add(process.write_bytes_delta) as f32)
        / 1_048_576.0;
    let ctxt_score = (process
        .voluntary_ctxt_switches_delta
        .saturating_add(process.nonvoluntary_ctxt_switches_delta) as f32)
        * 0.05;

    class_base + process.classification.confidence + cpu_score + io_score + ctxt_score
}

fn low_to_moderate_activity_bonus(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    let cpu_ticks = total_cpu_ticks(snapshot, member_pids);
    if cpu_ticks == 0 {
        0.0
    } else if cpu_ticks <= 150 {
        0.25
    } else {
        0.15
    }
}

fn game_group_penalty(snapshot: &FocusSnapshot, root_pids: &[u32], member_pids: &[u32]) -> f32 {
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

fn browser_group_penalty(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
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

fn compile_group_penalty(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
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

fn idle_group_penalty(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
    if total_cpu_ticks(snapshot, member_pids) == 0 {
        0.20
    } else {
        0.10
    }
}

fn desktop_group_penalty(snapshot: &FocusSnapshot, primary_pid: Option<u32>) -> f32 {
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
