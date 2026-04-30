#![no_std]

pub const EVENT_RUNNABLE_LATENCY: u32 = 1;
pub const EVENT_IRQ_LATENCY: u32 = 2;

pub const DROP_WAKEUP_TIMES_INSERT_FAILED: u32 = 0;
pub const DROP_RINGBUF_RESERVE_FAILED: u32 = 1;
pub const DROP_IRQ_START_TIMES_INSERT_FAILED: u32 = 2;
pub const DROP_COUNTERS_MAX: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SchedulerEvent {
    pub kind: u32,
    pub pid: u32,
    pub cpu: u32,
    pub prio: i32,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    pub latency_ns: u64,
    pub comm: [u8; 16],
}

#[cfg(feature = "user")]
unsafe impl aya::Pod for SchedulerEvent {}

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

// Compile-time layout assertions to ensure eBPF and userspace agree on struct sizes.
// These will fail the build if the sizes change unexpectedly.
const _: [(); core::mem::size_of::<SchedulerEvent>()] = [(); 56];
const _: [(); core::mem::size_of::<IrqEvent>()] = [(); 40];
