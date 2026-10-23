//! Privilege hygiene for children. Two shapes: a *profiling* child (campaign UID, a fixed
//! minimal capability set) and a *workload* child (campaign UID, no capabilities at all). The
//! drop is explicit rather than relying on implicit exec capability transitions.
use std::{os::unix::process::CommandExt, process::Command};

use crate::sys::{self, CapSets};

pub const CAP_DAC_READ_SEARCH: u32 = 2;
pub const CAP_SETPCAP: u32 = 8;
pub const CAP_SYS_NICE: u32 = 23;
pub const CAP_SYS_RESOURCE: u32 = 24;
pub const CAP_PERFMON: u32 = 38;
pub const CAP_BPF: u32 = 39;
/// Highest capability number this runner reasons about (covers every kernel in use).
const LAST_CAPABILITY: u32 = 63;

/// A set of capabilities as a bitmask.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet(pub u64);

impl CapabilitySet {
    pub fn of(capabilities: &[u32]) -> Self {
        Self(
            capabilities
                .iter()
                .fold(0, |mask, cap| mask | (1_u64 << cap)),
        )
    }
    pub fn contains(self, capability: u32) -> bool {
        self.0 & (1_u64 << capability) != 0
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The capabilities a profiling suite needs: BPF, perf and rlimit overrides; contention-
/// generating suites additionally need scheduler priority.
pub fn profiling_capabilities(needs_scheduler_priority: bool) -> CapabilitySet {
    // CAP_DAC_READ_SEARCH only reads: tracefs (tracepoint formats) is root-only on most hosts.
    let mut caps = vec![CAP_BPF, CAP_PERFMON, CAP_SYS_RESOURCE, CAP_DAC_READ_SEARCH];
    if needs_scheduler_priority {
        caps.push(CAP_SYS_NICE);
    }
    CapabilitySet::of(&caps)
}

/// Identity to assume before exec; `None` keeps the current uid/gid (non-root callers).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
}

/// Configures `command` so the child runs with exactly `capabilities` (possibly none) and
/// `no_new_privs`, as `identity` when given. Runs between fork and exec.
pub fn restrict_child(
    command: &mut Command,
    identity: Option<Identity>,
    capabilities: CapabilitySet,
) {
    // SAFETY: the closure only performs raw syscalls through `sys` (async-signal-safe, no
    // allocation, no locks), which is what `pre_exec` requires after fork.
    unsafe {
        command.pre_exec(move || apply_to_current_process(identity, capabilities));
    }
}

/// A child with no capabilities whatsoever (external workloads, game processes, fio, ...).
pub fn drop_all_privileges(command: &mut Command, identity: Option<Identity>) {
    restrict_child(command, identity, CapabilitySet::default());
}

/// The sequence applied in the child. Order matters: the bounding set is trimmed while the
/// process still holds CAP_SETPCAP, ids change with KEEPCAPS so the wanted capabilities
/// survive, then the sets and ambient capabilities are set exactly.
fn apply_to_current_process(
    identity: Option<Identity>,
    capabilities: CapabilitySet,
) -> std::io::Result<()> {
    let privileged = sys::effective_uid() == 0;
    if privileged {
        for capability in 0..=LAST_CAPABILITY {
            if !capabilities.contains(capability) {
                // EINVAL for capabilities this kernel does not have is expected and harmless.
                let _ = sys::drop_bounding(capability);
            }
        }
    }
    if let Some(identity) = identity {
        sys::keep_caps_across_setuid(true)?;
        sys::become_user(identity.uid, identity.gid)?;
    }
    sys::set_capabilities(CapSets {
        effective: capabilities.0,
        permitted: capabilities.0,
        inheritable: capabilities.0,
    })?;
    sys::clear_ambient()?;
    for capability in 0..=LAST_CAPABILITY {
        if capabilities.contains(capability) {
            sys::raise_ambient(capability)?;
        }
    }
    sys::keep_caps_across_setuid(false)?;
    sys::set_no_new_privs()
}
