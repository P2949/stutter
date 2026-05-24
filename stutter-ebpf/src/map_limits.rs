//! eBPF map capacity constants.
//!
//! Keep the capacity rationale outside `main.rs` so the tracepoint entrypoint
//! file stays focused on BPF maps and program bodies. Shared ABI capacities live
//! in `stutter-common`; userspace dynamic sizing clamps live in
//! `stutter/src/ebpf/maps.rs`; the full operator-facing sizing guide lives in
//! `docs/EBPF_CAPACITY.md`.

use stutter_common::BPF_MAX_TRACKED_CPUS;

/// Maximum target tasks accepted by the BPF-side target PID gate.
///
/// This mirrors the userspace target cap. Raising it increases memory in every
/// map whose capacity is expressed as a multiplier of target tasks.
pub(crate) const TARGET_PIDS_MAP_MAX_ENTRIES: u32 = 1_024;

/// Minimum wakeup-state slots reserved per configured target slot in the baked
/// BPF object.
///
/// Wakeup records need much more headroom than TARGET_PIDS because repeated
/// wakeups can replace a target task's pending record before sched_switch
/// consumes it. Keep at least 128x TARGET_PIDS unless the userspace sizing and
/// drop-counter diagnostics are updated with a new capacity model.
pub(crate) const WAKEUP_DATA_PER_TARGET_MULTIPLIER: u32 = 128;

/// Minimum runnable-task CPU bookkeeping slots reserved per configured target.
///
/// Runnable CPU state is per target TID, not per CPU. The 64x multiplier keeps
/// migration/churn bookkeeping from becoming the first bottleneck when many
/// short-lived target threads are seen between userspace refreshes.
pub(crate) const RUNNABLE_TASK_CPU_PER_TARGET_MULTIPLIER: u32 = 64;

/// Minimum previous-fault-counter slots reserved per configured target.
///
/// Fault counters mirror wakeup-map headroom so page-fault deltas are not the
/// first state lost under monitored-task churn.
pub(crate) const PREV_FAULTS_PER_TARGET_MULTIPLIER: u32 = 128;

/// Wakeup-state entries compiled into the BPF object before userspace resizing.
///
/// Memory impact is roughly this entry count times the `WakeupData` payload plus
/// hash-map metadata. The userspace loader may resize the map within documented
/// clamps when memory and memlock budget allow it.
pub(crate) const WAKEUP_DATA_MAP_MAX_ENTRIES: u32 =
    TARGET_PIDS_MAP_MAX_ENTRIES * WAKEUP_DATA_PER_TARGET_MULTIPLIER;

/// Runnable CPU-state entries compiled into the BPF object.
///
/// This is an LRU map keyed by TID. It should scale with monitored-task churn,
/// not CPU count.
pub(crate) const RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES: u32 =
    TARGET_PIDS_MAP_MAX_ENTRIES * RUNNABLE_TASK_CPU_PER_TARGET_MULTIPLIER;

/// Previous page-fault counter entries compiled into the BPF object.
///
/// This mirrors wakeup headroom so fault-delta tracking does not disappear first
/// during high task churn.
pub(crate) const PREV_FAULTS_MAP_MAX_ENTRIES: u32 =
    TARGET_PIDS_MAP_MAX_ENTRIES * PREV_FAULTS_PER_TARGET_MULTIPLIER;

/// Maximum cgroup ids accepted by the experimental native cgroup gate.
///
/// Native cgroup filtering is refused at runtime until cgroup-id resolution is
/// verified, so this remains a small diagnostic/future-use capacity rather than
/// a public tuning knob.
pub(crate) const TARGET_CGROUP_IDS_MAP_MAX_ENTRIES: u32 = 64;

/// Maximum IRQ numbers that can be allowlisted in the eBPF target IRQ map.
pub(crate) const TARGET_IRQS_MAP_MAX_ENTRIES: u32 = 64;

/// Number of in-flight IRQ handler entries tracked by IRQ/cpu key.
pub(crate) const IRQ_START_TIMES_MAP_MAX_ENTRIES: u32 = 1_024;

/// Number of in-flight block request starts retained for issue/complete matching.
pub(crate) const BLOCK_START_MAP_MAX_ENTRIES: u32 = 16_384;

/// Number of in-flight KMS flip request records retained for request/done matching.
pub(crate) const KMS_FLIP_STARTS_MAP_MAX_ENTRIES: u32 = 4_096;

/// Number of in-flight DRM fence wait-start records retained for wait interval matching.
pub(crate) const FENCE_WAIT_STARTS_MAP_MAX_ENTRIES: u32 = 4_096;

/// Number of recent DRM fence signal records retained for wait/signal correlation.
pub(crate) const FENCE_SIGNAL_TIMES_MAP_MAX_ENTRIES: u32 = 4_096;

const _: () = assert!(WAKEUP_DATA_MAP_MAX_ENTRIES >= TARGET_PIDS_MAP_MAX_ENTRIES * 128);
const _: () = assert!(PREV_FAULTS_MAP_MAX_ENTRIES >= TARGET_PIDS_MAP_MAX_ENTRIES * 128);
const _: () = assert!(RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES >= TARGET_PIDS_MAP_MAX_ENTRIES * 64);
const _: () = assert!(BPF_MAX_TRACKED_CPUS >= 1024);
