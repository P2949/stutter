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
    BPF_DEFAULT_EVENTS_RINGBUF_BYTES, BPF_MAX_TRACKED_CPUS, CpuFreqEvent, DRM_FENCE_EVENT_SIGNAL,
    DRM_FENCE_EVENT_WAIT_DONE, DRM_FENCE_EVENT_WAIT_INTERVAL, DRM_FENCE_HAS_CONTEXT,
    DRM_FENCE_HAS_DURATION, DRM_FENCE_HAS_PID, DRM_FENCE_HAS_SEQNO, DRM_FENCE_HAS_TIMELINE,
    DRM_FENCE_IS_EXPORTER_SIDE, DRM_FENCE_IS_IMPORTER_SIDE, DROP_COUNTERS_MAX,
    DROP_CPU_ACCOUNTING_UNTRACKED, DROP_IRQ_START_TIMES_INSERT_FAILED,
    DROP_WAKEUP_DATA_CONSUMED_READ_FAILED, DROP_WAKEUP_DATA_INSERT_FAILED,
    DROP_WAKEUP_DATA_REPLACED_ENTRY, DROP_WAKEUP_DATA_STALE_ENTRY, DrmFenceEvent, EVENT_CPU_FREQ,
    EVENT_DRM_FENCE, EVENT_EXEC, EVENT_IRQ_LATENCY, EVENT_MIGRATION, EVENT_RUNNABLE_LATENCY,
    EVENT_STAT_WAIT, ExecEvent, IrqEvent, KMS_FLIP_EVENT_PAGEFLIP_DONE, KMS_FLIP_EVENT_VBLANK,
    KMS_FLIP_PROVIDER_AMDGPU, KMS_FLIP_PROVIDER_DRM, KMS_FLIP_PROVIDER_I915, MigrationEvent,
    SchedulerEvent, StatWaitEvent,
    tracepoint_offsets::{
        CPU_FREQUENCY_CPU_ID_OFFSET, CPU_FREQUENCY_STATE_OFFSET, IRQ_HANDLER_IRQ_OFFSET,
        SCHED_MIGRATE_TASK_DEST_CPU_OFFSET, SCHED_MIGRATE_TASK_ORIG_CPU_OFFSET,
        SCHED_MIGRATE_TASK_PID_OFFSET, SCHED_STAT_WAIT_DELAY_OFFSET, SCHED_STAT_WAIT_PID_OFFSET,
        SCHED_SWITCH_NEXT_COMM_OFFSET, SCHED_SWITCH_NEXT_PID_OFFSET, SCHED_SWITCH_NEXT_PRIO_OFFSET,
        SCHED_SWITCH_PREV_PID_OFFSET, SCHED_SWITCH_PREV_STATE_OFFSET, SCHED_WAKEUP_PID_OFFSET,
        SCHED_WAKEUP_TARGET_CPU_OFFSET,
    },
};

// Layout:
// 1. Tracepoint field offsets and provider constants
// 2. Shared constants and map sizing
// 3. BPF maps and shared state structs
// 4. Scheduler entrypoints
// 5. Process lifecycle tracepoints
// 6. CPU frequency and scheduler wait tracepoints
// 7. Fault counters
// 8. IRQ overlap tracing
// 9. Target filtering, runnable-depth accounting, and drop accounting
// 10. Scheduler tracepoint implementations
// 11. Block I/O tracing
// 12. KMS/flip tracing
// 13. DRM fence waits/signals
// 14. KMS flip event emission
// 15. Tracepoint field readers
// 16. License and panic handler

macro_rules! emit_ringbuf_event {
    ($event_ty:ty, $reserve_failed:expr, $event:expr) => {{
        let Some(mut entry) = crate::EVENTS.reserve::<$event_ty>(0) else {
            crate::increment_drop_counter(stutter_common::DROP_RINGBUF_RESERVE_FAILED);
            $reserve_failed
        };
        unsafe { core::ptr::write(entry.as_mut_ptr(), $event) };
        entry.submit(0);
    }};
}

mod block_io;
mod kms_emit;
mod map_limits;
mod trace_offsets;
mod trace_read;
mod wakeup_data;

use kms_emit::emit_kms_flip_event;
use map_limits::{
    FENCE_SIGNAL_TIMES_MAP_MAX_ENTRIES, FENCE_WAIT_STARTS_MAP_MAX_ENTRIES,
    IRQ_START_TIMES_MAP_MAX_ENTRIES, KMS_FLIP_STARTS_MAP_MAX_ENTRIES, PREV_FAULTS_MAP_MAX_ENTRIES,
    RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES, TARGET_CGROUP_IDS_MAP_MAX_ENTRIES,
    TARGET_IRQS_MAP_MAX_ENTRIES, TARGET_PIDS_MAP_MAX_ENTRIES,
};
use trace_offsets::*;
use trace_read::{
    read_comm16, read_i32, read_i64, read_optional_u32, read_optional_u64, read_sequence_field,
    read_u32, read_u64,
};
use wakeup_data::{
    WAKEUP_CONSUME_CURSOR_INSERT_FAILED, WAKEUP_CONSUME_NONE, WAKEUP_CONSUME_OK,
    WAKEUP_RECORD_INSERT_FAILED, WAKEUP_RECORD_NEW_PENDING,
    WAKEUP_RECORD_REPLACED_PENDING_MOVED_CPU, WAKEUP_RECORD_REPLACED_PENDING_SAME_CPU, WakeupData,
};

