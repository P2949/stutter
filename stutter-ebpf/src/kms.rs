use aya_ebpf::{helpers::bpf_ktime_get_ns, programs::TracePointContext};
use stutter_common::{
    KMS_FLIP_EVENT_PAGEFLIP_DONE, KMS_FLIP_EVENT_VBLANK, KMS_FLIP_PROVIDER_AMDGPU,
    KMS_FLIP_PROVIDER_DRM, KMS_FLIP_PROVIDER_I915,
};

use crate::{
    kms_emit::emit_kms_flip_event,
    maps::{KMS_FLIP_STARTS, KmsFlipKey},
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
