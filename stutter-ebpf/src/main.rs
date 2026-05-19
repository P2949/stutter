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
    BlockIoEvent, CpuFreqEvent, DRM_FENCE_EVENT_SIGNAL, DRM_FENCE_EVENT_WAIT_DONE,
    DRM_FENCE_EVENT_WAIT_INTERVAL, DRM_FENCE_HAS_CONTEXT, DRM_FENCE_HAS_DURATION,
    DRM_FENCE_HAS_PID, DRM_FENCE_HAS_SEQNO, DRM_FENCE_HAS_TIMELINE, DRM_FENCE_IS_EXPORTER_SIDE,
    DRM_FENCE_IS_IMPORTER_SIDE, DRM_FENCE_PROVIDER_DMA_FENCE, DRM_GPU_ROLE_UNKNOWN,
    DROP_BLOCK_START_INSERT_FAILED, DROP_COUNTERS_MAX, DROP_IRQ_START_TIMES_INSERT_FAILED,
    DROP_RINGBUF_RESERVE_FAILED, DROP_WAKEUP_DATA_INSERT_FAILED, DROP_WAKEUP_DATA_STALE_ENTRY,
    DrmFenceEvent, EVENT_BLOCK_IO, EVENT_CPU_FREQ, EVENT_DRM_FENCE, EVENT_EXEC, EVENT_IRQ_LATENCY,
    EVENT_KMS_FLIP, EVENT_MIGRATION, EVENT_RUNNABLE_LATENCY, EVENT_STAT_WAIT, ExecEvent, IrqEvent,
    KMS_FLIP_EVENT_INTERVAL, KMS_FLIP_EVENT_PAGEFLIP_DONE, KMS_FLIP_EVENT_VBLANK,
    KMS_FLIP_HAS_CRTC, KMS_FLIP_HAS_DONE_NS, KMS_FLIP_HAS_DURATION_NS, KMS_FLIP_HAS_REQUEST_NS,
    KMS_FLIP_HAS_SEQUENCE, KMS_FLIP_PROVIDER_AMDGPU, KMS_FLIP_PROVIDER_DRM, KMS_FLIP_PROVIDER_I915,
    KmsFlipEvent, MigrationEvent, SchedulerEvent, StatWaitEvent,
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

#[unsafe(no_mangle)]
static mut I915_FLIP_REQUEST_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut I915_FLIP_REQUEST_PIPE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut I915_FLIP_DONE_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut I915_FLIP_DONE_PIPE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut I915_FLIP_DONE_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut I915_FLIP_DONE_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FLIP_REQUEST_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FLIP_REQUEST_PIPE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FLIP_DONE_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FLIP_DONE_PIPE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FLIP_DONE_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FLIP_DONE_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_VBLANK_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_VBLANK_PIPE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_VBLANK_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_VBLANK_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_FLIP_REQUEST_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_FLIP_REQUEST_PIPE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_FLIP_DONE_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_FLIP_DONE_PIPE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_FLIP_DONE_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_FLIP_DONE_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_VBLANK_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_VBLANK_PIPE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_VBLANK_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut AMDGPU_VBLANK_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_START_CONTEXT_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_START_SEQNO_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_START_TIMELINE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_DONE_CONTEXT_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_DONE_SEQNO_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_DONE_TIMELINE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_SIGNAL_CONTEXT_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_SIGNAL_SEQNO_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_SIGNAL_TIMELINE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_START_PROVIDER: u32 = DRM_FENCE_PROVIDER_DMA_FENCE;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_START_GPU_ROLE: u32 = DRM_GPU_ROLE_UNKNOWN;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_DONE_PROVIDER: u32 = DRM_FENCE_PROVIDER_DMA_FENCE;

#[unsafe(no_mangle)]
static mut DRM_FENCE_WAIT_DONE_GPU_ROLE: u32 = DRM_GPU_ROLE_UNKNOWN;

#[unsafe(no_mangle)]
static mut DRM_FENCE_SIGNAL_PROVIDER: u32 = DRM_FENCE_PROVIDER_DMA_FENCE;

#[unsafe(no_mangle)]
static mut DRM_FENCE_SIGNAL_GPU_ROLE: u32 = DRM_GPU_ROLE_UNKNOWN;

const TARGET_PIDS_MAP_MAX_ENTRIES: u32 = 1_024;
const WAKEUP_DATA_MAP_MAX_ENTRIES: u32 = 131_072;
const RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES: u32 = 65_536;
const PREV_FAULTS_MAP_MAX_ENTRIES: u32 = 131_072;