// -----------------------------------------------------------------------------
// Shared constants and map sizing
// -----------------------------------------------------------------------------

const _: () = assert!(BPF_MAX_TRACKED_CPUS >= 1024);

// -----------------------------------------------------------------------------
// BPF maps and shared state structs
// -----------------------------------------------------------------------------

#[map]
// Userspace overrides this before loading the BPF object based on the current
// memlock limit and available memory. The value here is only a safe fallback.
static EVENTS: RingBuf = RingBuf::with_byte_size(BPF_DEFAULT_EVENTS_RINGBUF_BYTES, 0);

#[map]
static TARGET_PIDS: HashMap<u32, u8> =
    HashMap::<u32, u8>::with_max_entries(TARGET_PIDS_MAP_MAX_ENTRIES, 0);

#[map]
static TARGET_CGROUP_IDS: HashMap<u64, u8> =
    HashMap::<u64, u8>::with_max_entries(TARGET_CGROUP_IDS_MAP_MAX_ENTRIES, 0);

#[map]
static TARGET_IRQS: HashMap<u32, u8> =
    HashMap::<u32, u8>::with_max_entries(TARGET_IRQS_MAP_MAX_ENTRIES, 0);

#[map]
static IRQ_START_TIMES: HashMap<u64, u64> =
    HashMap::<u64, u64>::with_max_entries(IRQ_START_TIMES_MAP_MAX_ENTRIES, 0);

#[map]
// Diagnostic-only count of monitored pending wakeups for this target/task.
// This is not CPU runqueue depth and must not be used as true CPU contention.
static TARGET_PENDING_WAKEUPS: Array<u32> = Array::<u32>::with_max_entries(BPF_MAX_TRACKED_CPUS, 0);

#[map]
// Approximate per-CPU runnable depth for monitored target tasks only,
// reconstructed from sched wakeup/switch/migrate tracepoints.
// This is not literal rq->nr_running and does not include unrelated system tasks.
static CPU_RUNNABLE_DEPTH: Array<u32> = Array::<u32>::with_max_entries(BPF_MAX_TRACKED_CPUS, 0);

#[map]
// Per-target-TID mapping to the CPU where the monitored task was last counted
// as runnable. Used to move monitored runnable counts during migration and
// avoid double-counting duplicate wakeups.
static RUNNABLE_TASK_CPU: LruHashMap<u32, u32> =
    LruHashMap::<u32, u32>::with_max_entries(RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES, 0);

