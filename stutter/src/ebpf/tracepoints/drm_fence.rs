//! DRM fence tracepoint identity validation.

use stutter_common::{
    DRM_FENCE_PROVIDER_AMDGPU, DRM_FENCE_PROVIDER_DMA_FENCE, DRM_FENCE_PROVIDER_DRM_SCHED,
    DRM_FENCE_PROVIDER_I915, DRM_GPU_ROLE_DISPLAY, DRM_GPU_ROLE_RENDER, DRM_GPU_ROLE_UNKNOWN,
};

use crate::drm_fence_tracepoints::{
    DrmFenceTracepointDiscovery, DrmFenceTracepointField, DrmFenceTracepointFormat,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DrmFenceTracepointOffsets {
    pub(crate) wait_start_context_offset: u32,
    pub(crate) wait_start_seqno_offset: u32,
    pub(crate) wait_start_timeline_offset: u32,
    pub(crate) wait_start_provider: u32,
    pub(crate) wait_start_gpu_role: u32,
    pub(crate) wait_done_context_offset: u32,
    pub(crate) wait_done_seqno_offset: u32,
    pub(crate) wait_done_timeline_offset: u32,
    pub(crate) wait_done_provider: u32,
    pub(crate) wait_done_gpu_role: u32,
    pub(crate) signal_context_offset: u32,
    pub(crate) signal_seqno_offset: u32,
    pub(crate) signal_timeline_offset: u32,
    pub(crate) signal_provider: u32,
    pub(crate) signal_gpu_role: u32,
    pub(crate) has_wait_interval: bool,
    pub(crate) has_signal: bool,
}

pub(crate) fn drm_fence_tracepoint_offsets(
    discovery: &DrmFenceTracepointDiscovery,
) -> Option<DrmFenceTracepointOffsets> {
    let mut offsets = DrmFenceTracepointOffsets::default();

    if let (Some(start), Some(done)) = (
        discovery.selected_wait_start(),
        discovery.selected_wait_done(),
    ) && let (Some(start_identity), Some(done_identity)) =
        (fence_identity_offsets(start), fence_identity_offsets(done))
    {
        let (start_provider, start_gpu_role) = drm_fence_provider_for_category(&start.category);
        let (done_provider, done_gpu_role) = drm_fence_provider_for_category(&done.category);
        offsets.wait_start_context_offset = start_identity.context_offset;
        offsets.wait_start_seqno_offset = start_identity.seqno_offset;
        offsets.wait_start_timeline_offset = start_identity.timeline_offset;
        offsets.wait_start_provider = start_provider;
        offsets.wait_start_gpu_role = start_gpu_role;
        offsets.wait_done_context_offset = done_identity.context_offset;
        offsets.wait_done_seqno_offset = done_identity.seqno_offset;
        offsets.wait_done_timeline_offset = done_identity.timeline_offset;
        offsets.wait_done_provider = done_provider;
        offsets.wait_done_gpu_role = done_gpu_role;
        offsets.has_wait_interval = true;
    }

    if let Some(signal) = discovery.selected_signal()
        && let Some(signal_identity) = fence_identity_offsets(signal)
    {
        let (signal_provider, signal_gpu_role) = drm_fence_provider_for_category(&signal.category);
        offsets.signal_context_offset = signal_identity.context_offset;
        offsets.signal_seqno_offset = signal_identity.seqno_offset;
        offsets.signal_timeline_offset = signal_identity.timeline_offset;
        offsets.signal_provider = signal_provider;
        offsets.signal_gpu_role = signal_gpu_role;
        offsets.has_signal = true;
    }

    (offsets.has_wait_interval || offsets.has_signal).then_some(offsets)
}

pub(crate) fn drm_fence_probe_supported(discovery: &DrmFenceTracepointDiscovery) -> bool {
    drm_fence_tracepoint_offsets(discovery).is_some()
}

pub(crate) fn drm_fence_probe_has_wait_interval(discovery: &DrmFenceTracepointDiscovery) -> bool {
    drm_fence_tracepoint_offsets(discovery).is_some_and(|offsets| offsets.has_wait_interval)
}

pub(crate) fn drm_fence_probe_has_signal(discovery: &DrmFenceTracepointDiscovery) -> bool {
    drm_fence_tracepoint_offsets(discovery).is_some_and(|offsets| offsets.has_signal)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FenceIdentityOffsets {
    context_offset: u32,
    seqno_offset: u32,
    timeline_offset: u32,
}

fn fence_identity_offsets(format: &DrmFenceTracepointFormat) -> Option<FenceIdentityOffsets> {
    let context = find_fence_field(format, &["context", "ctx"]);
    let timeline = find_fence_field(format, &["timeline", "timeline_hash", "timeline_name"]);
    let seqno = find_fence_field(format, &["seqno", "seq", "sequence"]);

    let has_context_or_timeline = context.is_some() || timeline.is_some();
    let seqno = seqno?;
    has_context_or_timeline.then_some(FenceIdentityOffsets {
        context_offset: context.map(|field| field.offset).unwrap_or(0),
        seqno_offset: seqno.offset,
        timeline_offset: timeline.map(|field| field.offset).unwrap_or(0),
    })
}

fn find_fence_field<'a>(
    format: &'a DrmFenceTracepointFormat,
    names: &[&str],
) -> Option<&'a DrmFenceTracepointField> {
    format.find_field(names).filter(|field| field.size >= 8)
}

fn drm_fence_provider_for_category(category: &str) -> (u32, u32) {
    match category {
        "amdgpu" => (DRM_FENCE_PROVIDER_AMDGPU, DRM_GPU_ROLE_RENDER),
        "i915" => (DRM_FENCE_PROVIDER_I915, DRM_GPU_ROLE_DISPLAY),
        "drm_sched" => (DRM_FENCE_PROVIDER_DRM_SCHED, DRM_GPU_ROLE_RENDER),
        _ => (DRM_FENCE_PROVIDER_DMA_FENCE, DRM_GPU_ROLE_UNKNOWN),
    }
}
