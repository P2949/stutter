#![no_std]
#![no_main]

use aya_ebpf::{macros::tracepoint, programs::TracePointContext};
use stutter_common::BPF_MAX_TRACKED_CPUS;

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
mod drop_counters;
mod kms_emit;
mod map_limits;
mod trace_offsets;
mod trace_read;
mod wakeup_data;

mod process_lifecycle;
mod runnable_depth;
use process_lifecycle::*;
mod cpu_frequency;
use cpu_frequency::*;
use drop_counters::increment_drop_counter;
mod irq;
use irq::*;
mod scheduler;
use scheduler::*;
mod target_filter;

mod maps;
use maps::*;
mod drm_fence;
mod kms;

// -----------------------------------------------------------------------------
// Shared constants and map sizing
// -----------------------------------------------------------------------------

const _: () = assert!(BPF_MAX_TRACKED_CPUS >= 1024);

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
///
/// Runnable-latency measurement treats sched_wakeup and sched_wakeup_new the
/// same way because both mean the wakee became runnable. The "new task"
/// distinction is useful for coverage diagnostics, but it does not change the
/// wakeup-to-switch latency calculation or target-local runnable accounting.
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
pub fn sched_process_exit(ctx: TracePointContext) -> u32 {
    try_sched_process_exit(ctx)
}

// -----------------------------------------------------------------------------
// Fault counters
// -----------------------------------------------------------------------------

#[aya_ebpf::macros::perf_event]
/// Perf-event entry for major page faults.
/// Increments per-target fault counters used on later scheduler events.
pub fn major_fault(ctx: aya_ebpf::programs::PerfEventContext) -> u32 {
    try_major_fault(ctx)
}

#[aya_ebpf::macros::perf_event]
/// Perf-event entry for minor page faults.
/// Increments per-target fault counters used on later scheduler events.
pub fn minor_fault(ctx: aya_ebpf::programs::PerfEventContext) -> u32 {
    try_minor_fault(ctx)
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
    kms::try_i915_flip_request(ctx)
}

#[tracepoint]
/// Tracepoint entry for i915 flip done.
/// Completes or emits i915 page-flip timing.
pub fn i915_flip_done(ctx: TracePointContext) -> u32 {
    kms::try_i915_flip_done(ctx)
}

#[tracepoint]
/// Tracepoint entry for DRM flip request.
/// Starts generic DRM flip interval tracking.
pub fn drm_flip_request(ctx: TracePointContext) -> u32 {
    kms::try_drm_flip_request(ctx)
}

#[tracepoint]
/// Tracepoint entry for DRM flip done.
/// Completes or emits generic DRM page-flip timing.
pub fn drm_flip_done(ctx: TracePointContext) -> u32 {
    kms::try_drm_flip_done(ctx)
}

#[tracepoint]
/// Tracepoint entry for DRM vblank events.
/// Emits generic DRM vblank timing when sequence fields are available.
pub fn drm_vblank_event(ctx: TracePointContext) -> u32 {
    kms::try_drm_vblank_event(ctx)
}

#[tracepoint]
/// Tracepoint entry for amdgpu flip request.
/// Starts AMDGPU flip interval tracking.
pub fn amdgpu_flip_request(ctx: TracePointContext) -> u32 {
    kms::try_amdgpu_flip_request(ctx)
}

#[tracepoint]
/// Tracepoint entry for amdgpu flip done.
/// Completes or emits AMDGPU page-flip timing.
pub fn amdgpu_flip_done(ctx: TracePointContext) -> u32 {
    kms::try_amdgpu_flip_done(ctx)
}

#[tracepoint]
/// Tracepoint entry for amdgpu vblank events.
/// Emits AMDGPU vblank timing when sequence fields are available.
pub fn amdgpu_vblank_event(ctx: TracePointContext) -> u32 {
    kms::try_amdgpu_vblank_event(ctx)
}

// -----------------------------------------------------------------------------
// DRM fence waits/signals
// -----------------------------------------------------------------------------

#[tracepoint]
/// Tracepoint entry for DRM fence wait start.
/// Stores fence wait start identity and task metadata.
pub fn drm_fence_wait_start(ctx: TracePointContext) -> u32 {
    drm_fence::try_drm_fence_wait_start(ctx)
}

#[tracepoint]
/// Tracepoint entry for DRM fence wait done.
/// Emits fence wait intervals and importer/exporter correlation when available.
pub fn drm_fence_wait_done(ctx: TracePointContext) -> u32 {
    drm_fence::try_drm_fence_wait_done(ctx)
}

#[tracepoint]
/// Tracepoint entry for DRM fence signal.
/// Emits exporter-side fence signals and caches signal timestamps for waits.
pub fn drm_fence_signal(ctx: TracePointContext) -> u32 {
    drm_fence::try_drm_fence_signal(ctx)
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