#[repr(C)]
#[derive(Clone, Copy)]
struct FaultCounters {
    maj: u64,
    min: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct KmsFlipKey {
    card_minor: u32,
    crtc_id: u32,
    pipe: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FenceKey {
    context: u64,
    seqno: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FenceWaitStart {
    ts: u64,
    pid: u32,
    tid: u32,
    provider: u32,
    gpu_role: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FenceSignal {
    ts: u64,
    provider: u32,
    gpu_role: u32,
}

#[map]
static PREV_FAULTS: HashMap<u32, FaultCounters> =
    HashMap::<u32, FaultCounters>::with_max_entries(PREV_FAULTS_MAP_MAX_ENTRIES, 0);

#[map]
static KMS_FLIP_STARTS: HashMap<KmsFlipKey, u64> =
    HashMap::<KmsFlipKey, u64>::with_max_entries(KMS_FLIP_STARTS_MAP_MAX_ENTRIES, 0);

#[map]
static FENCE_WAIT_STARTS: HashMap<FenceKey, FenceWaitStart> =
    HashMap::<FenceKey, FenceWaitStart>::with_max_entries(FENCE_WAIT_STARTS_MAP_MAX_ENTRIES, 0);

#[map]
static FENCE_SIGNAL_TIMES: HashMap<FenceKey, FenceSignal> =
    HashMap::<FenceKey, FenceSignal>::with_max_entries(FENCE_SIGNAL_TIMES_MAP_MAX_ENTRIES, 0);

#[map]
static DROP_COUNTERS: PerCpuArray<u64> = PerCpuArray::<u64>::with_max_entries(DROP_COUNTERS_MAX, 0);

// -----------------------------------------------------------------------------
// Scheduler entrypoints
// -----------------------------------------------------------------------------

#[tracepoint]
/// Tracepoint entry for sched_wakeup.
/// Records target wakeup timing and runnable-depth state.
pub fn sched_wakeup(ctx: TracePointContext) -> u32 {
    try_sched_wakeup(ctx)
}

#[tracepoint]
/// Tracepoint entry for sched_wakeup_new.
/// Uses the same target wakeup accounting as sched_wakeup.
pub fn sched_wakeup_new(ctx: TracePointContext) -> u32 {
    try_sched_wakeup(ctx)
}

#[tracepoint]
/// Tracepoint entry for sched_switch.
/// Emits runnable latency for target tasks with pending wakeup data.
pub fn sched_switch(ctx: TracePointContext) -> u32 {
    try_sched_switch(ctx)
}

#[tracepoint]
/// Tracepoint entry for sched_migrate_task.
/// Moves monitored runnable-depth accounting across CPUs.
pub fn sched_migrate_task(ctx: TracePointContext) -> u32 {
    try_sched_migrate_task(ctx)
}

// -----------------------------------------------------------------------------
// Process lifecycle tracepoints
// -----------------------------------------------------------------------------

#[tracepoint]
/// Tracepoint entry for sched_process_exec.
/// Emits exec events for monitored tasks or current target cgroups.
pub fn sched_process_exec(ctx: TracePointContext) -> u32 {
    try_sched_process_exec(ctx)
}

#[inline(always)]
fn try_sched_process_exec(_ctx: TracePointContext) -> u32 {
    let pid_tgid = bpf_get_current_pid_tgid();
    let pid = (pid_tgid >> 32) as u32;
    let tid = (pid_tgid & 0xffff_ffff) as u32;

    if !is_target_pid(pid) && !is_target_pid_or_current_cgroup(tid) {
        return 0;
    }

    let comm = aya_ebpf::helpers::bpf_get_current_comm().unwrap_or([0; 16]);
    emit_ringbuf_event!(
        ExecEvent,
        return 0,
        ExecEvent {
            kind: EVENT_EXEC,
            pid,
            tid,
            comm,
        }
    );

    0
}

// -----------------------------------------------------------------------------
// CPU frequency and scheduler wait tracepoints
// -----------------------------------------------------------------------------

#[tracepoint]
/// Tracepoint entry for cpu_frequency.
/// Emits CPU frequency state changes.
pub fn cpu_frequency(ctx: TracePointContext) -> u32 {
    try_cpu_frequency(ctx)
}

#[tracepoint]
/// Tracepoint entry for sched_stat_wait.
/// Emits scheduler wait delay for monitored target tasks.
pub fn sched_stat_wait(ctx: TracePointContext) -> u32 {
    try_sched_stat_wait(ctx)
}

#[tracepoint]
/// Tracepoint entry for sched_process_exit.
/// Clears per-task wakeup, runnable, and fault state.
pub fn sched_process_exit(_ctx: TracePointContext) -> u32 {
    let tid = (bpf_get_current_pid_tgid() & 0xffff_ffff) as u32;

    remove_runnable_task_if_present(tid);

    let mut old = WakeupData::default();
    if wakeup_data::remove_for_exit(tid, &mut old) {
        decrement_target_pending(old.target_cpu);
    }
    let _ = PREV_FAULTS.remove(tid);
    0
}

// -----------------------------------------------------------------------------
// Fault counters
// -----------------------------------------------------------------------------

#[aya_ebpf::macros::perf_event]
/// Perf-event entry for major page faults.
/// Increments per-target fault counters used on later scheduler events.
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
/// Perf-event entry for minor page faults.
/// Increments per-target fault counters used on later scheduler events.
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

// -----------------------------------------------------------------------------
// IRQ overlap tracing
// -----------------------------------------------------------------------------

#[tracepoint]
/// Tracepoint entry for irq_handler_entry.
/// Records start time for allowlisted IRQs on the current CPU.
pub fn irq_handler_entry(ctx: TracePointContext) -> u32 {
    try_irq_handler_entry(ctx)
}

#[tracepoint]
/// Tracepoint entry for irq_handler_exit.
/// Emits IRQ duration for matching target IRQ starts.
pub fn irq_handler_exit(ctx: TracePointContext) -> u32 {
    try_irq_handler_exit(ctx)
}

// -----------------------------------------------------------------------------
// Target filtering, runnable-depth accounting, and drop accounting
// -----------------------------------------------------------------------------

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
    // High 32 bits: IRQ number. Low 32 bits: CPU ID.
    // CPU IDs such as 65_536 remain entirely in the low half and do not collide
    // with the IRQ half of the packed key.
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
    cpu < BPF_MAX_TRACKED_CPUS
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
    if pid == 0 {
        return;
    }
    if !valid_cpu(target_cpu) {
        increment_drop_counter(DROP_CPU_ACCOUNTING_UNTRACKED);
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
    if !valid_cpu(cpu) {
        increment_drop_counter(DROP_CPU_ACCOUNTING_UNTRACKED);
        return;
    }
    if let Some(depth) = TARGET_PENDING_WAKEUPS.get_ptr_mut(cpu) {
        // Diagnostic-only increment.
        unsafe { *depth = (*depth).saturating_add(1) };
    }
}

fn decrement_target_pending(cpu: u32) {
    if !valid_cpu(cpu) {
        return;
    }
    if let Some(depth) = TARGET_PENDING_WAKEUPS.get_ptr_mut(cpu) {
        // Diagnostic-only decrement.
        unsafe { *depth = (*depth).saturating_sub(1) };
    }
}

// -----------------------------------------------------------------------------
// Scheduler tracepoint implementations
// -----------------------------------------------------------------------------

#[inline(always)]
fn try_sched_wakeup(ctx: TracePointContext) -> u32 {
    let mut pid: i32 = 0;
    if !read_i32(&ctx, SCHED_WAKEUP_PID_OFFSET, &mut pid) {
        return 1;
    }
    let mut target_cpu: u32 = 0;
    if !read_u32(&ctx, SCHED_WAKEUP_TARGET_CPU_OFFSET, &mut target_cpu) {
        return 1;
    }

    if pid <= 0 {
        return 0;
    }

    let pid = pid as u32;

    // Keep PID filtering here. current_cgroup is the waker, not the wakee.
    // Runnable-depth accounting is intentionally target-local: only monitored
    // target tasks are counted. Do not call mark_task_runnable() before this
    // filter, or unrelated system wakeups can leak into CPU_RUNNABLE_DEPTH.
    if !is_target_pid(pid) {
        return 0;
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
    let mut old = WakeupData::default();

    match wakeup_data::record_wakeup(pid, data, &mut old) {
        WAKEUP_RECORD_NEW_PENDING => increment_target_pending(target_cpu),
        WAKEUP_RECORD_REPLACED_PENDING_MOVED_CPU => {
            increment_drop_counter(DROP_WAKEUP_DATA_REPLACED_ENTRY);
            decrement_target_pending(old.target_cpu);
            increment_target_pending(target_cpu);
        }
        WAKEUP_RECORD_REPLACED_PENDING_SAME_CPU => {
            increment_drop_counter(DROP_WAKEUP_DATA_REPLACED_ENTRY);
        }
        WAKEUP_RECORD_INSERT_FAILED => increment_drop_counter(DROP_WAKEUP_DATA_INSERT_FAILED),
        _ => {}
    }

    0
}

#[inline(always)]
fn try_sched_switch(ctx: TracePointContext) -> u32 {
    let mut next_pid: i32 = 0;
    if !read_i32(&ctx, SCHED_SWITCH_NEXT_PID_OFFSET, &mut next_pid) {
        return 1;
    }

    if next_pid <= 0 {
        return 0;
    }

    let pid = next_pid as u32;

    let mut wakeup_data = WakeupData::default();
    match wakeup_data::consume_pending_wakeup(pid, &mut wakeup_data) {
        WAKEUP_CONSUME_OK => {}
        WAKEUP_CONSUME_CURSOR_INSERT_FAILED => {
            increment_drop_counter(DROP_WAKEUP_DATA_INSERT_FAILED);
            return 0;
        }
        WAKEUP_CONSUME_NONE => return 0,
        _ => return 0,
    }

    if !is_target_pid(pid) {
        increment_drop_counter(DROP_WAKEUP_DATA_STALE_ENTRY);
        decrement_target_pending(wakeup_data.target_cpu);
        remove_runnable_task_if_present(pid);
        return 0;
    }

    // Wakeup data is marked consumed before slower tracepoint reads to avoid stale state.

    // Read the previous task context only after the cheap relevance filters pass.
    // Offsets are named above and validated in userspace preflight before load.
    let mut prev_pid_raw: i32 = 0;
    if !read_i32(&ctx, SCHED_SWITCH_PREV_PID_OFFSET, &mut prev_pid_raw) {
        increment_drop_counter(DROP_WAKEUP_DATA_CONSUMED_READ_FAILED);
        decrement_target_pending(wakeup_data.target_cpu);
        remove_runnable_task_if_present(pid);
        return 1;
    }
    let mut prev_state: i64 = 0;
    if !read_i64(&ctx, SCHED_SWITCH_PREV_STATE_OFFSET, &mut prev_state) {
        increment_drop_counter(DROP_WAKEUP_DATA_CONSUMED_READ_FAILED);
        decrement_target_pending(wakeup_data.target_cpu);
        remove_runnable_task_if_present(pid);
        return 1;
    }
    let switch_prev_pid = if prev_pid_raw > 0 {
        prev_pid_raw as u32
    } else {
        0
    };

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

    let target_pending_wakeups = if valid_cpu(cpu) {
        TARGET_PENDING_WAKEUPS.get(cpu).copied().unwrap_or(0)
    } else {
        0
    };

    let switch_ns = unsafe { bpf_ktime_get_ns() };
    let latency_ns = switch_ns.saturating_sub(wakeup_ns);

    let faults = unsafe {
        PREV_FAULTS
            .get(pid)
            .copied()
            .unwrap_or(FaultCounters { maj: 0, min: 0 })
    };

    let mut prio: i32 = 0;
    if !read_i32(&ctx, SCHED_SWITCH_NEXT_PRIO_OFFSET, &mut prio) {
        return 1;
    }
    let mut comm: [u8; 16] = [0; 16];
    if !read_comm16(&ctx, SCHED_SWITCH_NEXT_COMM_OFFSET, &mut comm) {
        return 1;
    }

    emit_ringbuf_event!(
        SchedulerEvent,
        return 0,
        SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            tid: pid,
            cpu,
            wakeup_target_cpu: target_cpu,
            prio,
            waker_tid,
            target_pending_wakeups,
            observed_runnable_depth,
            maj_flt: faults.maj,
            min_flt: faults.min,
            wakeup_ns,
            switch_ns,
            latency_ns,
            comm,
            switch_prev_pid,
            _pad0: 0,
            switch_prev_state: prev_state,
        }
    );

    0
}

#[inline(always)]
fn try_sched_migrate_task(ctx: TracePointContext) -> u32 {
    let mut pid: i32 = 0;
    if !read_i32(&ctx, SCHED_MIGRATE_TASK_PID_OFFSET, &mut pid) {
        return 1;
    }
    if pid <= 0 {
        return 0;
    }

    let pid = pid as u32;
    if !is_target_pid(pid) {
        return 0;
    }

    let mut orig_cpu: i32 = 0;
    if !read_i32(&ctx, SCHED_MIGRATE_TASK_ORIG_CPU_OFFSET, &mut orig_cpu) {
        return 1;
    }
    let mut dest_cpu: i32 = 0;
    if !read_i32(&ctx, SCHED_MIGRATE_TASK_DEST_CPU_OFFSET, &mut dest_cpu) {
        return 1;
    }
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
    let mut old_cpu = 0;
    let new_cpu = dest_cpu as u32;
    if wakeup_data::move_pending_cpu(pid, new_cpu, &mut old_cpu) {
        decrement_target_pending(old_cpu);
        increment_target_pending(new_cpu);
    }

    emit_ringbuf_event!(
        MigrationEvent,
        return 0,
        MigrationEvent {
            kind: EVENT_MIGRATION,
            tid: pid,
            from_cpu: orig_cpu as u32,
            to_cpu: dest_cpu as u32,
            timestamp_ns: now,
        }
    );

    0
}

#[inline(always)]
fn try_cpu_frequency(ctx: TracePointContext) -> u32 {
    let mut state: u32 = 0;
    if !read_u32(&ctx, CPU_FREQUENCY_STATE_OFFSET, &mut state) {
        return 1;
    }
    let mut cpu_id: u32 = 0;
    if !read_u32(&ctx, CPU_FREQUENCY_CPU_ID_OFFSET, &mut cpu_id) {
        return 1;
    }
    let now = unsafe { bpf_ktime_get_ns() };

    emit_ringbuf_event!(
        CpuFreqEvent,
        return 0,
        CpuFreqEvent {
            kind: EVENT_CPU_FREQ,
            cpu: cpu_id,
            state,
            _pad: 0,
            timestamp_ns: now,
        }
    );

    0
}

#[inline(always)]
fn try_sched_stat_wait(ctx: TracePointContext) -> u32 {
    let mut pid: i32 = 0;
    if !read_i32(&ctx, SCHED_STAT_WAIT_PID_OFFSET, &mut pid) {
        return 1;
    }
    if pid <= 0 {
        return 0;
    }

    let pid = pid as u32;
    if !is_target_pid(pid) {
        return 0;
    }

    let mut delay: u64 = 0;
    if !read_u64(&ctx, SCHED_STAT_WAIT_DELAY_OFFSET, &mut delay) {
        return 1;
    }

    emit_ringbuf_event!(
        StatWaitEvent,
        return 0,
        StatWaitEvent {
            kind: EVENT_STAT_WAIT,
            tid: pid,
            delay_ns: delay,
        }
    );

    0
}

#[inline(always)]
fn try_irq_handler_entry(ctx: TracePointContext) -> u32 {
    let mut irq: i32 = 0;
    if !read_i32(&ctx, IRQ_HANDLER_IRQ_OFFSET, &mut irq) {
        return 1;
    }
    if irq < 0 {
        return 0;
    }

    let irq = irq as u32;
    if !is_target_irq(irq) {
        return 0;
    }

    let cpu = unsafe { bpf_get_smp_processor_id() };
    let key = irq_key(irq, cpu);
    let now = unsafe { bpf_ktime_get_ns() };
    if IRQ_START_TIMES.insert(key, now, 0).is_err() {
        increment_drop_counter(DROP_IRQ_START_TIMES_INSERT_FAILED);
    }
    0
}

#[inline(always)]
fn try_irq_handler_exit(ctx: TracePointContext) -> u32 {
    let mut irq: i32 = 0;
    if !read_i32(&ctx, IRQ_HANDLER_IRQ_OFFSET, &mut irq) {
        return 1;
    }
    if irq < 0 {
        return 0;
    }

    let irq = irq as u32;
    if !is_target_irq(irq) {
        return 0;
    }

    let cpu = unsafe { bpf_get_smp_processor_id() };
    let key = irq_key(irq, cpu);
    let enter_ns = match unsafe { IRQ_START_TIMES.get(key) } {
        Some(ts) => *ts,
        None => return 0,
    };
    let _ = IRQ_START_TIMES.remove(key);

    let exit_ns = unsafe { bpf_ktime_get_ns() };
    let duration_ns = exit_ns.saturating_sub(enter_ns);

    emit_ringbuf_event!(
        IrqEvent,
        return 0,
        IrqEvent {
            kind: EVENT_IRQ_LATENCY,
            irq,
            cpu,
            _pad0: 0,
            enter_ns,
            exit_ns,
            duration_ns,
        }
    );
    0
}

// -----------------------------------------------------------------------------
// Block I/O tracing
// -----------------------------------------------------------------------------

#[tracepoint]
/// Tracepoint entry for block_rq_issue.
/// Records target task block I/O start metadata.
pub fn block_rq_issue(ctx: TracePointContext) -> u32 {
    block_io::try_block_rq_issue(ctx)
}

#[tracepoint]
/// Tracepoint entry for block_rq_complete.
/// Emits target task block I/O duration from matching issue metadata.
pub fn block_rq_complete(ctx: TracePointContext) -> u32 {
    block_io::try_block_rq_complete(ctx)
}

// -----------------------------------------------------------------------------
// KMS/flip tracing
// -----------------------------------------------------------------------------

#[tracepoint]
/// Tracepoint entry for i915 flip request.
/// Starts KMS flip interval tracking for i915 tracepoints.
pub fn i915_flip_request(ctx: TracePointContext) -> u32 {
    try_kms_flip_request(
        ctx,
        unsafe { core::ptr::read_volatile(&raw const I915_FLIP_REQUEST_CRTC_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const I915_FLIP_REQUEST_PIPE_OFFSET) },
    )
}

#[tracepoint]
/// Tracepoint entry for i915 flip done.
/// Completes or emits i915 page-flip timing.
pub fn i915_flip_done(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_I915 << 16) | KMS_FLIP_EVENT_PAGEFLIP_DONE,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_PIPE_OFFSET) },
        ),
    )
}