const _: () = assert!(WAKEUP_DATA_MAP_MAX_ENTRIES >= TARGET_PIDS_MAP_MAX_ENTRIES * 128);
const _: () = assert!(PREV_FAULTS_MAP_MAX_ENTRIES >= TARGET_PIDS_MAP_MAX_ENTRIES * 128);
const _: () = assert!(RUNNABLE_TASK_CPU_MAP_MAX_ENTRIES >= TARGET_PIDS_MAP_MAX_ENTRIES * 64);

// Capacity model:
// TARGET_PIDS is the primary target gatekeeper. The state maps below are
// intentionally much larger than TARGET_PIDS so wakeup timestamps,
// runnable-task CPU state, and fault counters do not become the first
// bottleneck under normal monitored-task churn.
// Do not reduce WAKEUP_DATA, RUNNABLE_TASK_CPU, or PREV_FAULTS toward
// TARGET_PIDS without updating these capacity invariants and the userspace
// diagnostics that report target and wakeup-map capacity.

#[map]
// Userspace overrides this before loading the BPF object based on the current
// memlock limit and available memory. The value here is only a safe fallback.
static EVENTS: RingBuf = RingBuf::with_byte_size(256 * 1024, 0);

#[map]
static TARGET_PIDS: HashMap<u32, u8> =
    HashMap::<u32, u8>::with_max_entries(TARGET_PIDS_MAP_MAX_ENTRIES, 0);

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
    HashMap::<u32, WakeupData>::with_max_entries(WAKEUP_DATA_MAP_MAX_ENTRIES, 0);

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
struct IoStart {
    ts: u64,
    tid: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KmsFlipKey {
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
static BLOCK_START: LruHashMap<u64, IoStart> =
    LruHashMap::<u64, IoStart>::with_max_entries(16384, 0);

#[map]
static KMS_FLIP_STARTS: HashMap<KmsFlipKey, u64> =
    HashMap::<KmsFlipKey, u64>::with_max_entries(4096, 0);

#[map]
static FENCE_WAIT_STARTS: HashMap<FenceKey, FenceWaitStart> =
    HashMap::<FenceKey, FenceWaitStart>::with_max_entries(4096, 0);

#[map]
static FENCE_SIGNAL_TIMES: HashMap<FenceKey, FenceSignal> =
    HashMap::<FenceKey, FenceSignal>::with_max_entries(4096, 0);

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

    if !is_target_pid(pid) {
        increment_drop_counter(DROP_WAKEUP_DATA_STALE_ENTRY);
        let _ = WAKEUP_DATA.remove(pid);
        decrement_target_pending(wakeup_data.target_cpu);
        remove_runnable_task_if_present(pid);
        return Ok(0);
    }

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
        (*event).tid = pid;
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

#[inline(always)]
fn block_rq_fallback_key(
    ctx: &TracePointContext,
    sector: u64,
    dev: u32,
    nr_sector_offset: u32,
    rwbs_offset: u32,
) -> u64 {
    // Use a 64-bit mixed key of (sector, dev, nr_sector, rwbs) to minimize
    // collisions during the fallback correlation mode.
    let mut h = sector.wrapping_mul(11400714819323198485u64)
        ^ ((dev as u64).wrapping_mul(14029467366897019727u64));

    if nr_sector_offset != 0 {
        let nr_sector: u32 = unsafe { ctx.read_at(nr_sector_offset as usize).unwrap_or(0) };
        h ^= (nr_sector as u64).wrapping_mul(11500714819323198485u64);
    }

    if rwbs_offset != 0 {
        let rwbs: u64 = unsafe { ctx.read_at(rwbs_offset as usize).unwrap_or(0) };
        h ^= rwbs.wrapping_mul(11600714819323198485u64);
    }

    h
}

#[tracepoint]
pub fn block_rq_issue(ctx: TracePointContext) -> u32 {
    match try_block_rq_issue(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn i915_flip_request(ctx: TracePointContext) -> u32 {
    match try_kms_flip_request(
        ctx,
        unsafe { core::ptr::read_volatile(&raw const I915_FLIP_REQUEST_CRTC_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const I915_FLIP_REQUEST_PIPE_OFFSET) },
    ) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn i915_flip_done(ctx: TracePointContext) -> u32 {
    match try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_I915 << 16) | KMS_FLIP_EVENT_PAGEFLIP_DONE,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_PIPE_OFFSET) },
        ),
    ) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn drm_flip_request(ctx: TracePointContext) -> u32 {
    match try_kms_flip_request(
        ctx,
        unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_REQUEST_CRTC_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_REQUEST_PIPE_OFFSET) },
    ) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn drm_flip_done(ctx: TracePointContext) -> u32 {
    match try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_DRM << 16) | KMS_FLIP_EVENT_PAGEFLIP_DONE,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_PIPE_OFFSET) },
        ),
    ) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn drm_vblank_event(ctx: TracePointContext) -> u32 {
    match try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_DRM << 16) | KMS_FLIP_EVENT_VBLANK,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_PIPE_OFFSET) },
        ),
    ) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn amdgpu_flip_request(ctx: TracePointContext) -> u32 {
    match try_kms_flip_request(
        ctx,
        unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_REQUEST_CRTC_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_REQUEST_PIPE_OFFSET) },
    ) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn amdgpu_flip_done(ctx: TracePointContext) -> u32 {
    match try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_AMDGPU << 16) | KMS_FLIP_EVENT_PAGEFLIP_DONE,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_PIPE_OFFSET) },
        ),
    ) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

