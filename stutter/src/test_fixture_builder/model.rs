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
}
