//! Dispatch module for broad regression test suites.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use stutter_common::{EVENT_RUNNABLE_LATENCY, SchedulerEvent};

use crate::{
    alert::AlertPayload,
    artifacts::{ArtifactKind, ArtifactSelection},
    ebpf_loader::DropCountersSnapshot,
    events as raw_events,
    metadata::SystemMetadata,
    metrics,
    process_tree::{self, TargetDiffAction, TaskClass, TaskInfo},
    recorder::{
        self, FinalizeRecordingInput, FrameEvent, GpuSample, IrqEventRecord, RecordedCpuSnapshot,
        RecordedLatency, RecordingRun, SESSION_SCHEMA_VERSION, SessionFile, SessionTask,
        SpikeEvent, SpikeEventBuffer, recorded_config, recorded_time,
    },
    session::sinks::{MonitorEventSink, MonitorOutputConfig, MonitorSinkContext, RecorderSink},
    tasks, tune,
};

mod support;

mod latency_metrics;
mod process_model;
mod process_snapshot;
mod recording;
mod reporting;
mod serialization;
mod streaming;
mod task_lifecycle;
mod tune_coverage;