#[tracepoint]
pub fn amdgpu_vblank_event(ctx: TracePointContext) -> u32 {
    match try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_AMDGPU << 16) | KMS_FLIP_EVENT_VBLANK,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_PIPE_OFFSET) },
        ),
    ) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

fn try_kms_flip_request(
    ctx: TracePointContext,
    crtc_offset: u32,
    pipe_offset: u32,
) -> Result<u32, u32> {
    let Some(key) = kms_flip_key(&ctx, crtc_offset, pipe_offset) else {
        return Ok(0);
    };

    let now = unsafe { bpf_ktime_get_ns() };
    let _ = KMS_FLIP_STARTS.insert(key, now, 0);

    Ok(0)
}

fn try_kms_flip_done(
    ctx: TracePointContext,
    provider_and_event_kind: u32,
    offset_pair: u64,
) -> Result<u32, u32> {
    let provider = provider_and_event_kind >> 16;
    let completion_event_kind = provider_and_event_kind & 0xffff;
    let crtc_offset = (offset_pair >> 32) as u32;
    let pipe_offset = offset_pair as u32;
    let now = unsafe { bpf_ktime_get_ns() };
    let Some(key) = kms_flip_key(&ctx, crtc_offset, pipe_offset) else {
        return Ok(0);
    };

    let start_ns = unsafe { KMS_FLIP_STARTS.get(key).copied() };
    let _ = KMS_FLIP_STARTS.remove(key);
    let (sequence_offset, sequence_size) = kms_sequence_offsets(provider, completion_event_kind);
    let sequence = read_sequence_field(&ctx, sequence_offset, sequence_size);

    emit_kms_flip_event(&key, provider_and_event_kind, sequence, start_ns, now);

    Ok(0)
}

fn kms_offset_pair(crtc_offset: u32, pipe_offset: u32) -> u64 {
    ((crtc_offset as u64) << 32) | (pipe_offset as u64)
}

fn kms_sequence_offsets(provider: u32, completion_event_kind: u32) -> (u32, u32) {
    if provider == KMS_FLIP_PROVIDER_I915 {
        return (
            unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_SEQUENCE_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_SEQUENCE_SIZE) },
        );
    }

    if provider == KMS_FLIP_PROVIDER_DRM {
        if completion_event_kind == KMS_FLIP_EVENT_VBLANK {
            return (
                unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_SEQUENCE_OFFSET) },
                unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_SEQUENCE_SIZE) },
            );
        }
        return (
            unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_SEQUENCE_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_SEQUENCE_SIZE) },
        );
    }

    if completion_event_kind == KMS_FLIP_EVENT_VBLANK {
        (
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_SEQUENCE_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_SEQUENCE_SIZE) },
        )
    } else {
        (
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_SEQUENCE_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_SEQUENCE_SIZE) },
        )
    }
}

fn kms_flip_key(ctx: &TracePointContext, crtc_offset: u32, pipe_offset: u32) -> Option<KmsFlipKey> {
    let crtc_id = read_optional_u32(ctx, crtc_offset).unwrap_or(0);
    let pipe = read_optional_u32(ctx, pipe_offset).unwrap_or(0);

    if crtc_id == 0 && pipe == 0 {
        None
    } else {
        Some(KmsFlipKey {
            card_minor: 0,
            crtc_id,
            pipe,
        })
    }
}

