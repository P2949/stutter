use aya_ebpf::{
    macros::map,
    maps::{Array, HashMap, PerCpuArray, RingBuf},
};
use stutter_common::{BPF_DEFAULT_EVENTS_RINGBUF_BYTES, BPF_MAX_TRACKED_CPUS, DROP_COUNTERS_MAX};

use crate::map_limits::{
    FENCE_SIGNAL_TIMES_MAP_MAX_ENTRIES, FENCE_WAIT_STARTS_MAP_MAX_ENTRIES,
    KMS_FLIP_STARTS_MAP_MAX_ENTRIES, PREV_FAULTS_MAP_MAX_ENTRIES,
    RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES, TARGET_CGROUP_IDS_MAP_MAX_ENTRIES,
    TARGET_PIDS_MAP_MAX_ENTRIES,
};

#[map]
// Userspace overrides this before loading the BPF object based on the current
// memlock limit and available memory. The value here is only a safe fallback.
pub static EVENTS: RingBuf = RingBuf::with_byte_size(BPF_DEFAULT_EVENTS_RINGBUF_BYTES, 0);

#[map]
pub static TARGET_PIDS: HashMap<u32, u8> =
    HashMap::<u32, u8>::with_max_entries(TARGET_PIDS_MAP_MAX_ENTRIES, 0);

#[map]
pub static TARGET_CGROUP_IDS: HashMap<u64, u8> =
    HashMap::<u64, u8>::with_max_entries(TARGET_CGROUP_IDS_MAP_MAX_ENTRIES, 0);

#[map]
// Diagnostic-only count of monitored pending wakeups for this target/task.
// This is not CPU runqueue depth and must not be used as true CPU contention.
pub static TARGET_PENDING_WAKEUPS: Array<u32> =
    Array::<u32>::with_max_entries(BPF_MAX_TRACKED_CPUS, 0);

#[map]
// Approximate per-CPU runnable depth for monitored target tasks only,
// reconstructed from sched wakeup/switch/migrate tracepoints.
// This is not literal rq->nr_running and does not include unrelated system tasks.
pub static CPU_RUNNABLE_DEPTH: Array<u32> = Array::<u32>::with_max_entries(BPF_MAX_TRACKED_CPUS, 0);

#[map]
// Per-target-TID mapping to the CPU where the monitored task was last counted
// as runnable. Used to move monitored runnable counts during migration and
// avoid double-counting duplicate wakeups.
pub static RUNNABLE_TASK_CPU: HashMap<u32, u32> =
    HashMap::<u32, u32>::with_max_entries(RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES, 0);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FaultCounters {
    pub maj: u64,
    pub min: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KmsFlipKey {
    pub provider: u32,
    pub card_minor: u32,
    pub crtc_id: u32,
    pub pipe: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FenceKey {
    pub context: u64,
    pub seqno: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FenceWaitStart {
    pub ts: u64,
    pub pid: u32,
    pub tid: u32,
    pub provider: u32,
    pub gpu_role: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FenceSignal {
    pub ts: u64,
    pub provider: u32,
    pub gpu_role: u32,
}

#[map]
pub static PREV_FAULTS: HashMap<u32, FaultCounters> =
    HashMap::<u32, FaultCounters>::with_max_entries(PREV_FAULTS_MAP_MAX_ENTRIES, 0);

#[map]
pub static KMS_FLIP_STARTS: HashMap<KmsFlipKey, u64> =
    HashMap::<KmsFlipKey, u64>::with_max_entries(KMS_FLIP_STARTS_MAP_MAX_ENTRIES, 0);

#[map]
pub static FENCE_WAIT_STARTS: HashMap<FenceKey, FenceWaitStart> =
    HashMap::<FenceKey, FenceWaitStart>::with_max_entries(FENCE_WAIT_STARTS_MAP_MAX_ENTRIES, 0);

#[map]
pub static FENCE_SIGNAL_TIMES: HashMap<FenceKey, FenceSignal> =
    HashMap::<FenceKey, FenceSignal>::with_max_entries(FENCE_SIGNAL_TIMES_MAP_MAX_ENTRIES, 0);

#[map]
pub static DROP_COUNTERS: PerCpuArray<u64> =
    PerCpuArray::<u64>::with_max_entries(DROP_COUNTERS_MAX, 0);
