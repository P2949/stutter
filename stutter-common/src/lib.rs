#![no_std]

pub const EVENT_RUNNABLE_LATENCY: u32 = 1;

pub const DROP_WAKEUP_TIMES_INSERT_FAILED: u32 = 0;
pub const DROP_RINGBUF_RESERVE_FAILED: u32 = 1;
pub const DROP_COUNTERS_MAX: u32 = 2;

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
