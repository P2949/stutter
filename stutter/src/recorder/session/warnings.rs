use serde::Serialize;

use crate::recorder::LiveRecorder;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingWarningKind {
    IntervalsDropped,
    SpikeEventsDropped,
    EventStreamWriteErrors,
    ProcessScanBudgetExceeded,
    ThreadScanLimited,
    ExistingRunDir,
    KernelTooOld,
    MissingDisplayTopology,
    MissingWaylandPresentation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RecordingWarning {
    pub kind: RecordingWarningKind,
    pub message: String,
}

impl RecordingWarning {
    fn new(kind: RecordingWarningKind, message: String) -> Self {
        Self { kind, message }
    }
}

impl std::fmt::Display for RecordingWarning {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

pub fn recording_warnings(recorder: &LiveRecorder) -> Vec<RecordingWarning> {
    let mut warnings = Vec::new();

    if recorder.counters.intervals_dropped > 0 {
        warnings.push(RecordingWarning::new(
            RecordingWarningKind::IntervalsDropped,
            format!(
                "warning: {} interval record(s) were dropped due to --retain-intervals; reports may not include full interval history",
                recorder.counters.intervals_dropped
            ),
        ));
    }

    if recorder.counters.spike_events_dropped_count > 0 {
        warnings.push(RecordingWarning::new(
            RecordingWarningKind::SpikeEventsDropped,
            format!(
                "warning: {} spike event record(s) were dropped because the in-memory spike buffer was full; reports may not include every spike",
                recorder.counters.spike_events_dropped_count
            ),
        ));
    }

    if recorder.counters.event_stream_write_errors > 0 {
        let first_err_suffix = if let Some(first_error) =
            recorder.counters.first_event_stream_write_error.as_deref()
        {
            format!("; first error: {}", first_error)
        } else {
            "".to_owned()
        };
        warnings.push(RecordingWarning::new(
            RecordingWarningKind::EventStreamWriteErrors,
            format!(
                "warning: {} event stream write error(s) occurred while recording{}; one or more NDJSON artifact files may be incomplete",
                recorder.counters.event_stream_write_errors, first_err_suffix
            ),
        ));
    }

    if recorder.counters.process_scan_budget_exceeded_count > 0 {
        warnings.push(RecordingWarning::new(
            RecordingWarningKind::ProcessScanBudgetExceeded,
            format!(
                "warning: process tree scan budget exceeded {} times; reports may be incomplete due to skipping task discovery",
                recorder.counters.process_scan_budget_exceeded_count
            ),
        ));
    }

    if recorder.counters.thread_scan_limited_count > 0 {
        warnings.push(RecordingWarning::new(
            RecordingWarningKind::ThreadScanLimited,
            format!(
                "warning: thread scan limit exceeded {} times; reports may be incomplete due to skipping thread discovery within massive processes",
                recorder.counters.thread_scan_limited_count
            ),
        ));
    }

    warnings
}

pub fn print_recording_warnings(recorder: &LiveRecorder) {
    for warning in recording_warnings(recorder) {
        eprintln!("{warning}");
    }
}
