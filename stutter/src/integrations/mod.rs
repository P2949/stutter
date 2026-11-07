//! Read-only probes of other tuning owners on the host (package E).
//!
//! These report who actually holds a resource, from kernel state and process identity, and never
//! write. An owner that is present or uncertain makes the matching resource domain *externally
//! owned*, and the autotuner then denies instead of racing or re-applying user settings.
use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

/// Most `/proc` entries examined when looking for a daemon; the scan is bounded.
const MAX_PROC_ENTRIES: usize = 16_384;

/// A resource family another controller may own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceDomain {
    /// The CPU scheduler itself (sched_ext).
    Scheduler,
    /// GPU electrical and performance profile.
    GpuPower,
    /// Per-task priority, I/O priority and CPU governor, which GameMode adjusts.
    TaskPriorityAndPower,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedExt {
    /// The kernel has no sched_ext.
    Unsupported,
    /// Supported and no BPF scheduler is loaded.
    Disabled,
    /// A BPF scheduler other than the default one is running.
    Enabled { ops: String },
    /// The state could not be read: treated as owned.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Daemon {
    NotRunning,
    /// The daemon's process exists. Whether it currently acts is not known from here.
    Running,
    /// `/proc` could not be scanned: treated as running.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerReport {
    pub sched_ext: SchedExt,
    /// `scx_loader` or an `scx_*` scheduler binary is running (a hint that sched_ext is managed).
    pub scx_manager: Daemon,
    pub gamemode: Daemon,
    pub lact: Daemon,
}

/// Process names (`comm`, at most 15 bytes) found in `/proc`, for the wanted names only.
fn running(proc_root: &Path, wanted: &[&str]) -> Daemon {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return Daemon::Unknown;
    };
    for entry in entries.flatten().take(MAX_PROC_ENTRIES) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        if let Ok(comm) = fs::read_to_string(entry.path().join("comm")) {
            let comm = comm.trim();
            if wanted.iter().any(|want| {
                comm == *want || (want.ends_with('*') && comm.starts_with(&want[..want.len() - 1]))
            }) {
                return Daemon::Running;
            }
        }
    }
    Daemon::NotRunning
}

fn sched_ext(sys_root: &Path) -> SchedExt {
    let base = sys_root.join("kernel/sched_ext");
    if !base.exists() {
        return SchedExt::Unsupported;
    }
    match fs::read_to_string(base.join("state")) {
        Ok(state) if state.trim() == "disabled" => SchedExt::Disabled,
        Ok(state) if state.trim() == "enabled" => match fs::read_to_string(base.join("root/ops")) {
            Ok(ops) if !ops.trim().is_empty() => SchedExt::Enabled {
                ops: ops.trim().to_owned(),
            },
            _ => SchedExt::Unknown,
        },
        _ => SchedExt::Unknown,
    }
}

pub fn probe(proc_root: &Path, sys_root: &Path) -> OwnerReport {
    OwnerReport {
        sched_ext: sched_ext(sys_root),
        scx_manager: running(proc_root, &["scx_loader", "scx_*"]),
        gamemode: running(proc_root, &["gamemoded"]),
        lact: running(proc_root, &["lact", "lactd"]),
    }
}

impl OwnerReport {
    /// Why `domain` is owned by something else, or `None` when it is free. Anything uncertain
    /// counts as owned: ownership fails closed.
    pub fn external_owner(&self, domain: ResourceDomain) -> Option<&'static str> {
        let held = |daemon: Daemon| daemon != Daemon::NotRunning;
        match domain {
            ResourceDomain::Scheduler => match &self.sched_ext {
                SchedExt::Enabled { .. } => Some("a sched_ext scheduler is loaded"),
                SchedExt::Unknown => Some("the sched_ext state is unknown"),
                SchedExt::Unsupported | SchedExt::Disabled if held(self.scx_manager) => {
                    Some("a sched_ext manager is running")
                }
                _ => None,
            },
            ResourceDomain::GpuPower => held(self.lact).then_some("LACT manages the GPU profile"),
            ResourceDomain::TaskPriorityAndPower => {
                held(self.gamemode).then_some("GameMode adjusts task priority and power")
            }
        }
    }
}

#[cfg(test)]
mod tests;
