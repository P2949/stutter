use super::{
    super::{
        group_build::is_stable_build_root,
        process_scan::{is_active_foreground_candidate, process_identity_text},
        snapshot::FocusSnapshot,
    },
    total_cpu_ticks,
};
use crate::process_tree::TaskClass as SystemTaskClass;

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
