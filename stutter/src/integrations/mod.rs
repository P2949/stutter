//! Read-only probes of other tuning owners on the host (packages E and G).
//!
//! These report who actually holds a resource, from kernel state and process identity, and never
//! write. An owner that is present or uncertain makes the matching resource domain *externally
//! owned*, and the autotuner then denies instead of racing or re-applying user settings. Optional
//! tools are never a dependency: absence is simply "not running". Ananicy rules and explicit
//! `nice`/`taskset` settings stay policy hints; this module only arbitrates ownership.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::autotune::operation::MutationOperationKind;

/// Most `/proc` entries examined; the scan is bounded and happens once per probe.
const MAX_PROC_ENTRIES: usize = 16_384;
/// The kernel truncates `comm` to this many bytes.
const COMM_BYTES: usize = 15;

/// A resource family another controller may own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ResourceDomain {
    /// The CPU scheduler itself (sched_ext).
    Scheduler,
    GpuPower,
    /// Per-task nice and I/O priority.
    TaskPriority,
    /// CPU governor and energy-performance preference.
    CpuPower,
    VmKnobs,
    IrqAffinity,
    /// Memory placement and NUMA migration.
    MemoryPlacement,
    Network,
}

/// Tools that can hold resources. Each is detected by process identity, never required.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum KnownOwner {
    GameMode,
    Lact,
    ScxLoader,
    AnanicyCpp,
    System76Scheduler,
    Tuned,
    AutoCpufreq,
    PowerProfilesDaemon,
    Irqbalance,
    Bpftune,
    Numad,
    /// Kills under memory pressure: never tuned, only a guardrail.
    SystemdOomd,
    Uresourced,
    /// Applies power settings from a oneshot unit: visible only through systemd.
    Tlp,
    Thermald,
}

struct OwnerSpec {
    owner: KnownOwner,
    /// Exact process names (matched against the kernel's truncated `comm`), or `prefix*`.
    comms: &'static [&'static str],
    domains: &'static [ResourceDomain],
}

use ResourceDomain as D;

