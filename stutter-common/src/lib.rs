#![no_std]

pub const EVENT_RUNNABLE_LATENCY: u32 = 1;
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
/// skipped and counted via `DROP_CPU_ACCOUNTING_UNTRACKED`. Keep this in sync
/// with the eBPF map capacities for `CPU_RUNNABLE_DEPTH` and
/// `TARGET_PENDING_WAKEUPS`.
pub const BPF_MAX_TRACKED_CPUS: u32 = 16_384;

/// Fallback eBPF ring-buffer size baked into the BPF object.
///
/// Userspace normally overrides the `EVENTS` map size before loading based on
/// memlock and available memory. This value is the safe fallback used only if
/// that loader-side sizing path is bypassed.
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
pub const DROP_COUNTERS_MAX: u32 = 9;

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

    /// Raw sched_switch.prev_state for switch_prev_pid.
    /// This describes the task switched out, not the task switched in.
    pub switch_prev_state: i64,
}

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
    pub enter_ns: u64,
    pub exit_ns: u64,
    pub duration_ns: u64,
}

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

#[cfg(feature = "user")]
unsafe impl aya::Pod for CpuFreqEvent {}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct StatWaitEvent {
    pub kind: u32,
    pub tid: u32,
    pub delay_ns: u64,
}

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
    pub context: u64,
    pub seqno: u64,
    pub timeline_hash: u64,
    pub wait_start_ns: u64,
    pub wait_done_ns: u64,
    pub signal_ns: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
}

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