fn read_optional_u32(ctx: &TracePointContext, offset: u32) -> Option<u32> {
    if offset == 0 {
        None
    } else {
        unsafe { ctx.read_at::<u32>(offset as usize).ok() }
    }
}

fn read_sequence_field(ctx: &TracePointContext, offset: u32, size: u32) -> Option<u64> {
    if offset == 0 {
        None
    } else if size >= 8 {
        unsafe { ctx.read_at::<u64>(offset as usize).ok() }
    } else {
        unsafe {
            ctx.read_at::<u32>(offset as usize)
                .ok()
                .map(|value| value as u64)
        }
    }
}

fn emit_kms_flip_event(
    key: &KmsFlipKey,
    provider_and_event_kind: u32,
    sequence: Option<u64>,
    start_ns: Option<u64>,
    done_ns: u64,
) {
    let Some(mut entry) = EVENTS.reserve::<KmsFlipEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return;
    };

    let event = entry.as_mut_ptr();
    let duration_ns = start_ns
        .map(|start| done_ns.saturating_sub(start))
        .unwrap_or(0);

    let mut flags = KMS_FLIP_HAS_DONE_NS;
    if key.crtc_id != 0 {
        flags |= KMS_FLIP_HAS_CRTC;
    }
    if start_ns.is_some() {
        flags |= KMS_FLIP_HAS_REQUEST_NS | KMS_FLIP_HAS_DURATION_NS;
    }
    if sequence.is_some() {
        flags |= KMS_FLIP_HAS_SEQUENCE;
    }

    unsafe {
        (*event).kind = EVENT_KMS_FLIP;
        let provider = provider_and_event_kind >> 16;
        let completion_event_kind = provider_and_event_kind & 0xffff;
        (*event).event_kind = if start_ns.is_some() {
            KMS_FLIP_EVENT_INTERVAL
        } else {
            completion_event_kind
        };
        (*event).provider = provider;
        (*event).flags = flags;
        let pid_tgid = bpf_get_current_pid_tgid();
        (*event).pid = (pid_tgid >> 32) as u32;
        (*event).tid = (pid_tgid & 0xffff_ffff) as u32;
        (*event).cpu = bpf_get_smp_processor_id() as u32;
        (*event).card_minor = key.card_minor;
        (*event).crtc_id = key.crtc_id;
        (*event).pipe = key.pipe;
        (*event).sequence = sequence.unwrap_or(0);
        (*event).request_ns = start_ns.unwrap_or(0);
        (*event).done_ns = done_ns;
        (*event).duration_ns = duration_ns;
        (*event).timestamp_ns = done_ns;
    }

    entry.submit(0);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct FenceIdentity {
    key: FenceKey,
    flags: u32,
    timeline_hash: u64,
}

#[tracepoint]
pub fn drm_fence_wait_start(ctx: TracePointContext) -> u32 {
    match try_drm_fence_wait_start(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

fn try_drm_fence_wait_start(ctx: TracePointContext) -> Result<u32, u32> {
    let Some(identity) = fence_identity(
        &ctx,
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_CONTEXT_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_SEQNO_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_TIMELINE_OFFSET) },
    ) else {
        return Ok(0);
    };

    let pid_tgid = bpf_get_current_pid_tgid();
    let start = FenceWaitStart {
        ts: unsafe { bpf_ktime_get_ns() },
        pid: (pid_tgid >> 32) as u32,
        tid: (pid_tgid & 0xffff_ffff) as u32,
        provider: unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_PROVIDER) },
        gpu_role: unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_START_GPU_ROLE) },
    };
    let _ = FENCE_WAIT_STARTS.insert(identity.key, start, 0);

    Ok(0)
}