#[tracepoint]
/// Tracepoint entry for DRM flip request.
/// Starts generic DRM flip interval tracking.
pub fn drm_flip_request(ctx: TracePointContext) -> u32 {
    try_kms_flip_request(
        ctx,
        unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_REQUEST_CRTC_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_REQUEST_PIPE_OFFSET) },
    )
}

#[tracepoint]
/// Tracepoint entry for DRM flip done.
/// Completes or emits generic DRM page-flip timing.
pub fn drm_flip_done(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_DRM << 16) | KMS_FLIP_EVENT_PAGEFLIP_DONE,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_PIPE_OFFSET) },
        ),
    )
}

#[tracepoint]
/// Tracepoint entry for DRM vblank events.
/// Emits generic DRM vblank timing when sequence fields are available.
pub fn drm_vblank_event(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_DRM << 16) | KMS_FLIP_EVENT_VBLANK,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_PIPE_OFFSET) },
        ),
    )
}

#[tracepoint]
/// Tracepoint entry for amdgpu flip request.
/// Starts AMDGPU flip interval tracking.
pub fn amdgpu_flip_request(ctx: TracePointContext) -> u32 {
    try_kms_flip_request(
        ctx,
        unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_REQUEST_CRTC_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_REQUEST_PIPE_OFFSET) },
    )
}

