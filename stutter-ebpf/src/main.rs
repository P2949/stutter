#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{
        bpf_get_current_cgroup_id, bpf_get_current_pid_tgid, bpf_get_smp_processor_id,
        bpf_ktime_get_ns,
    },
    macros::{map, tracepoint},
    maps::{Array, HashMap, LruHashMap, PerCpuArray, RingBuf},
    programs::TracePointContext,
};
use stutter_common::{
    BlockIoEvent, CpuFreqEvent, DROP_BLOCK_START_INSERT_FAILED, DROP_COUNTERS_MAX,
    DROP_IRQ_START_TIMES_INSERT_FAILED, DROP_RINGBUF_RESERVE_FAILED,
    DROP_WAKEUP_DATA_INSERT_FAILED, EVENT_BLOCK_IO, EVENT_CPU_FREQ, EVENT_EXEC, EVENT_IRQ_LATENCY,
    EVENT_MIGRATION, EVENT_RUNNABLE_LATENCY, EVENT_STAT_WAIT, ExecEvent, IrqEvent, MigrationEvent,
    SchedulerEvent, StatWaitEvent,
};

#[unsafe(no_mangle)]
static mut BLOCK_RQ_KEY_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut BLOCK_RQ_ISSUE_NR_SECTOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut BLOCK_RQ_ISSUE_RWBS_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut BLOCK_RQ_COMPLETE_RWBS_OFFSET: u32 = 0;

#[map]
// Userspace overrides this before loading the BPF object based on the current
// memlock limit and available memory. The value here is only a safe fallback.
static EVENTS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

#[map]
static TARGET_PIDS: HashMap<u32, u8> = HashMap::<u32, u8>::with_max_entries(1024, 0);

#[map]
static TARGET_CGROUP_IDS: HashMap<u64, u8> = HashMap::<u64, u8>::with_max_entries(64, 0);

#[map]
static TARGET_IRQS: HashMap<u32, u8> = HashMap::<u32, u8>::with_max_entries(64, 0);

#[repr(C)]
#[derive(Clone, Copy)]
struct WakeupData {
    ts: u64,
    target_cpu: u32,
    waker_tid: u32,
}

#[map]
static WAKEUP_DATA: HashMap<u32, WakeupData> =
    HashMap::<u32, WakeupData>::with_max_entries(131_072, 0);

#[map]
static IRQ_START_TIMES: HashMap<u64, u64> = HashMap::<u64, u64>::with_max_entries(1024, 0);

#[map]
// Diagnostic-only count of monitored pending wakeups for this target/task.
// This is not CPU runqueue depth and must not be used as true CPU contention.
static TARGET_PENDING_WAKEUPS: Array<u32> = Array::<u32>::with_max_entries(1024, 0);

#[map]
// Approximate per-CPU runnable depth for monitored target tasks only,
// reconstructed from sched wakeup/switch/migrate tracepoints.
// This is not literal rq->nr_running and does not include unrelated system tasks.
static CPU_RUNNABLE_DEPTH: Array<u32> = Array::<u32>::with_max_entries(1024, 0);

#[map]
// Per-target-TID mapping to the CPU where the monitored task was last counted
// as runnable. Used to move monitored runnable counts during migration and
// avoid double-counting duplicate wakeups.
static RUNNABLE_TASK_CPU: LruHashMap<u32, u32> = LruHashMap::<u32, u32>::with_max_entries(65536, 0);

