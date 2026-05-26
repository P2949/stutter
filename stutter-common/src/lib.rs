#![no_std]

//! Shared userspace/eBPF ABI constants and event records.
//!
//! # eBPF capacity constants
//!
//! The public capacity constants in this crate are part of the userspace/eBPF
//! contract. Keep the values here in sync with the private map capacities in
//! `stutter-ebpf/src/map_limits.rs` and the userspace sizing clamps in
//! `stutter/src/ebpf/maps.rs`.
//!
//! * [`BPF_MAX_TRACKED_CPUS`] controls the length of the eBPF per-CPU
//!   accounting arrays. Raising it increases pinned kernel memory for every
//!   array keyed by CPU id. Lowering it is safe only when every supported CPU id
//!   is still below the new limit; out-of-range CPUs skip runnable-depth and
//!   pending-wakeup accounting and increment
//!   [`DROP_CPU_ACCOUNTING_UNTRACKED`].
//! * [`BPF_DEFAULT_EVENTS_RINGBUF_BYTES`] is only the fallback `EVENTS`
//!   ring-buffer size compiled into the BPF object. Userspace normally replaces
//!   it before load using the automatic map-sizing policy or
//!   `--ringbuf-size-kb`.
//! * [`DROP_COUNTERS_MAX`] is the number of slots in the shared drop-counter
//!   ABI. It must remain greater than the largest `DROP_*` counter index.
//!
//! Detailed sizing rationale, memory impact, and tuning rules live in
//! `docs/EBPF_CAPACITY.md`.

pub const EVENT_RUNNABLE_LATENCY: u32 = 1;
pub mod ebpf_capacity;
pub mod tracepoint_offsets;

pub const EVENT_IRQ_LATENCY: u32 = 2;
pub const EVENT_MIGRATION: u32 = 3;
pub const EVENT_CPU_FREQ: u32 = 4;
pub const EVENT_STAT_WAIT: u32 = 5;
pub const EVENT_BLOCK_IO: u32 = 6;
pub const EVENT_EXEC: u32 = 7;
pub const EVENT_KMS_FLIP: u32 = 8;
pub const EVENT_DRM_FENCE: u32 = 9;

/// Maximum Linux CPU id tracked by the eBPF-side runnable-depth arrays.
///
/// This deliberately covers very large machines while keeping the BPF array
/// maps bounded. Events for CPU ids at or above this value are still safe, but
/// target-local runnable-depth and pending-wakeup accounting is intentionally
/// skipped and counted via [`DROP_CPU_ACCOUNTING_UNTRACKED`]. Keep this in sync
/// with the eBPF map capacities for `CPU_RUNNABLE_DEPTH` and
/// `TARGET_PENDING_WAKEUPS`.
///
/// Memory impact: each additional tracked CPU adds one slot to each CPU-indexed
/// BPF array that uses this limit. The current eBPF object uses it for runnable
/// depth and target-pending-wakeup accounting, so raising it should be treated
/// as a kernel-memory capacity change rather than a cosmetic constant change.
/// Lowering it below `1024` is intentionally rejected by a BPF-side compile-time
/// assertion.
pub const BPF_MAX_TRACKED_CPUS: u32 = 16_384;

/// Fallback eBPF ring-buffer size baked into the BPF object.
///
/// Userspace normally overrides the `EVENTS` map size before loading based on
/// memlock and available memory. This value is the safe fallback used only if
/// that loader-side sizing path is bypassed. It must stay within the userspace
/// clamp documented by `MIN_EVENTS_RINGBUF_BYTES` and
/// `MAX_EVENTS_RINGBUF_BYTES` in `stutter/src/ebpf/maps.rs`.
///
/// Memory impact: ring-buffer bytes are locked kernel memory. Raising this
/// fallback can help only when userspace sizing is unavailable; normal tuning
/// should use `--ringbuf-size-kb` instead.
pub const BPF_DEFAULT_EVENTS_RINGBUF_BYTES: u32 = 256 * 1024;

pub const KMS_FLIP_HAS_REQUEST_NS: u32 = 1 << 0;
pub const KMS_FLIP_HAS_DONE_NS: u32 = 1 << 1;
pub const KMS_FLIP_HAS_DURATION_NS: u32 = 1 << 2;
pub const KMS_FLIP_HAS_SEQUENCE: u32 = 1 << 3;
pub const KMS_FLIP_HAS_CRTC: u32 = 1 << 4;

