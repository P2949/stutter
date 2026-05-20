//! Dispatch module for test fixture corpus generation.

use std::{
    borrow::Cow,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::{
    ebpf_loader::DropCountersSnapshot,
    metadata::SystemMetadata,
    process_tree::TaskClass,
    recorder::{
        BlockIoRecord, ForegroundEvent, FrameEvent, GpuSample, IntervalRecord, IrqEventRecord,
        MetadataFile, RecordedConfig, RecordedCpuSnapshot, RecordedLatency, RecordedTime,
        SESSION_SCHEMA_VERSION, SessionFile, SessionTask, SpikeEvent,
    },
};

mod corpus;
mod fixtures;
mod metadata;
mod model;

pub(crate) use corpus::{
    fixture_path, write_autotune_replay_corpus, write_public_examples_v21, write_validation_corpus,
};
pub(crate) use model::FixtureArtifacts;
