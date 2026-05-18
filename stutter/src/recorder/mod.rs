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
mod writers;

pub const SESSION_SCHEMA_VERSION: u32 = 21;

// Re-export types from metrics
pub type IntervalRecord = crate::metrics::IntervalRecord;
pub type RuntimeSliceRecord = crate::metrics::RuntimeSliceRecord;

// Re-export from other crates
// Re-export from event_types.rs - these were pub in the original recorder.rs
#[allow(unused_imports)]
pub use event_types::{
    BlockIoRecord, CpuFreqRecord, FocusEvent, FrameEvent, GpuSample, IrqEventRecord,
    MigrationEventRecord, SpikeDiagnosticContext, SpikeEvent, TreeEvent,
};
// Re-export from live.rs - these were pub in the original recorder.rs
#[allow(unused_imports)]
pub use live::{ExporterState, LiveBuffers, LiveRecorder, RecordingCounters};
#[allow(unused_imports)]
pub use retention::{
    RecordingRetentionPolicy, RecordingRetentionSummary, apply_recording_retention,
    ensure_min_free_space_for_path,
};
// Re-export from session.rs - these were pub in the original recorder.rs
#[allow(unused_imports)]
pub use session::{
    CpuPerfStatus, FinalizeRecordingInput, RecordingRun, SyncTracker, finalize_recording,
    prepare_recording, print_recording_warnings, recorded_config, recorded_time,
    recording_warnings,
};
// Re-export pub(crate) items for internal crate use
#[allow(unused_imports)]
pub(crate) use session::{elapsed_ms_from_monotonic, monotonic_now_ns, saturating_u128_to_u64};
// Re-export from session_files.rs - these were pub in the original recorder.rs
#[allow(unused_imports)]
pub use session_files::{
    MetadataFile, RecordedConfig, RecordedCpuSnapshot, RecordedLatency, RecordedSpike,
    RecordedTime, SessionFile, SessionMetadataCore, SessionSpike, SessionTask, WakerEntry,
};
#[allow(unused_imports)]
pub(crate) use session_files::{
    focus_source_label, foreground_source_arg_label, foreground_source_label,
};
// Re-export from spike_buffer.rs - these were pub in the original recorder.rs
#[allow(unused_imports)]
pub use spike_buffer::{MAX_SPIKE_EVENTS, SpikeEventBuffer, SpikePushResult};
// Re-export test helper
#[cfg(test)]
pub use writers::write_interval_csv;
// Re-export from writers.rs - these were pub in the original recorder.rs
#[allow(unused_imports)]
pub use writers::{
    CsvOutput, IntervalCsvWriter, NdjsonWriter, StdoutJsonStream, write_ndjson_value,
};

pub use crate::{foreground::ForegroundEvent, scx::ScxEvent};
