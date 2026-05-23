//! eBPF map capacity constants.
//!
//! Keep the capacity rationale outside `main.rs` so the tracepoint entrypoint
//! file stays focused on BPF maps and program bodies.

use stutter_common::BPF_MAX_TRACKED_CPUS;

/// Maximum target tasks accepted by the BPF-side target PID gate.
pub(crate) const TARGET_PIDS_MAP_MAX_ENTRIES: u32 = 1_024;

/// Wakeup records need much more headroom than TARGET_PIDS because repeated
/// wakeups can replace a target task's pending record before sched_switch
/// consumes it. Keep at least 128x TARGET_PIDS unless the userspace sizing and
/// drop-counter diagnostics are updated with a new capacity model.
pub(crate) const WAKEUP_DATA_MAP_MAX_ENTRIES: u32 = TARGET_PIDS_MAP_MAX_ENTRIES * 128;

/// Runnable CPU state is per target TID, not per CPU. The 64x multiplier keeps
/// migration/churn bookkeeping from becoming the first bottleneck when many
/// short-lived target threads are seen between userspace refreshes.
pub(crate) const RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES: u32 = TARGET_PIDS_MAP_MAX_ENTRIES * 64;

/// Fault counters mirror wakeup-map headroom so page-fault deltas are not the
/// first state lost under monitored-task churn.
pub(crate) const PREV_FAULTS_MAP_MAX_ENTRIES: u32 = TARGET_PIDS_MAP_MAX_ENTRIES * 128;

const _: () = assert!(WAKEUP_DATA_MAP_MAX_ENTRIES >= TARGET_PIDS_MAP_MAX_ENTRIES * 128);
const _: () = assert!(PREV_FAULTS_MAP_MAX_ENTRIES >= TARGET_PIDS_MAP_MAX_ENTRIES * 128);
const _: () = assert!(RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES >= TARGET_PIDS_MAP_MAX_ENTRIES * 64);
const _: () = assert!(BPF_MAX_TRACKED_CPUS >= 1024);
