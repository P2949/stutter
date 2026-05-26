use stutter_common::{DRM_FENCE_PROVIDER_DMA_FENCE, DRM_GPU_ROLE_UNKNOWN};

#[unsafe(no_mangle)]
pub(crate) static mut BLOCK_RQ_KEY_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut BLOCK_RQ_ISSUE_NR_SECTOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut BLOCK_RQ_ISSUE_RWBS_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut BLOCK_RQ_COMPLETE_NR_SECTOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut BLOCK_RQ_COMPLETE_RWBS_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut I915_FLIP_REQUEST_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut I915_FLIP_REQUEST_PIPE_OFFSET: u32 = 0;
#[unsafe(no_mangle)]
pub(crate) static mut I915_FLIP_REQUEST_CARD_MINOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut I915_FLIP_DONE_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut I915_FLIP_DONE_PIPE_OFFSET: u32 = 0;
#[unsafe(no_mangle)]
pub(crate) static mut I915_FLIP_DONE_CARD_MINOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut I915_FLIP_DONE_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut I915_FLIP_DONE_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FLIP_REQUEST_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FLIP_REQUEST_PIPE_OFFSET: u32 = 0;
#[unsafe(no_mangle)]
pub(crate) static mut DRM_FLIP_REQUEST_CARD_MINOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FLIP_DONE_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FLIP_DONE_PIPE_OFFSET: u32 = 0;
#[unsafe(no_mangle)]
pub(crate) static mut DRM_FLIP_DONE_CARD_MINOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FLIP_DONE_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FLIP_DONE_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_VBLANK_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_VBLANK_PIPE_OFFSET: u32 = 0;
#[unsafe(no_mangle)]
pub(crate) static mut DRM_VBLANK_CARD_MINOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_VBLANK_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_VBLANK_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_FLIP_REQUEST_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_FLIP_REQUEST_PIPE_OFFSET: u32 = 0;
#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_FLIP_REQUEST_CARD_MINOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_FLIP_DONE_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_FLIP_DONE_PIPE_OFFSET: u32 = 0;
#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_FLIP_DONE_CARD_MINOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_FLIP_DONE_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_FLIP_DONE_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_VBLANK_CRTC_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_VBLANK_PIPE_OFFSET: u32 = 0;
#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_VBLANK_CARD_MINOR_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_VBLANK_SEQUENCE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut AMDGPU_VBLANK_SEQUENCE_SIZE: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_START_CONTEXT_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_START_SEQNO_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_START_TIMELINE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_DONE_CONTEXT_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_DONE_SEQNO_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_DONE_TIMELINE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_SIGNAL_CONTEXT_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_SIGNAL_SEQNO_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_SIGNAL_TIMELINE_OFFSET: u32 = 0;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_START_PROVIDER: u32 = DRM_FENCE_PROVIDER_DMA_FENCE;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_START_GPU_ROLE: u32 = DRM_GPU_ROLE_UNKNOWN;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_DONE_PROVIDER: u32 = DRM_FENCE_PROVIDER_DMA_FENCE;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_WAIT_DONE_GPU_ROLE: u32 = DRM_GPU_ROLE_UNKNOWN;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_SIGNAL_PROVIDER: u32 = DRM_FENCE_PROVIDER_DMA_FENCE;

#[unsafe(no_mangle)]
pub(crate) static mut DRM_FENCE_SIGNAL_GPU_ROLE: u32 = DRM_GPU_ROLE_UNKNOWN;