const OWNERS: &[OwnerSpec] = &[
    OwnerSpec {
        owner: KnownOwner::GameMode,
        comms: &["gamemoded"],
        domains: &[D::TaskPriority, D::CpuPower],
    },
    OwnerSpec {
        owner: KnownOwner::Lact,
        comms: &["lact", "lactd"],
        domains: &[D::GpuPower],
    },
    OwnerSpec {
        owner: KnownOwner::ScxLoader,
        comms: &["scx_loader", "scx_*"],
        domains: &[D::Scheduler],
    },
    OwnerSpec {
        owner: KnownOwner::AnanicyCpp,
        comms: &["ananicy-cpp", "ananicy"],
        domains: &[D::TaskPriority],
    },
    OwnerSpec {
        owner: KnownOwner::System76Scheduler,
        comms: &["system76-scheduler"],
        domains: &[D::TaskPriority],
    },
    OwnerSpec {
        owner: KnownOwner::Tuned,
        comms: &["tuned"],
        domains: &[D::CpuPower, D::VmKnobs],
    },
    OwnerSpec {
        owner: KnownOwner::AutoCpufreq,
        comms: &["auto-cpufreq"],
        domains: &[D::CpuPower],
    },
    OwnerSpec {
        owner: KnownOwner::PowerProfilesDaemon,
        comms: &["power-profiles-daemon"],
        domains: &[D::CpuPower],
    },
    OwnerSpec {
        owner: KnownOwner::Irqbalance,
        comms: &["irqbalance"],
        domains: &[D::IrqAffinity],
    },
    OwnerSpec {
        owner: KnownOwner::Bpftune,
        comms: &["bpftune"],
        domains: &[D::Network, D::VmKnobs],
    },
    OwnerSpec {
        owner: KnownOwner::Numad,
        comms: &["numad"],
        domains: &[D::MemoryPlacement],
    },
    OwnerSpec {
        owner: KnownOwner::SystemdOomd,
        comms: &["systemd-oomd"],
        domains: &[],
    },
    OwnerSpec {
        owner: KnownOwner::Uresourced,
        comms: &["uresourced"],
        domains: &[],
    },
    OwnerSpec {
        owner: KnownOwner::Tlp,
        comms: &[],
        domains: &[D::CpuPower],
    },
    OwnerSpec {
        owner: KnownOwner::Thermald,
        comms: &["thermald"],
        domains: &[D::CpuPower],
    },
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedExt {
    /// The kernel has no sched_ext.
    Unsupported,
    /// Supported and no BPF scheduler is loaded.
    Disabled,
    /// A BPF scheduler is running.
    Enabled { ops: String },
    /// The state could not be read: treated as owned.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Daemon {
    NotRunning,
    /// The process runs but the tool's own interface says it is serving nobody right now, so it
    /// claims no domain. Only an exact interface can say this; a process name never can.
    Idle,
    /// The process exists. Whether it currently acts is not known from here.
    Running,
    /// `/proc` could not be scanned: treated as running.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerReport {
    pub sched_ext: SchedExt,
    pub daemons: BTreeMap<KnownOwner, Daemon>,
}

/// NixOS wraps daemons (`.lact-wrapped`, `.gamemoded-wrapped`): the kernel `comm` is the wrapper name.
fn normalize_comm(comm: &str) -> &str {
    let comm = comm.strip_prefix('.').unwrap_or(comm);
    // The 15-byte truncation can cut the suffix short (`-wrap`), so any prefix of it counts.
    (2..=8)
        .rev()
        .find_map(|len| comm.strip_suffix(&"-wrapped"[..len]))
        .unwrap_or(comm)
}

fn comm_matches(comm: &str, pattern: &str) -> bool {
    let comm = normalize_comm(comm);
    match pattern.strip_suffix('*') {
        Some(prefix) => comm.starts_with(prefix),
        None => comm == &pattern[..pattern.len().min(COMM_BYTES)],
    }
}

/// The distinct process names in `/proc`, or `None` when it cannot be read.
fn process_names(proc_root: &Path) -> Option<BTreeSet<String>> {
    let entries = fs::read_dir(proc_root).ok()?;
    let mut names = BTreeSet::new();
    for entry in entries.flatten().take(MAX_PROC_ENTRIES) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.is_empty() || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        if let Ok(comm) = fs::read_to_string(entry.path().join("comm")) {
            names.insert(comm.trim().to_owned());
        }
    }
    Some(names)
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
    let names = process_names(proc_root);
    let daemons = OWNERS
        .iter()
        .map(|spec| {
            let state = match &names {
                None => Daemon::Unknown,
                Some(names) => {
                    if names
                        .iter()
                        .any(|comm| spec.comms.iter().any(|pattern| comm_matches(comm, pattern)))
                    {
                        Daemon::Running
                    } else {
                        Daemon::NotRunning
                    }
                }
            };
            (spec.owner, state)
        })
        .collect();
    OwnerReport {
        sched_ext: sched_ext(sys_root),
        daemons,
    }
}

impl OwnerReport {
    pub fn daemon(&self, owner: KnownOwner) -> Daemon {
        self.daemons.get(&owner).copied().unwrap_or(Daemon::Unknown)
    }

    /// Owners that are running (or unknown) and claim `domain`.
    pub fn owners_of(&self, domain: ResourceDomain) -> Vec<KnownOwner> {
        OWNERS
            .iter()
            .filter(|spec| spec.domains.contains(&domain))
            .filter(|spec| !matches!(self.daemon(spec.owner), Daemon::NotRunning | Daemon::Idle))
            .map(|spec| spec.owner)
            .collect()
    }

