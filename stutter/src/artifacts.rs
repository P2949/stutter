use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use log::warn;
use serde::Serialize;

use crate::recorder::{LiveRecorder, NdjsonWriter, RecordingCounters};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Metadata,
    Session,
    Interval,
    SpikeEvents,
    TreeEvents,
    IrqEvents,
    GpuSamples,
    FrameCorrelation,
    FrameEvents,
    MigrationEvents,
    CpuFreqSamples,
    BlockIoEvents,
    ScxEvents,
    RuntimeSlices,
    FocusEvents,
    ForegroundEvents,
    KmsFlipEvents,
    DrmFenceEvents,
    WaylandPresentationEvents,
    DisplayTopology,
    DmaBufEvents,
    GpuEngineSamples,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactEncoding {
    JsonObject,
    Ndjson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactCounter {
    IntervalRecord,
    SpikeEventsRetained,
    IrqEvent,
    GpuSample,
    FrameEvent,
    BlockIoEvent,
    RuntimeSlice,
    FocusEvent,
    ForegroundEvent,
    MigrationEvent,
    CpuFreqSample,
    ScxEvent,
    KmsFlipEvent,
    DrmFenceEvent,
    WaylandPresentationEvent,
    DmaBufEvent,
    GpuEngineSample,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ArtifactSpec {
    pub kind: ArtifactKind,
    pub file_name: &'static str,
    pub encoding: ArtifactEncoding,
    pub required: bool,
    pub legacy_aliases: &'static [&'static str],
    pub counter_field: Option<ArtifactCounter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactPath {
    kind: ArtifactKind,
    path: PathBuf,
}

impl ArtifactPath {
    pub fn new(run_dir: impl AsRef<Path>, kind: ArtifactKind) -> Self {
        Self {
            kind,
            path: run_dir.as_ref().join(artifact_file_name(kind)),
        }
    }

    pub fn kind(&self) -> ArtifactKind {
        self.kind
    }

    pub fn file_name(&self) -> &'static str {
        artifact_file_name(self.kind)
    }

    pub fn as_path(&self) -> &Path {
        &self.path
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.path
    }
}

impl AsRef<Path> for ArtifactPath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

pub const ARTIFACT_SPECS: &[ArtifactSpec] = &[
    ArtifactSpec {
        kind: ArtifactKind::Session,
        file_name: "session.json",
        encoding: ArtifactEncoding::JsonObject,
        required: true,
        legacy_aliases: &[],
        counter_field: None,
    },
    ArtifactSpec {
        kind: ArtifactKind::Metadata,
        file_name: "metadata.json",
        encoding: ArtifactEncoding::JsonObject,
        required: false,
        legacy_aliases: &[],
        counter_field: None,
    },
    ArtifactSpec {
        kind: ArtifactKind::Interval,
        file_name: "interval.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::IntervalRecord),
    },
    ArtifactSpec {
        kind: ArtifactKind::SpikeEvents,
        file_name: "spike_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::SpikeEventsRetained),
    },
    ArtifactSpec {
        kind: ArtifactKind::TreeEvents,
        file_name: "tree_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: None,
    },
    ArtifactSpec {
        kind: ArtifactKind::IrqEvents,
        file_name: "irq_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::IrqEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::GpuSamples,
        file_name: "gpu_samples.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::GpuSample),
    },
    ArtifactSpec {
        kind: ArtifactKind::FrameEvents,
        file_name: "frame_correlation.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &["frame_events.json"],
        counter_field: Some(ArtifactCounter::FrameEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::FrameCorrelation,
        file_name: "frame_correlation.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::FrameEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::MigrationEvents,
        file_name: "migration_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::MigrationEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::CpuFreqSamples,
        file_name: "cpu_freq_samples.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::CpuFreqSample),
    },
    ArtifactSpec {
        kind: ArtifactKind::BlockIoEvents,
        file_name: "io_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::BlockIoEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::ScxEvents,
        file_name: "scx_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::ScxEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::RuntimeSlices,
        file_name: "runtime_slices.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::RuntimeSlice),
    },
    ArtifactSpec {
        kind: ArtifactKind::FocusEvents,
        file_name: "focus_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::FocusEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::ForegroundEvents,
        file_name: "foreground_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::ForegroundEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::KmsFlipEvents,
        file_name: "kms_flip_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::KmsFlipEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::DrmFenceEvents,
        file_name: "drm_fence_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::DrmFenceEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::WaylandPresentationEvents,
        file_name: "wayland_presentation_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::WaylandPresentationEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::DisplayTopology,
        file_name: "display_topology.json",
        encoding: ArtifactEncoding::JsonObject,
        required: false,
        legacy_aliases: &[],
        counter_field: None,
    },
    ArtifactSpec {
        kind: ArtifactKind::DmaBufEvents,
        file_name: "dmabuf_events.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::DmaBufEvent),
    },
    ArtifactSpec {
        kind: ArtifactKind::GpuEngineSamples,
        file_name: "gpu_engine_samples.json",
        encoding: ArtifactEncoding::Ndjson,
        required: false,
        legacy_aliases: &[],
        counter_field: Some(ArtifactCounter::GpuEngineSample),
    },
];

#[derive(Debug, Default)]
pub struct ArtifactStreamRegistry {
    streams: BTreeMap<ArtifactKind, NdjsonWriter>,
}

impl ArtifactStreamRegistry {
    pub fn new() -> Self {
        Self {
            streams: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, kind: ArtifactKind, writer: NdjsonWriter) {
        self.streams.insert(kind, writer);
    }

    pub fn create_stream(&mut self, run_dir: &Path, kind: ArtifactKind) -> anyhow::Result<()> {
        let spec = artifact_spec(kind);
        if spec.encoding != ArtifactEncoding::Ndjson {
            anyhow::bail!("artifact {:?} is not an NDJSON stream", kind);
        }
        self.insert(kind, NdjsonWriter::create(artifact_path(run_dir, kind))?);
        Ok(())
    }

    pub fn contains(&self, kind: ArtifactKind) -> bool {
        self.streams.contains_key(&kind)
    }

    pub fn push<T: Serialize>(&mut self, kind: ArtifactKind, value: &T) -> anyhow::Result<bool> {
        let Some(writer) = self.streams.get_mut(&kind) else {
            return Ok(false);
        };
        writer.push(value)?;
        Ok(true)
    }

    pub fn finish_all(&mut self) -> anyhow::Result<()> {
        for writer in self.streams.values_mut() {
            writer.finish()?;
        }
        Ok(())
    }
}

pub fn artifact_spec(kind: ArtifactKind) -> &'static ArtifactSpec {
    ARTIFACT_SPECS
        .iter()
        .find(|spec| spec.kind == kind)
        // invariant: ArtifactKind enum elements are exhaustively mapped in ARTIFACT_SPECS
        .expect("ArtifactKind must have an ArtifactSpec")
}

pub fn artifact_file_name(kind: ArtifactKind) -> &'static str {
    artifact_spec(kind).file_name
}

pub fn artifact_is_ndjson_stream(kind: ArtifactKind) -> bool {
    artifact_spec(kind).encoding == ArtifactEncoding::Ndjson
}

pub fn artifact_kinds() -> impl Iterator<Item = ArtifactKind> {
    ARTIFACT_SPECS.iter().map(|spec| spec.kind)
}

pub fn optional_artifact_kinds() -> BTreeSet<ArtifactKind> {
    ARTIFACT_SPECS
        .iter()
        .filter(|spec| !spec.required)
        .map(|spec| spec.kind)
        .collect()
}

pub fn artifact_path(run_dir: &Path, kind: ArtifactKind) -> PathBuf {
    ArtifactPath::new(run_dir, kind).into_path_buf()
}

pub fn artifact_alias_paths(run_dir: &Path, kind: ArtifactKind) -> Vec<PathBuf> {
    artifact_spec(kind)
        .legacy_aliases
        .iter()
        .map(|alias| run_dir.join(alias))
        .collect()
}

pub fn artifact_primary_and_alias_paths(run_dir: &Path, kind: ArtifactKind) -> Vec<PathBuf> {
    let mut paths = vec![artifact_path(run_dir, kind)];
    paths.extend(artifact_alias_paths(run_dir, kind));
    paths
}

pub fn artifact_counter_label(counter: ArtifactCounter) -> &'static str {
    match counter {
        ArtifactCounter::IntervalRecord => "interval record",
        ArtifactCounter::SpikeEventsRetained => "spike event",
        ArtifactCounter::IrqEvent => "IRQ event",
        ArtifactCounter::GpuSample => "GPU sample",
        ArtifactCounter::FrameEvent => "frame event",
        ArtifactCounter::BlockIoEvent => "block I/O event",
        ArtifactCounter::RuntimeSlice => "runtime slice",
        ArtifactCounter::FocusEvent => "focus event",
        ArtifactCounter::ForegroundEvent => "foreground event",
        ArtifactCounter::MigrationEvent => "migration event",
        ArtifactCounter::CpuFreqSample => "CPU frequency sample",
        ArtifactCounter::ScxEvent => "SCX event",
        ArtifactCounter::KmsFlipEvent => "KMS flip event",
        ArtifactCounter::DrmFenceEvent => "DRM fence event",
        ArtifactCounter::WaylandPresentationEvent => "Wayland presentation event",
        ArtifactCounter::DmaBufEvent => "DMABUF event",
        ArtifactCounter::GpuEngineSample => "GPU engine sample",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArtifactSelection {
    kinds: BTreeSet<ArtifactKind>,
}

impl ArtifactSelection {
    pub fn new(kinds: impl IntoIterator<Item = ArtifactKind>) -> Self {
        Self {
            kinds: kinds.into_iter().collect(),
        }
    }

    pub fn empty() -> Self {
        Self {
            kinds: BTreeSet::new(),
        }
    }

    pub fn report() -> Self {
        Self::new([
            ArtifactKind::SpikeEvents,
            ArtifactKind::FrameEvents,
            ArtifactKind::FocusEvents,
            ArtifactKind::ForegroundEvents,
            ArtifactKind::KmsFlipEvents,
            ArtifactKind::DrmFenceEvents,
            ArtifactKind::WaylandPresentationEvents,
            ArtifactKind::DisplayTopology,
            ArtifactKind::DmaBufEvents,
            ArtifactKind::GpuEngineSamples,
        ])
    }

    pub fn tune() -> Self {
        Self::new([ArtifactKind::Interval, ArtifactKind::FrameEvents])
    }

    pub fn autotune_replay() -> Self {
        Self::new(
            artifact_kinds()
                .filter(|kind| {
                    !matches!(
                        kind,
                        ArtifactKind::Session
                            | ArtifactKind::Metadata
                            | ArtifactKind::FrameCorrelation
                    )
                })
                .collect::<Vec<_>>(),
        )
    }

    pub fn validate_only() -> Self {
        Self::empty()
    }

    pub fn recording(runtime_slices: bool, foreground_events: bool) -> Self {
        let mut kinds = BTreeSet::from([
            ArtifactKind::Interval,
            ArtifactKind::IrqEvents,
            ArtifactKind::MigrationEvents,
            ArtifactKind::CpuFreqSamples,
            ArtifactKind::GpuSamples,
            ArtifactKind::BlockIoEvents,
            ArtifactKind::ScxEvents,
            ArtifactKind::SpikeEvents,
            ArtifactKind::FrameEvents,
            ArtifactKind::FocusEvents,
            ArtifactKind::DisplayTopology,
            ArtifactKind::DmaBufEvents,
            ArtifactKind::GpuEngineSamples,
        ]);

        if runtime_slices {
            kinds.insert(ArtifactKind::RuntimeSlices);
        }

        if foreground_events {
            kinds.insert(ArtifactKind::ForegroundEvents);
        }

        Self { kinds }
    }

    pub fn contains(&self, kind: ArtifactKind) -> bool {
        self.kinds.contains(&kind)
    }

    pub fn insert(&mut self, kind: ArtifactKind) {
        self.kinds.insert(kind);
    }

    pub fn iter(&self) -> impl Iterator<Item = ArtifactKind> + '_ {
        self.kinds.iter().copied()
    }
}

impl Default for ArtifactSelection {
    fn default() -> Self {
        Self::empty()
    }
}

/// Pushes an event to an NDJSON stream via the registry.
pub fn push_artifact_event<T: Serialize, F>(
    recorder: &mut LiveRecorder,
    kind: ArtifactKind,
    value: &T,
    stream_name: &str,
    mut success_fn: F,
) where
    F: FnMut(&mut RecordingCounters),
{
    match recorder.streams.push(kind, value) {
        Ok(true) => success_fn(&mut recorder.counters),
        Ok(false) => {}
        Err(err) => {
            warn!("ndjson_write_failed stream={stream_name} err={err:#}");
            recorder
                .counters
                .record_stream_write_error(stream_name, err);
        }
    }
}
