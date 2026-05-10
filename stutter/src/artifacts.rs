use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::recorder::NdjsonWriter;

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
        self.insert(kind, NdjsonWriter::create(run_dir.join(spec.file_name))?);
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
    run_dir.join(artifact_file_name(kind))
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
