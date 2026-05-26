use aya_ebpf::helpers::bpf_get_current_cgroup_id;
use stutter_common::BPF_MAX_TRACKED_CPUS;

use crate::maps::{TARGET_CGROUP_IDS, TARGET_PIDS};

#[inline(always)]
pub(crate) fn is_target_current_cgroup() -> bool {
    // Experimental current-task filter. This must not be used for wakee pid
    // fields from scheduler tracepoints.
    let cgroup_id = unsafe { bpf_get_current_cgroup_id() };
    unsafe { TARGET_CGROUP_IDS.get(cgroup_id).is_some() }
}

#[inline(always)]
pub(crate) fn is_target_pid(pid: u32) -> bool {
    // Only consider explicit per-TID entries populated from userspace.
    // Avoid relying on eBPF cgroup heuristics for discovery — userspace
    // periodically refreshes `TARGET_PIDS` from the cgroup tree.
    unsafe { TARGET_PIDS.get(pid).is_some() }
}

#[inline(always)]
pub(crate) fn is_target_pid_or_current_cgroup(pid: u32) -> bool {
    is_target_pid(pid) || is_target_current_cgroup()
}

#[inline(always)]
pub(crate) fn valid_cpu(cpu: u32) -> bool {
    cpu < BPF_MAX_TRACKED_CPUS
}