#[tracepoint]
/// Tracepoint entry for amdgpu flip done.
/// Completes or emits AMDGPU page-flip timing.
pub fn amdgpu_flip_done(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_AMDGPU << 16) | KMS_FLIP_EVENT_PAGEFLIP_DONE,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_PIPE_OFFSET) },
        ),
    )
}

#[tracepoint]
/// Tracepoint entry for amdgpu vblank events.
/// Emits AMDGPU vblank timing when sequence fields are available.
pub fn amdgpu_vblank_event(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_AMDGPU << 16) | KMS_FLIP_EVENT_VBLANK,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_PIPE_OFFSET) },
        ),
    )
}

#[inline(always)]
fn try_kms_flip_request(ctx: TracePointContext, crtc_offset: u32, pipe_offset: u32) -> u32 {
    let mut key = KmsFlipKey {
        card_minor: 0,
        crtc_id: 0,
        pipe: 0,
    };
    if !fill_kms_flip_key(&mut key, &ctx, crtc_offset, pipe_offset) {
        return 0;
    }

    let now = unsafe { bpf_ktime_get_ns() };
    let _ = KMS_FLIP_STARTS.insert(key, now, 0);

    0
}

#[inline(always)]
fn try_kms_flip_done(
    ctx: TracePointContext,
    provider_and_event_kind: u32,
    offset_pair: u64,
) -> u32 {
    let provider = KmsProvider::from_raw(provider_and_event_kind >> 16);
    let completion_event_kind = provider_and_event_kind & 0xffff;
    let crtc_offset = (offset_pair >> 32) as u32;
    let pipe_offset = offset_pair as u32;
    let now = unsafe { bpf_ktime_get_ns() };

    let mut key = KmsFlipKey {
        card_minor: 0,
        crtc_id: 0,
        pipe: 0,
    };
    if !fill_kms_flip_key(&mut key, &ctx, crtc_offset, pipe_offset) {
        return 0;
    }

    let mut start_ns = 0;
    let has_start_ns = match unsafe { KMS_FLIP_STARTS.get(key) } {
        Some(value) => {
            start_ns = *value;
            true
        }
        None => false,
    };
    let _ = KMS_FLIP_STARTS.remove(key);

    let completion_event = KmsCompletionEvent::from_raw(completion_event_kind);
    let mut sequence = 0;
    let mut sequence_offset = 0;
    let mut sequence_size = 0;
    let has_sequence =
        kms_sequence_offsets(
            provider,
            completion_event,
            &mut sequence_offset,
            &mut sequence_size,
        ) && read_sequence_field(&ctx, sequence_offset, sequence_size, &mut sequence);

    emit_kms_flip_event(
        &key,
        provider_and_event_kind,
        has_sequence,
        sequence,
        has_start_ns,
        start_ns,
        now,
    );

    0
}