#[tracepoint]
pub fn drm_fence_wait_done(ctx: TracePointContext) -> u32 {
    match try_drm_fence_wait_done(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

fn try_drm_fence_wait_done(ctx: TracePointContext) -> Result<u32, u32> {
    let Some(identity) = fence_identity(
        &ctx,
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_DONE_CONTEXT_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_DONE_SEQNO_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_WAIT_DONE_TIMELINE_OFFSET) },
    ) else {
        return Ok(0);
    };

    let now = unsafe { bpf_ktime_get_ns() };
    let start = unsafe { FENCE_WAIT_STARTS.get(identity.key).copied() };
    let _ = FENCE_WAIT_STARTS.remove(identity.key);
    let signal = unsafe { FENCE_SIGNAL_TIMES.get(identity.key).copied() };
    let _ = FENCE_SIGNAL_TIMES.remove(identity.key);

    let Some(mut entry) = EVENTS.reserve::<DrmFenceEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return Ok(0);
    };
    let event = entry.as_mut_ptr();

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

    unsafe {
        (*event).kind = EVENT_DRM_FENCE;
        (*event).event_kind = event_kind;
        (*event).provider = provider;
        (*event).flags = identity.flags | extra_flags;
        (*event).pid = pid;
        (*event).tid = tid;
        (*event).cpu = bpf_get_smp_processor_id() as u32;
        (*event).driver_id = driver_id;
        (*event).gpu_role = gpu_role;
        (*event).context = identity.key.context;
        (*event).seqno = identity.key.seqno;
        (*event).timeline_hash = identity.timeline_hash;
        (*event).wait_start_ns = wait_start_ns;
        (*event).wait_done_ns = now;
        (*event).signal_ns = signal_ns;
        (*event).duration_ns = duration_ns;
        (*event).timestamp_ns = now;
    }

    entry.submit(0);
    Ok(0)
}

#[tracepoint]
pub fn drm_fence_signal(ctx: TracePointContext) -> u32 {
    match try_drm_fence_signal(ctx) {
        Ok(ret) => ret,
        Err(ret) => ret,
    }
}

fn try_drm_fence_signal(ctx: TracePointContext) -> Result<u32, u32> {
    let Some(identity) = fence_identity(
        &ctx,
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_SIGNAL_CONTEXT_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_SIGNAL_SEQNO_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FENCE_SIGNAL_TIMELINE_OFFSET) },
    ) else {
        return Ok(0);
    };

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

    let Some(mut entry) = EVENTS.reserve::<DrmFenceEvent>(0) else {
        increment_drop_counter(DROP_RINGBUF_RESERVE_FAILED);
        return Ok(0);
    };
    let event = entry.as_mut_ptr();
    unsafe {
        (*event).kind = EVENT_DRM_FENCE;
        (*event).event_kind = DRM_FENCE_EVENT_SIGNAL;
        (*event).provider = provider;
        (*event).flags = identity.flags | DRM_FENCE_IS_EXPORTER_SIDE;
        (*event).pid = 0;
        (*event).tid = 0;
        (*event).cpu = bpf_get_smp_processor_id() as u32;
        (*event).driver_id = provider;
        (*event).gpu_role = gpu_role;
        (*event).context = identity.key.context;
        (*event).seqno = identity.key.seqno;
        (*event).timeline_hash = identity.timeline_hash;
        (*event).wait_start_ns = 0;
        (*event).wait_done_ns = 0;
        (*event).signal_ns = now;
        (*event).duration_ns = 0;
        (*event).timestamp_ns = now;
    }
    entry.submit(0);

    Ok(0)
}

fn fence_identity(
    ctx: &TracePointContext,
    context_offset: u32,
    seqno_offset: u32,
    timeline_offset: u32,
) -> Option<FenceIdentity> {
    let context = read_optional_u64(ctx, context_offset);
    let seqno = read_optional_u64(ctx, seqno_offset);
    let timeline_hash = read_optional_u64(ctx, timeline_offset).unwrap_or(0);
    let key_context = context.or((timeline_hash != 0).then_some(timeline_hash))?;
    let key_seqno = seqno?;

    let mut flags = DRM_FENCE_HAS_SEQNO;
    if context.is_some() {
        flags |= DRM_FENCE_HAS_CONTEXT;
    }
    if timeline_hash != 0 {
        flags |= DRM_FENCE_HAS_TIMELINE;
    }

    Some(FenceIdentity {
        key: FenceKey {
            context: key_context,
            seqno: key_seqno,
        },
        flags,
        timeline_hash,
    })
}

fn read_optional_u64(ctx: &TracePointContext, offset: u32) -> Option<u64> {
    if offset == 0 {
        None
    } else {
        unsafe { ctx.read_at::<u64>(offset as usize).ok() }
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
        let nr_sector_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_ISSUE_NR_SECTOR_OFFSET) };
        let rwbs_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_ISSUE_RWBS_OFFSET) };

        block_rq_fallback_key(&ctx, sector, dev, nr_sector_offset, rwbs_offset)
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
        let nr_sector_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET) };
        let rwbs_offset =
            unsafe { core::ptr::read_volatile(&raw const BLOCK_RQ_COMPLETE_RWBS_OFFSET) };

        block_rq_fallback_key(&ctx, sector, dev, nr_sector_offset, rwbs_offset)
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
