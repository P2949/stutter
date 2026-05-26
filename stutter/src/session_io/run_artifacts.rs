use std::path::PathBuf;

use anyhow::Result;
use serde::Serialize;

use super::loader::ArtifactLoader;
use crate::{
    artifacts::ArtifactKind,
    display_topology::DisplayTopologySnapshot,
    recorder::{
        BlockIoRecord, CpuFreqRecord, DmaBufEventRecord, DrmFenceEventRecord, FocusEvent,
        ForegroundEvent, FrameEvent, GpuEngineSample, GpuSample, IntervalRecord, IrqEventRecord,
        KmsFlipEventRecord, MetadataFile, MigrationEventRecord, RuntimeSliceRecord, ScxEvent,
        SessionFile, SpikeEvent, TreeEvent, WaylandPresentationEventRecord,
    },
};

#[derive(Debug, Serialize, Default)]
pub struct RunArtifacts {
    pub run_dir: PathBuf,
    pub session: SessionFile,
    pub metadata: Option<MetadataFile>,

    pub intervals: Vec<IntervalRecord>,
    pub spikes: Vec<SpikeEvent>,
    pub tree_events: Vec<TreeEvent>,
    pub irq_events: Vec<IrqEventRecord>,
    pub gpu_samples: Vec<GpuSample>,
    pub frame_events: Vec<FrameEvent>,
    pub migration_events: Vec<MigrationEventRecord>,
    pub cpu_freq_events: Vec<CpuFreqRecord>,
    pub block_io_events: Vec<BlockIoRecord>,
    pub scx_events: Vec<ScxEvent>,
    pub runtime_slices: Vec<RuntimeSliceRecord>,
    pub focus_events: Vec<FocusEvent>,
    pub foreground_events: Vec<ForegroundEvent>,
    pub kms_flip_events: Vec<KmsFlipEventRecord>,
    pub drm_fence_events: Vec<DrmFenceEventRecord>,
    pub wayland_presentation_events: Vec<WaylandPresentationEventRecord>,
    pub display_topology: Option<DisplayTopologySnapshot>,
    pub dmabuf_events: Vec<DmaBufEventRecord>,
    pub gpu_engine_samples: Vec<GpuEngineSample>,

    pub validation: RunValidationReport,
}

#[derive(Debug, Clone, Default)]
pub struct CorrelationWindows {
    pub windows_ms: Vec<(u64, u64)>,
    pub windows_ns: Vec<(u64, u64)>,
}

impl CorrelationWindows {
    pub fn is_in_ms(&self, ms: u64) -> bool {
        self.windows_ms
            .iter()
            .any(|(min, max)| ms >= *min && ms <= *max)
    }

    pub fn is_in_ns(&self, ns: u64) -> bool {
        self.windows_ns
            .iter()
            .any(|(start, end)| ns >= *start && ns <= *end)
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct RunValidationReport {
    pub run_dir: PathBuf,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub missing_optional_files: Vec<String>,
    pub present_files: Vec<String>,
}

impl RunValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

impl RunArtifacts {
    pub fn load_correlations(&mut self, windows: CorrelationWindows) -> Result<()> {
        if windows.windows_ms.is_empty() && windows.windows_ns.is_empty() {
            return Ok(());
        }

        let run_dir = &self.run_dir;
        let validation = &mut self.validation;
        let mut loader = ArtifactLoader::new(run_dir, validation);

        self.intervals = loader
            .load_optional_ndjson_filtered(ArtifactKind::Interval, |r: &IntervalRecord| {
                windows.is_in_ms(r.elapsed_ms)
            })?;

        self.tree_events = loader.load_optional_ndjson(ArtifactKind::TreeEvents)?;

        self.irq_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::IrqEvents,
            |r: &IrqEventRecord| {
                windows
                    .windows_ns
                    .iter()
                    .any(|(start, end)| r.exit_ns >= *start && r.enter_ns <= *end)
            },
        )?;

        self.gpu_samples = loader
            .load_optional_ndjson_filtered(ArtifactKind::GpuSamples, |r: &GpuSample| {
                windows.is_in_ms(r.elapsed_ms)
            })?;

        self.migration_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::MigrationEvents,
            |r: &MigrationEventRecord| windows.is_in_ns(r.timestamp_ns),
        )?;

        self.cpu_freq_events = loader
            .load_optional_ndjson_filtered(ArtifactKind::CpuFreqSamples, |r: &CpuFreqRecord| {
                windows.is_in_ns(r.timestamp_ns)
            })?;

        self.block_io_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::BlockIoEvents,
            |r: &BlockIoRecord| {
                let start_ns = r.timestamp_ns.saturating_sub(r.duration_ns);
                let end_ns = r.timestamp_ns;

                windows
                    .windows_ns
                    .iter()
                    .any(|(start, end)| end_ns >= *start && start_ns <= *end)
            },
        )?;

        self.scx_events = loader
            .load_optional_ndjson_filtered(ArtifactKind::ScxEvents, |r: &ScxEvent| {
                windows.is_in_ms(r.elapsed_ms)
            })?;

        self.runtime_slices = loader.load_optional_ndjson_filtered(
            ArtifactKind::RuntimeSlices,
            |r: &RuntimeSliceRecord| windows.is_in_ms(r.elapsed_ms),
        )?;

        self.focus_events = loader
            .load_optional_ndjson_filtered(ArtifactKind::FocusEvents, |r: &FocusEvent| {
                windows.is_in_ms(r.elapsed_ms)
            })?;

        self.foreground_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::ForegroundEvents,
            |r: &ForegroundEvent| windows.is_in_ms(r.elapsed_ms),
        )?;

        self.kms_flip_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::KmsFlipEvents,
            |r: &KmsFlipEventRecord| {
                if windows.is_in_ns(r.timestamp_ns) {
                    return true;
                }
                if let (Some(start_ns), Some(done_ns)) = (r.request_ns, r.done_ns) {
                    return windows
                        .windows_ns
                        .iter()
                        .any(|(start, end)| done_ns >= *start && start_ns <= *end);
                }
                false
            },
        )?;

        self.drm_fence_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::DrmFenceEvents,
            |r: &DrmFenceEventRecord| {
                if windows.is_in_ns(r.timestamp_ns) {
                    return true;
                }
                if let (Some(start_ns), Some(done_ns)) = (r.wait_start_ns, r.wait_done_ns) {
                    return windows
                        .windows_ns
                        .iter()
                        .any(|(start, end)| done_ns >= *start && start_ns <= *end);
                }
                false
            },
        )?;

        self.wayland_presentation_events = loader.load_optional_ndjson_filtered(
            ArtifactKind::WaylandPresentationEvents,
            |r: &WaylandPresentationEventRecord| {
                if r.presented_ns
                    .is_some_and(|presented| windows.is_in_ns(presented))
                {
                    return true;
                }
                if let (Some(commit_ns), Some(presented_ns)) = (r.commit_ns, r.presented_ns) {
                    return windows
                        .windows_ns
                        .iter()
                        .any(|(start, end)| presented_ns >= *start && commit_ns <= *end);
                }
                windows.is_in_ms(r.elapsed_ms)
            },
        )?;

        Ok(())
    }
}
