use super::{
    foreground_match::{
        add_foreground_fallback_group_if_needed, is_foreground_fallback_group,
        process_name_looks_like_xwayland,
    },
    groups::{FocusGroup, FocusScoreBreakdown},
    safety::{is_critical_realtime_process, process_name_looks_like_systemd},
    score::priority_band_rank,
    snapshot::FocusSnapshot,
    tree_walk::same_process_family,
};
use crate::{
    config::FocusSource, foreground::ForegroundProviderStatus,
    process_tree::TaskClass as SystemTaskClass,
};

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

    if foreground.status != ForegroundProviderStatus::Available {
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
