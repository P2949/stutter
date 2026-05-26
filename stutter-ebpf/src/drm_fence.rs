use aya_ebpf::{
    helpers::{bpf_get_current_pid_tgid, bpf_get_smp_processor_id, bpf_ktime_get_ns},
    programs::TracePointContext,
};
use stutter_common::{
    DRM_FENCE_EVENT_SIGNAL, DRM_FENCE_EVENT_WAIT_DONE, DRM_FENCE_EVENT_WAIT_INTERVAL,
    DRM_FENCE_HAS_CONTEXT, DRM_FENCE_HAS_DURATION, DRM_FENCE_HAS_PID, DRM_FENCE_HAS_SEQNO,
    DRM_FENCE_HAS_TIMELINE, DRM_FENCE_IS_EXPORTER_SIDE, DRM_FENCE_IS_IMPORTER_SIDE,
    DRM_FENCE_WAIT_DONE_WITHOUT_START, DROP_DRM_FENCE_MISSING_START, DrmFenceEvent,
    EVENT_DRM_FENCE,
};

use crate::{
    drop_counters::increment_drop_counter,
    maps::{FENCE_SIGNAL_TIMES, FENCE_WAIT_STARTS, FenceKey, FenceSignal, FenceWaitStart},
    trace_offsets::*,
    trace_read::read_optional_u64,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct FenceIdentity {
    key: FenceKey,
    flags: u32,
    timeline_hash: u64,
}

#[inline(always)]
pub fn try_drm_fence_wait_start(ctx: TracePointContext) -> u32 {
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

#[inline(always)]
pub fn try_drm_fence_wait_done(ctx: TracePointContext) -> u32 {
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

#[inline(always)]
pub fn try_drm_fence_signal(ctx: TracePointContext) -> u32 {
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
