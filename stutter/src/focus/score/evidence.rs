use super::{
    super::{
        group_build::is_stable_build_root,
        process_scan::{
            contains_game_runtime_text, is_active_foreground_candidate, process_identity_text,
        },
        snapshot::FocusSnapshot,
    },
    active_process_count, clamp_score, focus_group_cpu_score, total_cpu_ticks,
};
use crate::process_tree::TaskClass as SystemTaskClass;

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

pub(super) fn score_game_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
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

pub(super) fn score_browser_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
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

pub(super) fn score_compile_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
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

pub(super) fn score_media_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
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

pub(super) fn score_recording_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
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

pub(super) fn score_virtual_machine_class_evidence(
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

pub(super) fn score_desktop_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
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

pub(super) fn score_idle_class_evidence(snapshot: &FocusSnapshot, member_pids: &[u32]) -> f32 {
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