#[repr(C)]
#[derive(Clone, Copy)]
struct FaultCounters {
    maj: u64,
    min: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IoStart {
    ts: u64,
    tid: u32,
}

#[map]
static PREV_FAULTS: HashMap<u32, FaultCounters> =
    HashMap::<u32, FaultCounters>::with_max_entries(131_072, 0);

#[map]
static BLOCK_START: LruHashMap<u64, IoStart> =
    LruHashMap::<u64, IoStart>::with_max_entries(16384, 0);

#[map]
static DROP_COUNTERS: PerCpuArray<u64> = PerCpuArray::<u64>::with_max_entries(DROP_COUNTERS_MAX, 0);

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
pub fn sched_migrate_task(ctx: TracePointContext) -> u32 {
    match try_sched_migrate_task(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn sched_process_exec(ctx: TracePointContext) -> u32 {
    match try_sched_process_exec(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

fn try_sched_process_exec(_ctx: TracePointContext) -> Result<u32, u32> {
    let pid_tgid = bpf_get_current_pid_tgid();
    let pid = (pid_tgid >> 32) as u32;
    let tid = (pid_tgid & 0xffff_ffff) as u32;

    if !is_target_pid(pid) && !is_target_pid_or_current_cgroup(tid) {
        return Ok(0);
    }

    if let Some(mut entry) = EVENTS.reserve::<ExecEvent>(0) {
        let p = entry.as_mut_ptr();
        unsafe {
            (*p).kind = EVENT_EXEC;
            (*p).pid = pid;
            (*p).tid = tid;
            if let Ok(comm) = aya_ebpf::helpers::bpf_get_current_comm() {
                (*p).comm = comm;
            }
        }
        entry.submit(0);
    } else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
    }

    Ok(0)
}

#[tracepoint]
pub fn cpu_frequency(ctx: TracePointContext) -> u32 {
    match try_cpu_frequency(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn sched_stat_wait(ctx: TracePointContext) -> u32 {
    match try_sched_stat_wait(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn sched_process_exit(_ctx: TracePointContext) -> u32 {
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;

    remove_runnable_task_if_present(tid);

    if let Some(old) = unsafe { WAKEUP_DATA.get(tid).copied() } {
        decrement_target_pending(old.target_cpu);
    }

    let _ = WAKEUP_DATA.remove(tid);
    let _ = PREV_FAULTS.remove(tid);
    0
}

#[aya_ebpf::macros::perf_event]
pub fn major_fault(_ctx: aya_ebpf::programs::PerfEventContext) -> u32 {
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;
    if is_target_pid_or_current_cgroup(tid) {
        if let Some(counters) = PREV_FAULTS.get_ptr_mut(tid) {
            unsafe { (*counters).maj += 1 };
        } else {
            let _ = PREV_FAULTS.insert(tid, FaultCounters { maj: 1, min: 0 }, 0);
        }
    }
    0
}

#[aya_ebpf::macros::perf_event]
pub fn minor_fault(_ctx: aya_ebpf::programs::PerfEventContext) -> u32 {
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;
    if is_target_pid_or_current_cgroup(tid) {
        if let Some(counters) = PREV_FAULTS.get_ptr_mut(tid) {
            unsafe { (*counters).min += 1 };
        } else {
            let _ = PREV_FAULTS.insert(tid, FaultCounters { maj: 0, min: 1 }, 0);
        }
    }
    0
}

#[tracepoint]
pub fn irq_handler_entry(ctx: TracePointContext) -> u32 {
    match try_irq_handler_entry(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn irq_handler_exit(ctx: TracePointContext) -> u32 {
    match try_irq_handler_exit(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[inline(always)]
fn is_target_current_cgroup() -> bool {
    // Experimental current-task filter. This must not be used for wakee pid
    // fields from scheduler tracepoints.
    let cgroup_id = unsafe { bpf_get_current_cgroup_id() };
    unsafe { TARGET_CGROUP_IDS.get(cgroup_id).is_some() }
}

#[inline(always)]
fn is_target_pid(pid: u32) -> bool {
    // Only consider explicit per-TID entries populated from userspace.
    // Avoid relying on eBPF cgroup heuristics for discovery — userspace
    // periodically refreshes `TARGET_PIDS` from the cgroup tree.
    unsafe { TARGET_PIDS.get(pid).is_some() }
}

#[inline(always)]
fn is_target_pid_or_current_cgroup(pid: u32) -> bool {
    is_target_pid(pid) || is_target_current_cgroup()
}

fn is_target_irq(irq: u32) -> bool {
    unsafe { TARGET_IRQS.get(irq).is_some() }
}

fn irq_key(irq: u32, cpu: u32) -> u64 {
    ((irq as u64) << 32) | cpu as u64
}

fn increment_drop_counter(reason: u32) {
    let Some(counter) = DROP_COUNTERS.get_ptr_mut(reason) else {
        return;
    };

    unsafe {
        *counter = (*counter).saturating_add(1);
    }
}

#[inline(always)]
fn valid_cpu(cpu: u32) -> bool {
    cpu < 1024
}

#[inline(always)]
fn read_cpu_runnable_depth(cpu: u32) -> u32 {
    if !valid_cpu(cpu) {
        return 0;
    }
    CPU_RUNNABLE_DEPTH.get(cpu).copied().unwrap_or(0)
}

#[inline(always)]
fn increment_cpu_runnable_depth(cpu: u32) {
    if !valid_cpu(cpu) {
        return;
    }
    if let Some(depth) = CPU_RUNNABLE_DEPTH.get_ptr_mut(cpu) {
        unsafe { *depth = (*depth).saturating_add(1) };
    }
}

#[inline(always)]
fn decrement_cpu_runnable_depth(cpu: u32) {
    if !valid_cpu(cpu) {
        return;
    }
    if let Some(depth) = CPU_RUNNABLE_DEPTH.get_ptr_mut(cpu) {
        unsafe { *depth = (*depth).saturating_sub(1) };
    }
}

#[inline(always)]
fn mark_task_runnable(pid: u32, target_cpu: u32) {
    if pid == 0 || !valid_cpu(target_cpu) {
        return;
    }

    match unsafe { RUNNABLE_TASK_CPU.get(pid).copied() } {
        Some(old_cpu) if old_cpu == target_cpu => {
            // Already counted on the same CPU.
        }
        Some(old_cpu) => {
            // Migrated while runnable.
            decrement_cpu_runnable_depth(old_cpu);
            increment_cpu_runnable_depth(target_cpu);
            let _ = RUNNABLE_TASK_CPU.insert(pid, target_cpu, 0);
        }
        None => {
            increment_cpu_runnable_depth(target_cpu);
            let _ = RUNNABLE_TASK_CPU.insert(pid, target_cpu, 0);
        }
    }
}

#[inline(always)]
fn mark_task_running(pid: u32, current_cpu: u32) {
    if pid == 0 {
        return;
    }

    if let Some(stored_cpu) = unsafe { RUNNABLE_TASK_CPU.get(pid).copied() } {
        let cpu_to_decrement = if valid_cpu(stored_cpu) {
            stored_cpu
        } else {
            current_cpu
        };
        decrement_cpu_runnable_depth(cpu_to_decrement);
        let _ = RUNNABLE_TASK_CPU.remove(pid);
    }
}

#[inline(always)]
fn remove_runnable_task_if_present(pid: u32) {
    if pid == 0 {
        return;
    }
    if let Some(cpu) = unsafe { RUNNABLE_TASK_CPU.get(pid).copied() } {
        decrement_cpu_runnable_depth(cpu);
        let _ = RUNNABLE_TASK_CPU.remove(pid);
    }
}

fn increment_target_pending(cpu: u32) {
    if let Some(depth) = TARGET_PENDING_WAKEUPS.get_ptr_mut(cpu) {
        // Diagnostic-only increment.
        unsafe { *depth = (*depth).saturating_add(1) };
    }
}

fn decrement_target_pending(cpu: u32) {
    if let Some(depth) = TARGET_PENDING_WAKEUPS.get_ptr_mut(cpu) {
        // Diagnostic-only decrement.
        unsafe { *depth = (*depth).saturating_sub(1) };
    }
}

fn try_sched_wakeup(ctx: TracePointContext) -> Result<u32, u32> {
    let pid: i32 = unsafe { ctx.read_at(24).map_err(|_| 1u32)? };
    let target_cpu: u32 = unsafe { ctx.read_at(32).map_err(|_| 1u32)? };

    if pid <= 0 {
        return Ok(0);
    }

    let pid = pid as u32;

    // Keep PID filtering here. current_cgroup is the waker, not the wakee.
    // Runnable-depth accounting is intentionally target-local: only monitored
    // target tasks are counted. Do not call mark_task_runnable() before this
    // filter, or unrelated system wakeups can leak into CPU_RUNNABLE_DEPTH.
    if !is_target_pid(pid) {
        return Ok(0);
    }

    // Count only monitored target tasks as runnable. This keeps the increment path
    // aligned with sched_switch, which only decrements after consuming WAKEUP_DATA
    // for a monitored target task.
    mark_task_runnable(pid, target_cpu);

    let now = unsafe { bpf_ktime_get_ns() };
    let waker_tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;

    let data = WakeupData {
        ts: now,
        target_cpu,
        waker_tid,
    };
    let old_wakeup_data = unsafe { WAKEUP_DATA.get(pid).copied() };

    if WAKEUP_DATA.insert(pid, data, 0).is_ok() {
        match old_wakeup_data {
            None => increment_target_pending(target_cpu),
            Some(old) if old.target_cpu != target_cpu => {
                decrement_target_pending(old.target_cpu);
                increment_target_pending(target_cpu);
            }
            Some(_) => {}
        }
    } else {
        increment_drop_counter(DROP_WAKEUP_DATA_INSERT_FAILED);
    }

    Ok(0)
}

fn try_sched_switch(ctx: TracePointContext) -> Result<u32, u32> {
    let next_pid: i32 = unsafe { ctx.read_at(56).map_err(|_| 1u32)? };

    if next_pid <= 0 {
        return Ok(0);
    }

    let pid = next_pid as u32;

    // Read the previous task context from the tracepoint: prev_pid and prev_state.
    // Offsets validated in userspace preflight assume prev_pid at 24 and prev_state at 32.
    let prev_pid_raw: i32 = unsafe { ctx.read_at(24).map_err(|_| 1u32)? };
    let prev_state: i64 = unsafe { ctx.read_at(32).map_err(|_| 1u32)? };

    let switch_prev_pid = if prev_pid_raw > 0 {
        prev_pid_raw as u32
    } else {
        0
    };

    let wakeup_data = match unsafe { WAKEUP_DATA.get(pid) } {
        Some(d) => *d,
        None => return Ok(0),
    };

    let _ = WAKEUP_DATA.remove(pid);

    let wakeup_ns = wakeup_data.ts;
    let waker_tid = wakeup_data.waker_tid;
    let target_cpu = wakeup_data.target_cpu;

    // We only arrived here because a wakeup record for this PID exists
    // (inserted by sched_wakeup). Treat that as sufficient evidence this is
    // a target-related event.

    let cpu = unsafe { bpf_get_smp_processor_id() } as u32;

    // Every sched_switch means next_pid is now running, so it is no longer
    // counted as runnable in our approximation.
    mark_task_running(pid, cpu);

    // Read monitored runnable depth after removing next_pid. This represents
    // remaining monitored target tasks that are still counted runnable on this CPU,
    // not total kernel runqueue depth.
    let observed_runnable_depth = read_cpu_runnable_depth(cpu);

    // Decrement the target-pending counter for the CPU where the task was
    // originally queued. This is not kernel runqueue depth.
    decrement_target_pending(target_cpu);

    let target_pending_wakeups = TARGET_PENDING_WAKEUPS.get(cpu).copied().unwrap_or(0);

    let switch_ns = unsafe { bpf_ktime_get_ns() };
    let latency_ns = switch_ns.saturating_sub(wakeup_ns);

    let faults = unsafe {
        PREV_FAULTS
            .get(pid)
            .copied()
            .unwrap_or(FaultCounters { maj: 0, min: 0 })
    };

    let prio: i32 = unsafe { ctx.read_at(60).map_err(|_| 1u32)? };
    let comm: [u8; 16] = unsafe { ctx.read_at(40).map_err(|_| 1u32)? };

    let Some(mut entry) = EVENTS.reserve::<SchedulerEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return Ok(0);
    };
    let event = entry.as_mut_ptr();

    unsafe {
        (*event).kind = EVENT_RUNNABLE_LATENCY;
        (*event).pid = pid;
        (*event).cpu = cpu;
        (*event).wakeup_target_cpu = target_cpu;
        (*event).prio = prio;
        (*event).waker_tid = waker_tid;
        (*event).target_pending_wakeups = target_pending_wakeups;
        (*event).observed_runnable_depth = observed_runnable_depth;
        (*event).maj_flt = faults.maj;
        (*event).min_flt = faults.min;
        (*event).wakeup_ns = wakeup_ns;
        (*event).switch_ns = switch_ns;
        (*event).latency_ns = latency_ns;
        (*event).comm = comm;
        (*event).switch_prev_pid = switch_prev_pid;
        (*event).switch_prev_state = prev_state;
    }

    entry.submit(0);

    Ok(0)
}

fn try_sched_migrate_task(ctx: TracePointContext) -> Result<u32, u32> {
    let pid: i32 = unsafe { ctx.read_at(12).map_err(|_| 1u32)? };
    if pid <= 0 {
        return Ok(0);
    }

    let pid = pid as u32;
    if !is_target_pid(pid) {
        return Ok(0);
    }

    let orig_cpu: i32 = unsafe { ctx.read_at(20).map_err(|_| 1u32)? };
    let dest_cpu: i32 = unsafe { ctx.read_at(24).map_err(|_| 1u32)? };
    let now = unsafe { bpf_ktime_get_ns() };

    // Move monitored runnable count if this target task migrates while runnable.
    match unsafe { RUNNABLE_TASK_CPU.get(pid).copied() } {
        Some(old_cpu) if old_cpu != dest_cpu as u32 => {
            let new_cpu = dest_cpu as u32;
            decrement_cpu_runnable_depth(old_cpu);
            increment_cpu_runnable_depth(new_cpu);
            let _ = RUNNABLE_TASK_CPU.insert(pid, new_cpu, 0);
        }
        _ => {}
    }

    // If this task is currently a monitored target with a pending wakeup,
    // update its target CPU and move its diagnostic-only pending counter.
    if let Some(data) = WAKEUP_DATA.get_ptr_mut(pid) {
        let old_cpu = unsafe { (*data).target_cpu };
        let new_cpu = dest_cpu as u32;
        if old_cpu != new_cpu {
            unsafe { (*data).target_cpu = new_cpu };
            decrement_target_pending(old_cpu);
            increment_target_pending(new_cpu);
        }
    }

    let Some(mut entry) = EVENTS.reserve::<MigrationEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return Ok(0);
    };
    let event = entry.as_mut_ptr();

    unsafe {
        (*event).kind = EVENT_MIGRATION;
        (*event).tid = pid;
        (*event).from_cpu = orig_cpu as u32;
        (*event).to_cpu = dest_cpu as u32;
        (*event).timestamp_ns = now;
    }

    entry.submit(0);

    Ok(0)
}

fn try_cpu_frequency(ctx: TracePointContext) -> Result<u32, u32> {
    let state: u32 = unsafe { ctx.read_at(8).map_err(|_| 1u32)? };
    let cpu_id: u32 = unsafe { ctx.read_at(12).map_err(|_| 1u32)? };
    let now = unsafe { bpf_ktime_get_ns() };

    let Some(mut entry) = EVENTS.reserve::<CpuFreqEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return Ok(0);
    };
    let event = entry.as_mut_ptr();

    unsafe {
        (*event).kind = EVENT_CPU_FREQ;
        (*event).cpu = cpu_id;
        (*event).state = state;
        (*event)._pad = 0;
        (*event).timestamp_ns = now;
    }

    entry.submit(0);

    Ok(0)
}

fn try_sched_stat_wait(ctx: TracePointContext) -> Result<u32, u32> {
    let pid: i32 = unsafe { ctx.read_at(8).map_err(|_| 1u32)? };
    if pid <= 0 {
        return Ok(0);
    }

    let pid = pid as u32;
    if !is_target_pid(pid) {
        return Ok(0);
    }

    let delay: u64 = unsafe { ctx.read_at(16).map_err(|_| 1u32)? };

    let Some(mut entry) = EVENTS.reserve::<StatWaitEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return Ok(0);
    };
    let event = entry.as_mut_ptr();

    unsafe {
        (*event).kind = EVENT_STAT_WAIT;
        (*event).tid = pid;
        (*event).delay_ns = delay;
    }

    entry.submit(0);

    Ok(0)
}

fn try_irq_handler_entry(ctx: TracePointContext) -> Result<u32, u32> {
    let irq: i32 = unsafe { ctx.read_at(8).map_err(|_| 1u32)? };
    if irq < 0 {
        return Ok(0);
    }

    let irq = irq as u32;
    if !is_target_irq(irq) {
        return Ok(0);
    }

    let cpu = unsafe { bpf_get_smp_processor_id() };
    let key = irq_key(irq, cpu);
    let now = unsafe { bpf_ktime_get_ns() };
    if IRQ_START_TIMES.insert(key, now, 0).is_err() {
        increment_drop_counter(DROP_IRQ_START_TIMES_INSERT_FAILED);
    }
    Ok(0)
}

fn try_irq_handler_exit(ctx: TracePointContext) -> Result<u32, u32> {
    let irq: i32 = unsafe { ctx.read_at(8).map_err(|_| 1u32)? };
    if irq < 0 {
        return Ok(0);
    }

    let irq = irq as u32;
    if !is_target_irq(irq) {
        return Ok(0);
    }

    let cpu = unsafe { bpf_get_smp_processor_id() };
    let key = irq_key(irq, cpu);
    let enter_ns = match unsafe { IRQ_START_TIMES.get(key) } {
        Some(ts) => *ts,
        None => return Ok(0),
    };
    let _ = IRQ_START_TIMES.remove(key);

    let exit_ns = unsafe { bpf_ktime_get_ns() };
    let duration_ns = exit_ns.saturating_sub(enter_ns);

    let Some(mut entry) = EVENTS.reserve::<IrqEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return Ok(0);
    };
    let event = entry.as_mut_ptr();

    unsafe {
        (*event).kind = EVENT_IRQ_LATENCY;
        (*event).irq = irq;
        (*event).cpu = cpu;
        (*event).enter_ns = enter_ns;
        (*event).exit_ns = exit_ns;
        (*event).duration_ns = duration_ns;
    }

    entry.submit(0);
    Ok(0)
}

#[tracepoint]
pub fn block_rq_issue(ctx: TracePointContext) -> u32 {
    match try_block_rq_issue(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

fn try_block_rq_issue(ctx: TracePointContext) -> Result<u32, u32> {
    let dev: u32 = unsafe { ctx.read_at(8).map_err(|_| 1u32)? };
    let sector: u64 = unsafe { ctx.read_at(16).map_err(|_| 1u32)? };
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;
    // Only track starts for target tasks so unrelated system I/O cannot
    // evict target entries from the start LRU map.
    if !is_target_pid_or_current_cgroup(tid) {
        return Ok(0);
    }
    let ts = unsafe { bpf_ktime_get_ns() };
    // If userspace detected a unique request pointer field (like `rq`), use it
    // as the primary key. Otherwise fall back to a multi-field metadata hash.
    let key = if unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_KEY_OFFSET) } != 0 {
        unsafe {
            ctx.read_at::<u64>(core::ptr::read_volatile(&raw const BLOCK_RQ_KEY_OFFSET) as usize)
                .unwrap_or(0)
        }
    } else {
        // Use a 64-bit mixed key of (sector, dev, nr_sector, rwbs) to minimize
        // collisions during the fallback correlation mode.
        let mut h = sector.wrapping_mul(11400714819323198485u64)
            ^ ((dev as u64).wrapping_mul(14029467366897019727u64));

        let nr_sector_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_ISSUE_NR_SECTOR_OFFSET) };
        if nr_sector_offset != 0 {
            let nr_sector: u32 = unsafe { ctx.read_at(nr_sector_offset as usize).unwrap_or(0) };
            h ^= (nr_sector as u64).wrapping_mul(11500714819323198485u64);
        }

        let rwbs_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_ISSUE_RWBS_OFFSET) };
        if rwbs_offset != 0 {
            let rwbs: u64 = unsafe { ctx.read_at(rwbs_offset as usize).unwrap_or(0) };
            h ^= rwbs.wrapping_mul(11600714819323198485u64);
        }
        h
    };

    if key != 0 && BLOCK_START.insert(key, IoStart { ts, tid }, 0).is_err() {
        increment_drop_counter(DROP_BLOCK_START_INSERT_FAILED);
    }

    Ok(0)
}

#[tracepoint]
pub fn block_rq_complete(ctx: TracePointContext) -> u32 {
    match try_block_rq_complete(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

fn try_block_rq_complete(ctx: TracePointContext) -> Result<u32, u32> {
    let dev: u32 = unsafe { ctx.read_at(8).map_err(|_| 1u32)? };
    let sector: u64 = unsafe { ctx.read_at(16).map_err(|_| 1u32)? };
    let nr_sector: u32 = unsafe { ctx.read_at(24).map_err(|_| 1u32)? };

    let key = if unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_KEY_OFFSET) } != 0 {
        unsafe {
            ctx.read_at::<u64>(core::ptr::read_volatile(&raw const BLOCK_RQ_KEY_OFFSET) as usize)
                .unwrap_or(0)
        }
    } else {
        let mut h = sector.wrapping_mul(11400714819323198485u64)
            ^ ((dev as u64).wrapping_mul(14029467366897019727u64));

        let nr_sector_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET) };
        if nr_sector_offset != 0 {
            let nr_sector_val: u32 = unsafe { ctx.read_at(nr_sector_offset as usize).unwrap_or(0) };
            h ^= (nr_sector_val as u64).wrapping_mul(11500714819323198485u64);
        }

        let rwbs_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_COMPLETE_RWBS_OFFSET) };
        if rwbs_offset != 0 {
            let rwbs_val: u64 = unsafe { ctx.read_at(rwbs_offset as usize).unwrap_or(0) };
            h ^= rwbs_val.wrapping_mul(11600714819323198485u64);
        }
        h
    };

    let start = match if key != 0 {
        unsafe { BLOCK_START.get(key) }
    } else {
        None
    } {
        Some(s) => *s,
        None => return Ok(0),
    };
    let _ = BLOCK_START.remove(key);

    let now = unsafe { bpf_ktime_get_ns() };
    let duration_ns = now.saturating_sub(start.ts);

    let Some(mut entry) = EVENTS.reserve::<BlockIoEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return Ok(0);
    };
    let event = entry.as_mut_ptr();

    let rwbs_offset = unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_COMPLETE_RWBS_OFFSET) };
    let rwbs = if rwbs_offset != 0 {
        unsafe { ctx.read_at(rwbs_offset as usize).unwrap_or([0u8; 8]) }
    } else {
        [0u8; 8]
    };

    unsafe {
        (*event).kind = EVENT_BLOCK_IO;
        (*event).tid = start.tid;
        (*event).dev = dev;
        (*event).nr_sector = nr_sector;
        (*event).sector = sector;
        (*event).duration_ns = duration_ns;
        (*event).timestamp_ns = now;
        (*event).rwbs = rwbs;
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
