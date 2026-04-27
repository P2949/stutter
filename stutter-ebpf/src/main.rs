#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_get_smp_processor_id, bpf_ktime_get_ns},
    macros::{map, tracepoint},
    maps::{HashMap, RingBuf},
    programs::TracePointContext,
};
use stutter_common::{EVENT_RUNNABLE_LATENCY, SchedulerEvent};

#[map]
static EVENTS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

#[map]
static TARGET_PIDS: HashMap<u32, u8> = HashMap::<u32, u8>::with_max_entries(1024, 0);

#[map]
static WAKEUP_TIMES: HashMap<u32, u64> = HashMap::<u32, u64>::with_max_entries(4096, 0);

#[tracepoint]
pub fn sched_wakeup(ctx: TracePointContext) -> u32 {
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

fn is_target_pid(pid: u32) -> bool {
    unsafe { TARGET_PIDS.get(&pid).is_some() }
}

fn try_sched_wakeup(ctx: TracePointContext) -> Result<u32, u32> {
    // sched_wakeup format:
    // comm[16]    offset 8
    // pid         offset 24
    // prio        offset 28
    // target_cpu  offset 32
    let pid: i32 = unsafe { ctx.read_at(24).map_err(|_| 1u32)? };

    if pid <= 0 {
        return Ok(0);
    }

    let pid = pid as u32;

    if !is_target_pid(pid) {
        return Ok(0);
    }

    let now = unsafe { bpf_ktime_get_ns() };

    WAKEUP_TIMES.insert(&pid, &now, 0).map_err(|_| 1u32)?;

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

    let mut entry = EVENTS.reserve::<SchedulerEvent>(0).ok_or(1u32)?;
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

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