pub const KMS_FLIP_EVENT_REQUEST: u32 = 1;
pub const KMS_FLIP_EVENT_PAGEFLIP_DONE: u32 = 2;
pub const KMS_FLIP_EVENT_INTERVAL: u32 = 3;
pub const KMS_FLIP_EVENT_VBLANK: u32 = 4;

pub const KMS_FLIP_PROVIDER_DRM: u32 = 1;
pub const KMS_FLIP_PROVIDER_I915: u32 = 2;
pub const KMS_FLIP_PROVIDER_AMDGPU: u32 = 3;

pub const DRM_FENCE_HAS_CONTEXT: u32 = 1 << 0;
pub const DRM_FENCE_HAS_SEQNO: u32 = 1 << 1;
pub const DRM_FENCE_HAS_TIMELINE: u32 = 1 << 2;
pub const DRM_FENCE_HAS_DURATION: u32 = 1 << 3;
pub const DRM_FENCE_HAS_PID: u32 = 1 << 4;
pub const DRM_FENCE_IS_IMPORTER_SIDE: u32 = 1 << 5;
pub const DRM_FENCE_IS_EXPORTER_SIDE: u32 = 1 << 6;
/// Wait-done was observed without a matching wait-start record.
pub const DRM_FENCE_WAIT_DONE_WITHOUT_START: u32 = 1 << 7;

pub const DRM_FENCE_EVENT_WAIT_START: u32 = 1;
pub const DRM_FENCE_EVENT_WAIT_DONE: u32 = 2;
pub const DRM_FENCE_EVENT_SIGNAL: u32 = 3;
pub const DRM_FENCE_EVENT_WAIT_INTERVAL: u32 = 4;

pub const DRM_FENCE_PROVIDER_DMA_FENCE: u32 = 1;
pub const DRM_FENCE_PROVIDER_DRM_SCHED: u32 = 2;
pub const DRM_FENCE_PROVIDER_AMDGPU: u32 = 3;
pub const DRM_FENCE_PROVIDER_I915: u32 = 4;

pub const DRM_GPU_ROLE_UNKNOWN: u32 = 0;
pub const DRM_GPU_ROLE_RENDER: u32 = 1;
pub const DRM_GPU_ROLE_DISPLAY: u32 = 2;

pub const DROP_WAKEUP_DATA_INSERT_FAILED: u32 = 0;
pub const DROP_RINGBUF_RESERVE_FAILED: u32 = 1;
pub const DROP_IRQ_START_TIMES_INSERT_FAILED: u32 = 2;
pub const DROP_BLOCK_START_INSERT_FAILED: u32 = 3;
pub const DROP_WAKEUP_DATA_STALE_ENTRY: u32 = 4;
pub const DROP_BLOCK_FALLBACK_KEY_COLLISION: u32 = 5;
/// A wakeup record for a target TID was replaced before sched_switch consumed it.
pub const DROP_WAKEUP_DATA_REPLACED_ENTRY: u32 = 6;
/// A wakeup record was consumed, but sched_switch field reads failed before emit.
pub const DROP_WAKEUP_DATA_CONSUMED_READ_FAILED: u32 = 7;
/// Runnable-depth or pending-wakeup CPU accounting skipped an out-of-range CPU id.
pub const DROP_CPU_ACCOUNTING_UNTRACKED: u32 = 8;
/// A block I/O request produced key 0, which is reserved as "untrackable".
pub const DROP_BLOCK_ZERO_KEY: u32 = 9;
pub const DROP_DRM_FENCE_MISSING_START: u32 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DropCounterMetadata {
    pub index: u32,
    pub constant_name: &'static str,
    pub field_name: &'static str,
    pub label: &'static str,
}

/// Number of per-CPU drop-counter slots shared by the BPF object and userspace.
///
/// This is a count, not the highest valid index. It must always be greater than
/// every `DROP_*` index above. When adding a new drop reason, add its constant
/// immediately before this one, increment this count, and update
/// [`DROP_COUNTER_METADATA`] so docs and report labels cannot silently drift.
///
/// Memory impact: the BPF `DROP_COUNTERS` map is a per-CPU array, so each extra
/// slot consumes one `u64` per possible CPU plus kernel map metadata. The cost is
/// small compared with the wakeup maps and ring buffer, but the ABI count still
/// needs to stay exact.
pub const DROP_COUNTERS_MAX: u32 = 11;