fn kms_offset_pair(crtc_offset: u32, pipe_offset: u32) -> u64 {
    ((crtc_offset as u64) << 32) | (pipe_offset as u64)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum KmsProvider {
    I915,
    Drm,
    Amdgpu,
    Unknown,
}

impl KmsProvider {
    #[inline(always)]
    fn from_raw(provider: u32) -> Self {
        if provider == KMS_FLIP_PROVIDER_I915 {
            Self::I915
        } else if provider == KMS_FLIP_PROVIDER_DRM {
            Self::Drm
        } else if provider == KMS_FLIP_PROVIDER_AMDGPU {
            Self::Amdgpu
        } else {
            Self::Unknown
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum KmsCompletionEvent {
    PageflipDone,
    Vblank,
    Unknown,
}

impl KmsCompletionEvent {
    #[inline(always)]
    fn from_raw(completion_event_kind: u32) -> Self {
        if completion_event_kind == KMS_FLIP_EVENT_PAGEFLIP_DONE {
            Self::PageflipDone
        } else if completion_event_kind == KMS_FLIP_EVENT_VBLANK {
            Self::Vblank
        } else {
            Self::Unknown
        }
    }
}

#[inline(always)]
fn kms_sequence_offsets(
    provider: KmsProvider,
    completion_event: KmsCompletionEvent,
    sequence_offset: &mut u32,
    sequence_size: &mut u32,
) -> bool {
    match (provider, completion_event) {
        (KmsProvider::I915, KmsCompletionEvent::PageflipDone) => {
            *sequence_offset =
                unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_SEQUENCE_OFFSET) };
            *sequence_size =
                unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_SEQUENCE_SIZE) };
            true
        }
        (KmsProvider::Drm, KmsCompletionEvent::Vblank) => {
            *sequence_offset =
                unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_SEQUENCE_OFFSET) };
            *sequence_size =
                unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_SEQUENCE_SIZE) };
            true
        }
        (KmsProvider::Drm, KmsCompletionEvent::PageflipDone) => {
            *sequence_offset =
                unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_SEQUENCE_OFFSET) };
            *sequence_size =
                unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_SEQUENCE_SIZE) };
            true
        }
        (KmsProvider::Amdgpu, KmsCompletionEvent::Vblank) => {
            *sequence_offset =
                unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_SEQUENCE_OFFSET) };
            *sequence_size =
                unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_SEQUENCE_SIZE) };
            true
        }
        (KmsProvider::Amdgpu, KmsCompletionEvent::PageflipDone) => {
            *sequence_offset =
                unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_SEQUENCE_OFFSET) };
            *sequence_size =
                unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_SEQUENCE_SIZE) };
            true
        }
        (KmsProvider::I915, KmsCompletionEvent::Vblank)
        | (KmsProvider::I915, KmsCompletionEvent::Unknown)
        | (KmsProvider::Drm, KmsCompletionEvent::Unknown)
        | (KmsProvider::Amdgpu, KmsCompletionEvent::Unknown)
        | (KmsProvider::Unknown, KmsCompletionEvent::PageflipDone)
        | (KmsProvider::Unknown, KmsCompletionEvent::Vblank)
        | (KmsProvider::Unknown, KmsCompletionEvent::Unknown) => false,
    }
}

