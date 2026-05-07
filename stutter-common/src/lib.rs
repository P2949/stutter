#![no_std]

pub const EVENT_RUNNABLE_LATENCY: u32 = 1;
pub const EVENT_IRQ_LATENCY: u32 = 2;
pub const EVENT_MIGRATION: u32 = 3;
pub const EVENT_CPU_FREQ: u32 = 4;
pub const EVENT_STAT_WAIT: u32 = 5;
pub const EVENT_BLOCK_IO: u32 = 6;
pub const EVENT_EXEC: u32 = 7;

pub const DROP_WAKEUP_DATA_INSERT_FAILED: u32 = 0;
pub const DROP_RINGBUF_RESERVE_FAILED: u32 = 1;
pub const DROP_IRQ_START_TIMES_INSERT_FAILED: u32 = 2;
pub const DROP_BLOCK_START_INSERT_FAILED: u32 = 3;
pub const DROP_COUNTERS_MAX: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy)]
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
#[derive(Clone, Copy)]
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
#[derive(Clone, Copy)]
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
#[derive(Clone, Copy)]
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
#[derive(Clone, Copy)]
pub struct StatWaitEvent {
    pub kind: u32,
    pub tid: u32,
    pub delay_ns: u64,
}

#[cfg(feature = "user")]
unsafe impl aya::Pod for StatWaitEvent {}

#[repr(C)]
#[derive(Clone, Copy)]
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
#[derive(Clone, Copy)]
pub struct ExecEvent {
    pub kind: u32,
    pub pid: u32,
    pub tid: u32,
    pub comm: [u8; 16],
}

#[cfg(feature = "user")]
unsafe impl aya::Pod for ExecEvent {}

// Compile-time layout assertions to ensure eBPF and userspace agree on struct sizes.
// These will fail the build if the sizes change unexpectedly.
const _: [(); core::mem::size_of::<SchedulerEvent>()] = [(); 104];
const _: [(); core::mem::size_of::<IrqEvent>()] = [(); 40];
const _: [(); core::mem::size_of::<MigrationEvent>()] = [(); 24];
const _: [(); core::mem::size_of::<CpuFreqEvent>()] = [(); 24];
const _: [(); core::mem::size_of::<StatWaitEvent>()] = [(); 16];
const _: [(); core::mem::size_of::<BlockIoEvent>()] = [(); 48];
const _: [(); core::mem::size_of::<ExecEvent>()] = [(); 28];
