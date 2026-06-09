//! Safety checking logic and rules for focus target validation.
//!
//! Owns:
//! - Safety validations, broad service group detections, and critical realtime exclusions.
//!
//! Does not own:
//! - Scoring algorithms, foreground history buffers, or active cgroup tracking.

use super::{
    classify::PriorityBand,
    groups::{FocusGroup, FocusGroupKind, SafetyWarning},
    process_scan::is_active_foreground_candidate,
    snapshot::{FocusProcess, FocusSnapshot},
};
use crate::process_tree::TaskClass as SystemTaskClass;

pub(super) fn is_critical_realtime_process(process: &FocusProcess) -> bool {
    matches!(
        process.classification.class,
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input
    ) || process.classification.priority_band == PriorityBand::CriticalRealtime
}

pub(super) fn is_unknown_foreground_like(process: &FocusProcess) -> bool {
    process.classification.class == SystemTaskClass::Unknown
        && is_active_foreground_candidate(process)
        && process.classification.priority_band != PriorityBand::Background
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