#[inline(always)]
fn fill_kms_flip_key(
    key: &mut KmsFlipKey,
    ctx: &TracePointContext,
    crtc_offset: u32,
    pipe_offset: u32,
) -> bool {
    let mut crtc_id = 0;
    let mut pipe = 0;
    let _ = read_optional_u32(ctx, crtc_offset, &mut crtc_id);
    let _ = read_optional_u32(ctx, pipe_offset, &mut pipe);

    if crtc_id == 0 && pipe == 0 {
        return false;
    }

    key.card_minor = 0;
    key.crtc_id = crtc_id;
    key.pipe = pipe;
    true
}

// -----------------------------------------------------------------------------
// DRM fence waits/signals
// -----------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
struct FenceIdentity {
    key: FenceKey,
    flags: u32,
    timeline_hash: u64,
}

#[tracepoint]
/// Tracepoint entry for DRM fence wait start.
/// Stores fence wait start identity and task metadata.
pub fn drm_fence_wait_start(ctx: TracePointContext) -> u32 {
    try_drm_fence_wait_start(ctx)
}

#[inline(always)]
fn try_drm_fence_wait_start(ctx: TracePointContext) -> u32 {
    let mut identity = FenceIdentity {
        key: FenceKey {
            context: 0,
            seqno: 0,
        },
        flags: 0,
        timeline_hash: 0,
    };
    if !fill_fence_identity(
        &mut identity,
        &ctx,
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_CONTEXT_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_SEQNO_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_TIMELINE_OFFSET) },
    ) {
        return 0;
    }

    let pid_tgid = bpf_get_current_pid_tgid();
    let start = FenceWaitStart {
        ts: unsafe { bpf_ktime_get_ns() },
        pid: (pid_tgid >> 32) as u32,
        tid: (pid_tgid & 0xffff_ffff) as u32,
        provider: unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_PROVIDER) },
        gpu_role: unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_GPU_ROLE) },
    };
    let _ = FENCE_WAIT_STARTS.insert(identity.key, start, 0);

    0
}

#[tracepoint]
/// Tracepoint entry for DRM fence wait done.
/// Emits fence wait intervals and importer/exporter correlation when available.
pub fn drm_fence_wait_done(ctx: TracePointContext) -> u32 {
    try_drm_fence_wait_done(ctx)
}

