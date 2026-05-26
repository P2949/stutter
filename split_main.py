import os
import re

with open('stutter-ebpf/src/main.rs', 'r') as f:
    main_text = f.read()

# We can use regex to extract the blocks.
kms_block_match = re.search(r'(// -----------------------------------------------------------------------------\n// KMS/flip tracing\n// -----------------------------------------------------------------------------.*?)(?=\n// -----------------------------------------------------------------------------\n// DRM fence waits/signals)', main_text, re.DOTALL)
kms_block = kms_block_match.group(1)

drm_fence_block_match = re.search(r'(// -----------------------------------------------------------------------------\n// DRM fence waits/signals\n// -----------------------------------------------------------------------------.*?)(?=\n// -----------------------------------------------------------------------------\n// License and panic handler)', main_text, re.DOTALL)
drm_fence_block = drm_fence_block_match.group(1)

# KMS helpers start at `fn try_kms_flip_request`
kms_helpers_match = re.search(r'(#\[inline\(always\)\]\nfn try_kms_flip_request.*)', kms_block, re.DOTALL)
kms_helpers = kms_helpers_match.group(1)

# DRM Fence helpers start at `#[repr(C)]`
drm_fence_helpers_match = re.search(r'(#\[repr\(C\)\].*)', drm_fence_block, re.DOTALL)
drm_fence_helpers_raw = drm_fence_helpers_match.group(1)

# Remove tracepoints from drm_fence_helpers
drm_fence_helpers = re.sub(r'#\[tracepoint\].*?\}', '', drm_fence_helpers_raw, flags=re.DOTALL)
# Make helpers pub
drm_fence_helpers = drm_fence_helpers.replace("fn try_drm_fence_wait_start", "pub fn try_drm_fence_wait_start")
drm_fence_helpers = drm_fence_helpers.replace("fn try_drm_fence_wait_done", "pub fn try_drm_fence_wait_done")
drm_fence_helpers = drm_fence_helpers.replace("fn try_drm_fence_signal", "pub fn try_drm_fence_signal")


kms_rs = """use aya_ebpf::{helpers::bpf_ktime_get_ns, programs::TracePointContext};
use stutter_common::{
    KMS_FLIP_EVENT_PAGEFLIP_DONE, KMS_FLIP_EVENT_VBLANK, KMS_FLIP_PROVIDER_AMDGPU,
    KMS_FLIP_PROVIDER_DRM, KMS_FLIP_PROVIDER_I915,
};

use crate::{
    kms_emit::emit_kms_flip_event,
    maps::{KmsFlipKey, KMS_FLIP_STARTS},
    trace_offsets::*,
    trace_read::{read_optional_u32, read_sequence_field},
};

#[inline(always)]
pub fn try_i915_flip_request(ctx: TracePointContext) -> u32 {
    try_kms_flip_request(
        ctx,
        unsafe { core::ptr::read_volatile(&raw const I915_FLIP_REQUEST_CRTC_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const I915_FLIP_REQUEST_PIPE_OFFSET) },
    )
}

#[inline(always)]
pub fn try_i915_flip_done(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_I915 << 16) | KMS_FLIP_EVENT_PAGEFLIP_DONE,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const I915_FLIP_DONE_PIPE_OFFSET) },
        ),
    )
}

#[inline(always)]
pub fn try_drm_flip_request(ctx: TracePointContext) -> u32 {
    try_kms_flip_request(
        ctx,
        unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_REQUEST_CRTC_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_REQUEST_PIPE_OFFSET) },
    )
}

#[inline(always)]
pub fn try_drm_flip_done(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_DRM << 16) | KMS_FLIP_EVENT_PAGEFLIP_DONE,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const DRM_FLIP_DONE_PIPE_OFFSET) },
        ),
    )
}

#[inline(always)]
pub fn try_drm_vblank_event(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_DRM << 16) | KMS_FLIP_EVENT_VBLANK,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const DRM_VBLANK_PIPE_OFFSET) },
        ),
    )
}

#[inline(always)]
pub fn try_amdgpu_flip_request(ctx: TracePointContext) -> u32 {
    try_kms_flip_request(
        ctx,
        unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_REQUEST_CRTC_OFFSET) },
        unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_REQUEST_PIPE_OFFSET) },
    )
}

#[inline(always)]
pub fn try_amdgpu_flip_done(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_AMDGPU << 16) | KMS_FLIP_EVENT_PAGEFLIP_DONE,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_FLIP_DONE_PIPE_OFFSET) },
        ),
    )
}

#[inline(always)]
pub fn try_amdgpu_vblank_event(ctx: TracePointContext) -> u32 {
    try_kms_flip_done(
        ctx,
        (KMS_FLIP_PROVIDER_AMDGPU << 16) | KMS_FLIP_EVENT_VBLANK,
        kms_offset_pair(
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_CRTC_OFFSET) },
            unsafe { core::ptr::read_volatile(&raw const AMDGPU_VBLANK_PIPE_OFFSET) },
        ),
    )
}

""" + kms_helpers

drm_fence_rs = """use aya_ebpf::{
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
    maps::{FenceKey, FenceSignal, FenceWaitStart, FENCE_SIGNAL_TIMES, FENCE_WAIT_STARTS},
    trace_offsets::*,
    trace_read::read_optional_u64,
};

""" + drm_fence_helpers

new_main_tracepoints = """// -----------------------------------------------------------------------------
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
"""

new_main = main_text.replace(kms_block, "")
new_main = new_main.replace(drm_fence_block, new_main_tracepoints)

# Inject mod kms and mod drm_fence after mod maps;
new_main = new_main.replace("mod maps;\nuse maps::*;", "mod maps;\nuse maps::*;\nmod kms;\nmod drm_fence;")

# Remove unused imports from main
new_main = new_main.replace("use kms_emit::emit_kms_flip_event;\n", "")
new_main = new_main.replace("    DRM_FENCE_EVENT_SIGNAL, DRM_FENCE_EVENT_WAIT_DONE,\n", "")
new_main = new_main.replace("    DRM_FENCE_EVENT_WAIT_INTERVAL, DRM_FENCE_HAS_CONTEXT, DRM_FENCE_HAS_DURATION,\n", "")
new_main = new_main.replace("    DRM_FENCE_HAS_PID, DRM_FENCE_HAS_SEQNO, DRM_FENCE_HAS_TIMELINE, DRM_FENCE_IS_EXPORTER_SIDE,\n", "")
new_main = new_main.replace("    DRM_FENCE_IS_IMPORTER_SIDE, DRM_FENCE_WAIT_DONE_WITHOUT_START, DROP_DRM_FENCE_MISSING_START,\n", "")
new_main = new_main.replace("    DrmFenceEvent, EVENT_DRM_FENCE, KMS_FLIP_EVENT_PAGEFLIP_DONE, KMS_FLIP_EVENT_VBLANK,\n", "")
new_main = new_main.replace("    KMS_FLIP_PROVIDER_AMDGPU, KMS_FLIP_PROVIDER_DRM, KMS_FLIP_PROVIDER_I915,\n", "")
new_main = new_main.replace("mod kms_emit;\n", "")

with open('stutter-ebpf/src/kms.rs', 'w') as f:
    f.write(kms_rs)

with open('stutter-ebpf/src/drm_fence.rs', 'w') as f:
    f.write(drm_fence_rs)

with open('stutter-ebpf/src/main.rs', 'w') as f:
    f.write(new_main)
