//! Display timing event conversion helpers for monitor sessions.

pub(crate) fn elapsed_ms_from_event_timestamp(
    monotonic_start_ns: Option<u64>,
    timestamp_ns: u64,
) -> Option<u64> {
    monotonic_start_ns
        .and_then(|start_ns| timestamp_ns.checked_sub(start_ns))
        .map(|elapsed_ns| elapsed_ns / 1_000_000)
}

pub(crate) fn kms_flip_event_kind_name(kind: u32) -> &'static str {
    match kind {
        stutter_common::KMS_FLIP_EVENT_REQUEST => "request",
        stutter_common::KMS_FLIP_EVENT_PAGEFLIP_DONE => "pageflip_done",
        stutter_common::KMS_FLIP_EVENT_INTERVAL => "flip_interval",
        stutter_common::KMS_FLIP_EVENT_VBLANK => "vblank",
        _ => "unknown",
    }
}

pub(crate) fn kms_flip_provider_name(provider: u32) -> &'static str {
    match provider {
        stutter_common::KMS_FLIP_PROVIDER_DRM => "drm_tracepoint",
        stutter_common::KMS_FLIP_PROVIDER_I915 => "i915_tracepoint",
        stutter_common::KMS_FLIP_PROVIDER_AMDGPU => "amdgpu_tracepoint",
        _ => "unknown",
    }
}

pub(crate) fn kms_flip_flag_names(flags: u32) -> Vec<String> {
    [
        (stutter_common::KMS_FLIP_HAS_REQUEST_NS, "has_request_ns"),
        (stutter_common::KMS_FLIP_HAS_DONE_NS, "has_done_ns"),
        (stutter_common::KMS_FLIP_HAS_DURATION_NS, "has_duration_ns"),
        (stutter_common::KMS_FLIP_HAS_SEQUENCE, "has_sequence"),
        (stutter_common::KMS_FLIP_HAS_CRTC, "has_crtc"),
    ]
    .into_iter()
    .filter(|&(bit, _)| flags & bit != 0)
    .map(|(_, name)| name.to_owned())
    .collect()
}

pub(crate) fn drm_fence_event_kind_name(kind: u32) -> &'static str {
    match kind {
        stutter_common::DRM_FENCE_EVENT_WAIT_START => "wait_start",
        stutter_common::DRM_FENCE_EVENT_WAIT_DONE => "wait_done",
        stutter_common::DRM_FENCE_EVENT_SIGNAL => "signal",
        stutter_common::DRM_FENCE_EVENT_WAIT_INTERVAL => "wait_interval",
        _ => "unknown",
    }
}

pub(crate) fn drm_fence_provider_name(provider: u32) -> &'static str {
    match provider {
        stutter_common::DRM_FENCE_PROVIDER_DMA_FENCE => "dma_fence",
        stutter_common::DRM_FENCE_PROVIDER_DRM_SCHED => "drm_sched",
        stutter_common::DRM_FENCE_PROVIDER_AMDGPU => "amdgpu",
        stutter_common::DRM_FENCE_PROVIDER_I915 => "i915",
        _ => "unknown",
    }
}

pub(crate) fn drm_gpu_role_name(role: u32) -> &'static str {
    match role {
        stutter_common::DRM_GPU_ROLE_RENDER => "render",
        stutter_common::DRM_GPU_ROLE_DISPLAY => "display",
        _ => "unknown",
    }
}
