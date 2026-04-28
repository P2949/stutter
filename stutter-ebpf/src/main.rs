#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_get_smp_processor_id, bpf_ktime_get_ns},
    macros::{map, tracepoint},
    maps::{HashMap, RingBuf},
    programs::TracePointContext,
};
use stutter_common::{
    DROP_COUNTERS_MAX, DROP_RINGBUF_RESERVE_FAILED, DROP_WAKEUP_TIMES_INSERT_FAILED,
    EVENT_RUNNABLE_LATENCY, SchedulerEvent,
};

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

#[map]
static TARGET_PIDS: HashMap<u32, u8> = HashMap::<u32, u8>::with_max_entries(1024, 0);

#[map]
static WAKEUP_TIMES: HashMap<u32, u64> = HashMap::<u32, u64>::with_max_entries(16_384, 0);

#[map]
static DROP_COUNTERS: HashMap<u32, u64> = HashMap::<u32, u64>::with_max_entries(DROP_COUNTERS_MAX, 0);

#[tracepoint]
pub fn sched_wakeup(ctx: TracePointContext) -> u32 {
    match try_sched_wakeup(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn sched_wakeup_new(ctx: TracePointContext) -> u32 {
    match try_sched_wakeup(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn sched_switch(ctx: TracePointContext) -> u32 {
    match try_sched_switch(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn sched_process_exit(_ctx: TracePointContext) -> u32 {
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;
    let _ = WAKEUP_TIMES.remove(&tid);
    0
}

fn is_target_pid(pid: u32) -> bool {
    unsafe { TARGET_PIDS.get(&pid).is_some() }
}

fn increment_drop_counter(reason: u32) {
    let current = unsafe { DROP_COUNTERS.get(&reason).copied().unwrap_or(0) };
    let next = current.saturating_add(1);
    let _ = DROP_COUNTERS.insert(&reason, &next, 0);
}

fn try_sched_wakeup(ctx: TracePointContext) -> Result<u32, u32> {
    let pid: i32 = unsafe { ctx.read_at(24).map_err(|_| 1u32)? };

    if pid <= 0 {
        return Ok(0);
    }

    let pid = pid as u32;

    if !is_target_pid(pid) {
        return Ok(0);
    }

    let now = unsafe { bpf_ktime_get_ns() };

    if WAKEUP_TIMES.insert(&pid, &now, 0).is_err() {
        increment_drop_counter(DROP_WAKEUP_TIMES_INSERT_FAILED);
    }

    Ok(0)
}

fn try_sched_switch(ctx: TracePointContext) -> Result<u32, u32> {
    let next_pid: i32 = unsafe { ctx.read_at(56).map_err(|_| 1u32)? };

    if next_pid <= 0 {
        return Ok(0);
    }

    let pid = next_pid as u32;

    let wakeup_ns = match unsafe { WAKEUP_TIMES.get(&pid) } {
        Some(ts) => *ts,
        None => return Ok(0),
    };

    let _ = WAKEUP_TIMES.remove(&pid);

    if !is_target_pid(pid) {
        return Ok(0);
    }

    let switch_ns = unsafe { bpf_ktime_get_ns() };
    let latency_ns = switch_ns.saturating_sub(wakeup_ns);

    let prio: i32 = unsafe { ctx.read_at(60).map_err(|_| 1u32)? };
    let comm: [u8; 16] = unsafe { ctx.read_at(40).map_err(|_| 1u32)? };
    let cpu = unsafe { bpf_get_smp_processor_id() };

    let Some(mut entry) = EVENTS.reserve::<SchedulerEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return Ok(0);
    };
    let event = entry.as_mut_ptr();

    unsafe {
        (*event).kind = EVENT_RUNNABLE_LATENCY;
        (*event).pid = pid;
        (*event).cpu = cpu;
        (*event).prio = prio;
        (*event).wakeup_ns = wakeup_ns;
        (*event).switch_ns = switch_ns;
        (*event).latency_ns = latency_ns;
        (*event).comm = comm;
    }

    entry.submit(0);

    Ok(0)
}

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";

#[cfg(all(not(test), target_arch = "bpf"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
