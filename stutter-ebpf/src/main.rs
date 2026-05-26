#![no_std]
#![no_main]

use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_get_smp_processor_id, bpf_ktime_get_ns},
    macros::tracepoint,
    programs::TracePointContext,
};
use stutter_common::{
    BPF_MAX_TRACKED_CPUS, DRM_FENCE_EVENT_SIGNAL, DRM_FENCE_EVENT_WAIT_DONE,
    DRM_FENCE_EVENT_WAIT_INTERVAL, DRM_FENCE_HAS_CONTEXT, DRM_FENCE_HAS_DURATION,
    DRM_FENCE_HAS_PID, DRM_FENCE_HAS_SEQNO, DRM_FENCE_HAS_TIMELINE, DRM_FENCE_IS_EXPORTER_SIDE,
    DRM_FENCE_IS_IMPORTER_SIDE, DRM_FENCE_WAIT_DONE_WITHOUT_START, DROP_DRM_FENCE_MISSING_START,
    DrmFenceEvent, EVENT_DRM_FENCE, KMS_FLIP_EVENT_PAGEFLIP_DONE, KMS_FLIP_EVENT_VBLANK,
    KMS_FLIP_PROVIDER_AMDGPU, KMS_FLIP_PROVIDER_DRM, KMS_FLIP_PROVIDER_I915,
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
mod drop_counters;
mod kms_emit;
mod map_limits;
mod trace_offsets;
mod trace_read;
mod wakeup_data;

use kms_emit::emit_kms_flip_event;
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
use trace_offsets::*;
use trace_read::{read_optional_u32, read_optional_u64, read_sequence_field};

mod maps;
use maps::*;

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
        None => {
            increment_drop_counter(DROP_DRM_FENCE_MISSING_START);
            (
                DRM_FENCE_EVENT_WAIT_DONE,
                0,
                0,
                0,
                0,
                signal.map(|signal| signal.provider).unwrap_or(0),
                0,
                signal.map(|signal| signal.ts).unwrap_or(0),
                signal.map(|signal| signal.provider).unwrap_or(0),
                DRM_FENCE_WAIT_DONE_WITHOUT_START
                    | DRM_FENCE_IS_IMPORTER_SIDE
                    | if signal.is_some() {
                        DRM_FENCE_IS_EXPORTER_SIDE
                    } else {
                        0
                    },
            )
        }
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
