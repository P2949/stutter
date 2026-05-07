#[repr(C)]
pub struct SchedulerEvent {
    pub kind: u32,
    pub pid: u32,
    pub cpu: u32,
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub waker_tid: u32,
    pub target_pending_wakeups: u32,
    pub observed_runnable_depth: u32,
    pub maj_flt: u64,
    pub min_flt: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    pub latency_ns: u64,
    pub comm: [u8; 16],
    pub switch_prev_pid: u32,
    pub switch_prev_state: i64,
}

fn main() {
    println!("{}", std::mem::size_of::<SchedulerEvent>());
}