pub const DROP_COUNTER_METADATA: [DropCounterMetadata; DROP_COUNTERS_MAX as usize] = [
    DropCounterMetadata {
        index: DROP_WAKEUP_DATA_INSERT_FAILED,
        constant_name: "DROP_WAKEUP_DATA_INSERT_FAILED",
        field_name: "wakeup_data_insert_failed",
        label: "wakeup data insert failed",
    },
    DropCounterMetadata {
        index: DROP_RINGBUF_RESERVE_FAILED,
        constant_name: "DROP_RINGBUF_RESERVE_FAILED",
        field_name: "ringbuf_reserve_failed",
        label: "ring buffer reserve failed",
    },
    DropCounterMetadata {
        index: DROP_IRQ_START_TIMES_INSERT_FAILED,
        constant_name: "DROP_IRQ_START_TIMES_INSERT_FAILED",
        field_name: "irq_start_times_insert_failed",
        label: "IRQ start-time insert failed",
    },
    DropCounterMetadata {
        index: DROP_BLOCK_START_INSERT_FAILED,
        constant_name: "DROP_BLOCK_START_INSERT_FAILED",
        field_name: "block_start_insert_failed",
        label: "block I/O start insert failed",
    },
    DropCounterMetadata {
        index: DROP_WAKEUP_DATA_STALE_ENTRY,
        constant_name: "DROP_WAKEUP_DATA_STALE_ENTRY",
        field_name: "wakeup_data_stale_entries",
        label: "stale wakeup data entry",
    },
    DropCounterMetadata {
        index: DROP_BLOCK_FALLBACK_KEY_COLLISION,
        constant_name: "DROP_BLOCK_FALLBACK_KEY_COLLISION",
        field_name: "block_fallback_key_collisions",
        label: "block I/O fallback key collision",
    },
    DropCounterMetadata {
        index: DROP_WAKEUP_DATA_REPLACED_ENTRY,
        constant_name: "DROP_WAKEUP_DATA_REPLACED_ENTRY",
        field_name: "wakeup_data_replaced_entries",
        label: "wakeup data replaced before consumption",
    },
    DropCounterMetadata {
        index: DROP_WAKEUP_DATA_CONSUMED_READ_FAILED,
        constant_name: "DROP_WAKEUP_DATA_CONSUMED_READ_FAILED",
        field_name: "wakeup_data_consumed_read_failed",
        label: "consumed wakeup read failed",
    },
    DropCounterMetadata {
        index: DROP_CPU_ACCOUNTING_UNTRACKED,
        constant_name: "DROP_CPU_ACCOUNTING_UNTRACKED",
        field_name: "cpu_accounting_untracked",
        label: "CPU accounting untracked",
    },
    DropCounterMetadata {
        index: DROP_BLOCK_ZERO_KEY,
        constant_name: "DROP_BLOCK_ZERO_KEY",
        field_name: "block_zero_keys",
        label: "block I/O zero key",
    },
    DropCounterMetadata {
        index: DROP_DRM_FENCE_MISSING_START,
        constant_name: "DROP_DRM_FENCE_MISSING_START",
        field_name: "drm_fence_missing_start",
        label: "DRM fence missing wait start",
    },
];

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SchedulerEvent {
    pub kind: u32,

    /// Linux task/thread id from scheduler tracepoints.
    ///
    /// This is the sched wakee/switch-in task id (sched_switch.next_pid),
    /// not necessarily the process TGID. Userspace should treat this as
    /// a task/TID identity, not a process-level PID.
    pub tid: u32,

    pub cpu: u32,
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub waker_tid: u32,
    /// Diagnostic-only count of monitored pending wakeups for this target/task.
    /// This is not CPU runqueue depth and must not be used as true CPU contention.
    pub target_pending_wakeups: u32,
    /// Approximate per-CPU runnable depth for monitored target tasks only,
    /// reconstructed from sched wakeup/switch/migrate tracepoints.
    /// This is not literal rq->nr_running and does not include unrelated system tasks.
    pub observed_runnable_depth: u32,
    pub maj_flt: u64,
    pub min_flt: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    pub latency_ns: u64,
    pub comm: [u8; 16],
    /// PID of the task switched out immediately before this scheduler event.
    /// This is sched_switch.prev_pid, not necessarily the task that experienced
    /// wakeup latency.
    pub switch_prev_pid: u32,
    pub _pad0: u32,

    /// Raw sched_switch.prev_state for switch_prev_pid.
    /// This describes the task switched out, not the task switched in.
    pub switch_prev_state: i64,
}

