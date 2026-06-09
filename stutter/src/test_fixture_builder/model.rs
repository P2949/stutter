//! Shared fixture artifact model used by corpus builders.

use super::*;

#[derive(Default)]
pub(crate) struct FixtureArtifacts {
    pub(crate) spikes: Vec<SpikeEvent>,
    pub(crate) intervals: Vec<IntervalRecord>,
    pub(crate) irq_events: Vec<IrqEventRecord>,
    pub(crate) gpu_samples: Vec<GpuSample>,
    pub(crate) frame_events: Vec<FrameEvent>,
    pub(crate) block_io_events: Vec<BlockIoRecord>,
    pub(crate) foreground_events: Vec<ForegroundEvent>,
    pub(crate) kms_flip_events: Vec<KmsFlipEventRecord>,
    pub(crate) drm_fence_events: Vec<DrmFenceEventRecord>,
    pub(crate) wayland_presentation_events: Vec<WaylandPresentationEventRecord>,
    pub(crate) dmabuf_events: Vec<DmaBufEventRecord>,
    pub(crate) gpu_engine_samples: Vec<GpuEngineSample>,
    pub(crate) display_topology: Option<crate::display_topology::DisplayTopologySnapshot>,
}