#[inline(always)]
fn try_drm_fence_wait_done(ctx: TracePointContext) -> u32 {
    let mut identity = FenceIdentity {
        key: FenceKey {
            context: 0,
            seqno: 0,
        },
        flags: 0,
        timeline_hash: 0,
    };
    if !fill_fence_identity(
        &mut identity,
        &ctx,
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_DONE_CONTEXT_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_DONE_SEQNO_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_DONE_TIMELINE_OFFSET) },
    ) {
        return 0;
    }

    let now = unsafe { bpf_ktime_get_ns() };
    let start = unsafe { FENCE_WAIT_STARTS.get(identity.key).copied() };
    let _ = FENCE_WAIT_STARTS.remove(identity.key);
    let signal = unsafe { FENCE_SIGNAL_TIMES.get(identity.key).copied() };
    let _ = FENCE_SIGNAL_TIMES.remove(identity.key);

    let (
        event_kind,
        wait_start_ns,
        duration_ns,
        pid,
        tid,
        provider,
        gpu_role,
        signal_ns,
        driver_id,
        extra_flags,
    ) = match start {
        Some(start) => (
            DRM_FENCE_EVENT_WAIT_INTERVAL,
            start.ts,
            now.saturating_sub(start.ts),
            start.pid,
            start.tid,
            start.provider,
            start.gpu_role,
            signal.map(|signal| signal.ts).unwrap_or(0),
            signal
                .map(|signal| signal.provider)
                .unwrap_or(start.provider),
            DRM_FENCE_HAS_DURATION
                | DRM_FENCE_HAS_PID
                | DRM_FENCE_IS_IMPORTER_SIDE
                | if signal.is_some() {
                    DRM_FENCE_IS_EXPORTER_SIDE
                } else {
                    0
                },
        ),
        None => (
            DRM_FENCE_EVENT_WAIT_DONE,
            0,
            0,
            0,
            0,
            unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_DONE_PROVIDER) },
            unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_DONE_GPU_ROLE) },
            signal.map(|signal| signal.ts).unwrap_or(0),
            signal
                .map(|signal| signal.provider)
                .unwrap_or_else(|| unsafe {
                    core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_DONE_PROVIDER)
                }),
            DRM_FENCE_IS_IMPORTER_SIDE
                | if signal.is_some() {
                    DRM_FENCE_IS_EXPORTER_SIDE
                } else {
                    0
                },
        ),
    };

    emit_ringbuf_event!(
        DrmFenceEvent,
        return 0,
        DrmFenceEvent {
            kind: EVENT_DRM_FENCE,
            event_kind,
            provider,
            flags: identity.flags | extra_flags,
            pid,
            tid,
            cpu: bpf_get_smp_processor_id() as u32,
            driver_id,
            gpu_role,
            _pad0: 0,
            context: identity.key.context,
            seqno: identity.key.seqno,
            timeline_hash: identity.timeline_hash,
            wait_start_ns,
            wait_done_ns: now,
            signal_ns,
            duration_ns,
            timestamp_ns: now,
        }
    );
    0
}

#[tracepoint]
/// Tracepoint entry for DRM fence signal.
/// Emits exporter-side fence signals and caches signal timestamps for waits.
pub fn drm_fence_signal(ctx: TracePointContext) -> u32 {
    try_drm_fence_signal(ctx)
}

#[inline(always)]
fn try_drm_fence_signal(ctx: TracePointContext) -> u32 {
    let mut identity = FenceIdentity {
        key: FenceKey {
            context: 0,
            seqno: 0,
        },
        flags: 0,
        timeline_hash: 0,
    };
    if !fill_fence_identity(
        &mut identity,
        &ctx,
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_SIGNAL_CONTEXT_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_SIGNAL_SEQNO_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_SIGNAL_TIMELINE_OFFSET) },
    ) {
        return 0;
    }

    let now = unsafe { bpf_ktime_get_ns() };
    let provider = unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_SIGNAL_PROVIDER) };
    let gpu_role = unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_SIGNAL_GPU_ROLE) };
    let _ = FENCE_SIGNAL_TIMES.insert(
        identity.key,
        FenceSignal {
            ts: now,
            provider,
            gpu_role,
        },
        0,
    );

    emit_ringbuf_event!(
        DrmFenceEvent,
        return 0,
        DrmFenceEvent {
            kind: EVENT_DRM_FENCE,
            event_kind: DRM_FENCE_EVENT_SIGNAL,
            provider,
            flags: identity.flags | DRM_FENCE_IS_EXPORTER_SIDE,
            pid: 0,
            tid: 0,
            cpu: bpf_get_smp_processor_id() as u32,
            driver_id: provider,
            gpu_role,
            _pad0: 0,
            context: identity.key.context,
            seqno: identity.key.seqno,
            timeline_hash: identity.timeline_hash,
            wait_start_ns: 0,
            wait_done_ns: 0,
            signal_ns: now,
            duration_ns: 0,
            timestamp_ns: now,
        }
    );

    0
}

#[inline(always)]
fn fill_fence_identity(
    identity: &mut FenceIdentity,
    ctx: &TracePointContext,
    context_offset: u32,
    seqno_offset: u32,
    timeline_offset: u32,
) -> bool {
    let mut context = 0;
    let has_context = read_optional_u64(ctx, context_offset, &mut context);

    let mut seqno = 0;
    if !read_optional_u64(ctx, seqno_offset, &mut seqno) {
        return false;
    }

    let mut timeline_hash = 0;
    let has_timeline =
        read_optional_u64(ctx, timeline_offset, &mut timeline_hash) && timeline_hash != 0;

    let key_context = if has_context {
        context
    } else if has_timeline {
        timeline_hash
    } else {
        return false;
    };

    let mut flags = DRM_FENCE_HAS_SEQNO;
    if has_context {
        flags |= DRM_FENCE_HAS_CONTEXT;
    }
    if has_timeline {
        flags |= DRM_FENCE_HAS_TIMELINE;
    }

    identity.key.context = key_context;
    identity.key.seqno = seqno;
    identity.flags = flags;
    identity.timeline_hash = if has_timeline { timeline_hash } else { 0 };
    true
}

// -----------------------------------------------------------------------------
// License and panic handler
// -----------------------------------------------------------------------------

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";

#[cfg(all(not(test), target_arch = "bpf"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