// SAFETY: SchedulerEvent is a #[repr(C)] eBPF/userspace ABI record. It contains
// only fixed-width integers and byte arrays, has no references, pointers, Drop
// implementation, or invalid bit patterns, and padding is represented by the
// explicit `_pad0` field initialized by the eBPF producer. The size and critical
// offsets are pinned by compile-time assertions below and must stay in sync with
// eBPF writers.
#[cfg(feature = "user")]
unsafe impl aya::Pod for SchedulerEvent {}

#[cfg(feature = "user")]
impl SchedulerEvent {
    /// Returns the Linux task/thread id (same as `tid`).
    #[inline]
    pub fn task(&self) -> u32 {
        self.tid
    }

    /// Returns the Linux task/thread id (same as `tid`).
    #[inline]
    pub fn process_thread_id(&self) -> u32 {
        self.tid
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IrqEvent {
    pub kind: u32,
    pub irq: u32,
    pub cpu: u32,
    pub _pad0: u32,
    pub enter_ns: u64,
    pub exit_ns: u64,
    pub duration_ns: u64,
}

// SAFETY: IrqEvent is a #[repr(C)] eBPF/userspace ABI record containing only
// fixed-width integer fields. It has no references, pointers, Drop
// implementation, or invalid bit patterns, and its alignment padding is made
// explicit by `_pad0`, which eBPF initializes before emission. Layout is pinned
// by the compile-time assertions below.
#[cfg(feature = "user")]
unsafe impl aya::Pod for IrqEvent {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MigrationEvent {
    pub kind: u32,
    pub tid: u32,
    pub from_cpu: u32,
    pub to_cpu: u32,
    pub timestamp_ns: u64,
}

// SAFETY: MigrationEvent is a #[repr(C)] eBPF/userspace ABI record made only of
// fixed-width integers. It has no references, pointers, Drop implementation, or
// invalid bit patterns, and its size is pinned by a compile-time assertion.
#[cfg(feature = "user")]
unsafe impl aya::Pod for MigrationEvent {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CpuFreqEvent {
    pub kind: u32,
    pub cpu: u32,
    pub state: u32,
    pub _pad: u32,
    pub timestamp_ns: u64,
}

// SAFETY: CpuFreqEvent is a #[repr(C)] eBPF/userspace ABI record containing only
// fixed-width integers. It has no references, pointers, Drop implementation, or
// invalid bit patterns, and padding is represented by the explicit `_pad` field.
// Layout is pinned by a compile-time size assertion.
#[cfg(feature = "user")]
unsafe impl aya::Pod for CpuFreqEvent {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StatWaitEvent {
    pub kind: u32,
    pub tid: u32,
    pub delay_ns: u64,
}

// SAFETY: StatWaitEvent is a #[repr(C)] eBPF/userspace ABI record made only of
// fixed-width integers. It has no references, pointers, Drop implementation, or
// invalid bit patterns, and its size is pinned by a compile-time assertion.
#[cfg(feature = "user")]
unsafe impl aya::Pod for StatWaitEvent {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct BlockIoEvent {
    pub kind: u32,
    pub tid: u32,
    pub dev: u32,
    pub nr_sector: u32,
    pub sector: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
    pub rwbs: [u8; 8],
}

// SAFETY: BlockIoEvent is a #[repr(C)] eBPF/userspace ABI record containing only
// fixed-width integers and a byte array. It has no references, pointers, Drop
// implementation, or invalid bit patterns, and its size is pinned by a
// compile-time assertion.
#[cfg(feature = "user")]
unsafe impl aya::Pod for BlockIoEvent {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ExecEvent {
    pub kind: u32,
    pub pid: u32,
    pub tid: u32,
    pub comm: [u8; 16],
}

// SAFETY: ExecEvent is a #[repr(C)] eBPF/userspace ABI record containing only
// fixed-width integers and a byte array. It has no references, pointers, Drop
// implementation, or invalid bit patterns, and its size is pinned by a
// compile-time assertion.
#[cfg(feature = "user")]
unsafe impl aya::Pod for ExecEvent {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KmsFlipEvent {
    pub kind: u32,
    pub event_kind: u32,
    pub provider: u32,
    pub flags: u32,
    pub pid: u32,
    pub tid: u32,
    pub cpu: u32,
    pub card_minor: u32,
    pub crtc_id: u32,
    pub pipe: u32,
    pub sequence: u64,
    pub request_ns: u64,
    pub done_ns: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
}

// SAFETY: KmsFlipEvent is a #[repr(C)] eBPF/userspace ABI record made only of
// fixed-width integers. It has no references, pointers, Drop implementation, or
// invalid bit patterns, and its size is pinned by a compile-time assertion.
#[cfg(feature = "user")]
unsafe impl aya::Pod for KmsFlipEvent {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct DrmFenceEvent {
    pub kind: u32,
    pub event_kind: u32,
    pub provider: u32,
    pub flags: u32,
    pub pid: u32,
    pub tid: u32,
    pub cpu: u32,
    pub driver_id: u32,
    pub gpu_role: u32,
    pub _pad0: u32,
    pub context: u64,
    pub seqno: u64,
    pub timeline_hash: u64,
    pub wait_start_ns: u64,
    pub wait_done_ns: u64,
    pub signal_ns: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
}

// SAFETY: DrmFenceEvent is a #[repr(C)] eBPF/userspace ABI record containing
// only fixed-width integers. It has no references, pointers, Drop
// implementation, or invalid bit patterns, and padding is represented by the
// explicit `_pad0` field initialized by the eBPF producer. Layout is pinned by
// the compile-time assertions below.
#[cfg(feature = "user")]
unsafe impl aya::Pod for DrmFenceEvent {}

// Compile-time layout assertions to ensure eBPF and userspace agree on struct sizes.
// These will fail the build if the sizes change unexpectedly.
const _: [(); core::mem::size_of::<SchedulerEvent>()] = [(); 104];
const _: [(); core::mem::size_of::<IrqEvent>()] = [(); 40];
const _: [(); core::mem::size_of::<MigrationEvent>()] = [(); 24];
const _: [(); core::mem::size_of::<CpuFreqEvent>()] = [(); 24];
const _: [(); core::mem::size_of::<StatWaitEvent>()] = [(); 16];
const _: [(); core::mem::size_of::<BlockIoEvent>()] = [(); 48];
const _: [(); core::mem::size_of::<ExecEvent>()] = [(); 28];
const _: [(); core::mem::size_of::<KmsFlipEvent>()] = [(); 80];
const _: [(); core::mem::size_of::<DrmFenceEvent>()] = [(); 104];

const _: () = {
    assert!(core::mem::offset_of!(SchedulerEvent, switch_prev_pid) == 88);
    assert!(core::mem::offset_of!(SchedulerEvent, _pad0) == 92);
    assert!(core::mem::offset_of!(SchedulerEvent, switch_prev_state) == 96);
    assert!(core::mem::offset_of!(IrqEvent, cpu) == 8);
    assert!(core::mem::offset_of!(IrqEvent, _pad0) == 12);
    assert!(core::mem::offset_of!(IrqEvent, enter_ns) == 16);
    assert!(core::mem::offset_of!(DrmFenceEvent, gpu_role) == 32);
    assert!(core::mem::offset_of!(DrmFenceEvent, _pad0) == 36);
    assert!(core::mem::offset_of!(DrmFenceEvent, context) == 40);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_counter_metadata_covers_every_slot_with_labels() {
        assert_eq!(
            DROP_COUNTER_METADATA.len(),
            DROP_COUNTERS_MAX as usize,
            "drop counter metadata must cover every shared ABI slot"
        );

        let mut seen = [false; DROP_COUNTERS_MAX as usize];
        for metadata in DROP_COUNTER_METADATA {
            assert!(
                (metadata.index as usize) < seen.len(),
                "{} index {} exceeds DROP_COUNTERS_MAX {}",
                metadata.constant_name,
                metadata.index,
                DROP_COUNTERS_MAX
            );
            assert!(
                !seen[metadata.index as usize],
                "duplicate drop counter metadata index {}",
                metadata.index
            );
            seen[metadata.index as usize] = true;
            assert!(
                !metadata.constant_name.is_empty(),
                "drop counter metadata must name the constant"
            );
            assert!(
                !metadata.field_name.is_empty(),
                "{} must have a serialized field name",
                metadata.constant_name
            );
            assert!(
                !metadata.label.is_empty(),
                "{} must have a human-readable label",
                metadata.constant_name
            );
        }

        assert!(
            seen.iter().all(|slot| *slot),
            "drop counter metadata must not leave ABI slots unlabeled"
        );
    }
}