    /// Why `domain` is owned by something else, or `None` when it is free. Anything uncertain
    /// counts as owned: ownership fails closed.
    pub fn external_owner(&self, domain: ResourceDomain) -> Option<&'static str> {
        if domain == ResourceDomain::Scheduler {
            match &self.sched_ext {
                SchedExt::Enabled { .. } => return Some("a sched_ext scheduler is loaded"),
                SchedExt::Unknown => return Some("the sched_ext state is unknown"),
                SchedExt::Unsupported | SchedExt::Disabled => {}
            }
        }
        let owner = *self.owners_of(domain).first()?;
        Some(match owner {
            KnownOwner::GameMode => "GameMode adjusts task priority and power",
            KnownOwner::Lact => "LACT manages the GPU profile",
            KnownOwner::ScxLoader => "a sched_ext manager is running",
            KnownOwner::AnanicyCpp => "Ananicy manages task priorities",
            KnownOwner::System76Scheduler => "System76 Scheduler manages task priorities",
            KnownOwner::Tuned => "TuneD manages power and VM settings",
            KnownOwner::AutoCpufreq => "auto-cpufreq manages the CPU governor",
            KnownOwner::PowerProfilesDaemon => "power-profiles-daemon manages CPU power",
            KnownOwner::Irqbalance => "irqbalance manages IRQ affinity",
            KnownOwner::Bpftune => "bpftune manages network and VM settings",
            KnownOwner::Numad => "numad manages memory placement",
            KnownOwner::Tlp => "TLP manages power settings",
            KnownOwner::Thermald => "thermald manages thermal limits",
            KnownOwner::SystemdOomd | KnownOwner::Uresourced => "an out-of-memory guard owns this",
        })
    }
}

/// The resource domain an operation changes, when another controller could own it.
pub fn operation_domain(operation: MutationOperationKind) -> Option<ResourceDomain> {
    use MutationOperationKind as Op;
    match operation {
        Op::Nice | Op::IoPrio => Some(ResourceDomain::TaskPriority),
        Op::CpuPowerEpp | Op::CpuPowerGovernor => Some(ResourceDomain::CpuPower),
        Op::GpuPowerProfile | Op::GpuForcePerformance => Some(ResourceDomain::GpuPower),
        Op::VmSwappiness | Op::VmDirtyBackgroundRatio | Op::VmDirtyRatio => {
            Some(ResourceDomain::VmKnobs)
        }
        Op::IrqKnownDevice | Op::IrqStorageOrUnknown => Some(ResourceDomain::IrqAffinity),
        Op::CpuAffinityProfile | Op::Uclamp | Op::CgroupPlacement | Op::Unclassified => None,
    }
}

/// Why `operation` must be denied because another controller owns its resource. `None` when no
/// owner report exists (not probed) or the domain is free.
pub fn owner_denial(
    owners: Option<&OwnerReport>,
    operation: MutationOperationKind,
) -> Option<&'static str> {
    owners?.external_owner(operation_domain(operation)?)
}

/// Every resource domain, for reporting.
pub const ALL_DOMAINS: [ResourceDomain; 8] = [
    D::Scheduler,
    D::GpuPower,
    D::TaskPriority,
    D::CpuPower,
    D::VmKnobs,
    D::IrqAffinity,
    D::MemoryPlacement,
    D::Network,
];

pub mod receipt;

#[cfg(test)]
mod tests;

/// Every known owner with the domains it claims, in table order, for the adapter reports.
pub(crate) fn owner_domains() -> Vec<(KnownOwner, Vec<ResourceDomain>)> {
    OWNERS
        .iter()
        .map(|spec| (spec.owner, spec.domains.to_vec()))
        .collect()
}

pub mod adapters;
pub mod dbus;
pub mod exact;
pub mod scx_loader;
pub mod units;
pub mod user_intent;

/// The domains some other owner currently claims in `report`.
pub fn claimed_domains(report: &OwnerReport) -> std::collections::BTreeSet<ResourceDomain> {
    ALL_DOMAINS
        .into_iter()
        .filter(|domain| report.external_owner(*domain).is_some())
        .collect()
}

/// Whether `operation` changes a domain that became claimed since `previous`: an owner that
/// appeared (GameMode started serving a game, a manager started) while an experiment runs.
pub fn owner_appeared_for(
    previous: &std::collections::BTreeSet<ResourceDomain>,
    report: &OwnerReport,
    operation: MutationOperationKind,
) -> bool {
    operation_domain(operation).is_some_and(|domain| {
        report.external_owner(domain).is_some() && !previous.contains(&domain)
    })
}
