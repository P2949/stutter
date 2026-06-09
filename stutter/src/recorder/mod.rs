//! Recording artifact schema, live buffers, retention, and writer facade.
//!
//! Owns:
//! - session schema versioning, recorder event types, live recorder buffers, recording counters,
//!   spike buffering, session metadata models, artifact writers, and retention helpers.
//!
//! Does not own:
//! - report analysis/rendering, autotune planning, daemon policy decisions, remote API handling,
//!   or host tuning actions.
//!
//! Allowed dependencies:
//! - config labels, foreground/scx event models, metrics/runtime-slice records, session I/O, and
//!   filesystem helpers required to prepare, write, finalize, and prune recording artifacts.
//!
//! Main entry points:
//! - `SESSION_SCHEMA_VERSION`, `LiveRecorder`, `RecordingRun`, `FinalizeRecordingInput`,
//!   `prepare_recording`, `finalize_recording`, `SpikeEventBuffer`, NDJSON/CSV writers, and
//!   the re-exported session file models.
//!
//! Safety, mutation, and persistence invariants:
//! - artifact schema changes must update `SESSION_SCHEMA_VERSION` and validation fixtures;
//! - writers must preserve append/finalize ordering and surface I/O failures to callers;
//! - retention must only remove files selected by `RecordingRetentionPolicy`;
//! - recorder code may persist run artifacts but must not apply tuning or daemon policy changes.

mod event_types;
mod live;
mod retention;
mod session;
mod session_files;
mod spike_buffer;
mod sync_tracker;
mod writers;

pub const SESSION_SCHEMA_VERSION: ArtifactSchemaVersion = ArtifactSchemaVersion::new(23);

// Re-export types from metrics
pub type IntervalRecord = crate::metrics::IntervalRecord;
pub type RuntimeSliceRecord = crate::metrics::RuntimeSliceRecord;

// Re-export from other crates
// Re-export from event_types.rs - these were pub in the original recorder.rs
pub use event_types::{
    BlockIoRecord, CpuFreqRecord, DmaBufEventRecord, DrmFenceEventRecord, FocusEvent, FrameEvent,
    GpuEngineSample, GpuSample, IrqEventRecord, KmsFlipEventRecord, MigrationEventRecord,
    SpikeDiagnosticContext, SpikeEvent, TreeEvent, WaylandPresentationEventRecord,
};
// Re-export from live.rs - these were pub in the original recorder.rs
pub use live::{ExporterState, LiveBuffers, LiveRecorder, RecordingCounters};
pub use retention::{
    RecordingRetentionPolicy, RecordingRetentionSummary, apply_recording_retention,
    ensure_min_free_space_for_path,
};
// Re-export pub(crate) items for internal crate use
pub(crate) use session::monotonic_now_ns;
#[cfg(test)]
pub(crate) use session::saturating_u128_to_u64;
// Re-export from session.rs - these were pub in the original recorder.rs
pub use session::{
    CpuPerfStatus, FinalizeRecordingInput, RecordingRun, RecordingWarning, RecordingWarningKind,
    finalize_recording, prepare_recording, print_recording_warnings, recorded_config,
    recorded_time, recording_warnings,
};
// Re-export from session_files.rs - these were pub in the original recorder.rs
#[cfg(test)]
pub use session_files::DisplayPathMetadata;
pub use session_files::{
    ArtifactSchemaVersion, MetadataFile, RecordedConfig, RecordedCpuSnapshot, RecordedLatency,
    RecordedProbeActivationWarning, RecordedSpike, RecordedTime, SessionFile, SessionMetadataCore,
    SessionSpike, SessionTask, WakerEntry,
};
// Re-export from spike_buffer.rs - these were pub in the original recorder.rs
pub use spike_buffer::{MAX_SPIKE_EVENTS, SpikeEventBuffer, SpikePushResult};
pub use sync_tracker::SyncTracker;
// Re-export test helper
#[cfg(test)]
pub use writers::write_interval_csv;
// Re-export from writers.rs - these were pub in the original recorder.rs
pub use writers::{
    CsvOutput, IntervalCsvWriter, NdjsonWriter, StdoutJsonStream, write_ndjson_value,
};

pub use crate::{foreground::ForegroundEvent, scx::ScxEvent};

#[cfg(test)]
mod tests {
    #[test]
    fn recorder_child_modules_are_not_public_submodules() {
        let source = include_str!("mod.rs");

        let public_child_modules: Vec<&str> = source
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub mod "))
            .collect();

        assert!(
            public_child_modules.is_empty(),
            "recorder child modules must stay private and be exposed intentionally through api::recorder: {public_child_modules:?}"
        );
    }
}
