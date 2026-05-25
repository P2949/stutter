//! Report loading, analysis, rendering, diffing, and regression checks.
//!
//! Owns:
//! - conversion from recorded artifacts into report input models, spike/pressure/focus analysis,
//!   HTML/text rendering, run diffs, regression summaries, and report-facing data models.
//!
//! Does not own:
//! - live recording, daemon/runtime mutation, action execution, remote authorization, or raw probe
//!   collection.
//!
//! Allowed dependencies:
//! - diagnosis, metrics formatting, recorder event/session types, runtime slice models,
//!   session I/O, spike analysis, summary helpers, and autotune report overlays.
//!
//! Main entry points:
//! - `print_report`, `write_html_report`, `build_report_analysis`, `print_diff_report`,
//!   `print_batch_report`, `check_regression`, and the exported report model types.
//!
//! Safety, mutation, and persistence invariants:
//! - report code reads existing artifacts and writes requested report outputs only;
//! - schema-version expectations must come from recorder/session artifacts, not ad-hoc guesses;
//! - analysis must preserve data-quality warnings rather than hiding missing or stale evidence;
//! - renderers must not trigger host tuning actions or daemon state transitions.

use std::{
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::Serialize;

pub use crate::error::ReportError;
use crate::{
    diagnosis::{Diagnosis, FrameDiagnosis, diagnose_cluster, select_anchor_for_diagnosis},
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{
        FocusEvent, ForegroundEvent, FrameEvent, IntervalRecord, RecordedSpike,
        SESSION_SCHEMA_VERSION, SessionFile, SessionTask, SpikeEvent,
    },
    session_io::{self},
    spike::{
        DiagnosisCandidateView, DiagnosisEvidenceView, DiagnosisExplanation, SpikeCluster,
        SpikePoint, WakeGraphEdge,
    },
    summary::{self, format_latency_signed},
};

pub(crate) mod analysis;
pub(crate) mod diff;
pub(crate) mod html;
pub(crate) mod load;
mod model;
pub(crate) mod regression;
pub(crate) mod render;
pub(crate) mod text;

pub use analysis::build_report_analysis;
#[cfg(test)]
pub use analysis::build_spike_density;
#[cfg(test)]
pub(crate) use analysis::text_report_correlation_sections;
#[cfg(test)]
pub(crate) use analysis::{
    annotate_clusters_with_foreground, artifacts_summary_from_session, build_frame_pacing_summary,
    build_pressure_timeline, build_wake_graph, cluster_from_points, focus_report_summary,
    foreground_for_cluster, foreground_report_summary, identify_frame_spikes,
    runtime_slice_analysis_summary, spike_cluster_analysis,
};
pub(crate) use analysis::{
    build_report_analysis_from_input, data_quality_summary, event_stream_warning, ms_to_ns_i64,
    violation_from_delta,
};
#[cfg(test)]
pub use diff::render_diff_report;
pub(crate) use diff::{RunDiffSummary, TaskDeltaSummary, build_run_diff_summary};
pub use diff::{print_batch_report, print_diff_report};
#[cfg(test)]
pub use html::build_html_report_model;
pub(crate) use html::task_html_row;
pub use html::write_html_report;
pub(crate) use load::{load_report_input, load_report_session};
pub use model::{
    ArtifactsSummary, CrossGpuFenceCandidate, CrossGpuFenceSummary, DataQualityLevel,
    DataQualitySummary, DirectScanoutSummary, DisplayPathComponent, DisplayPathDiagnosisSummary,
    DmaBufPathSummary, DrmFenceTimingSummary, DrmFenceWaitSummary, FocusReportSummary,
    ForegroundReportSummary, FrameOutlierView, FramePacingSummary, GpuEngineActivitySummary,
    HtmlChartArtifacts, HtmlReportModel, KmsTimingSummary, PressureKind, PressurePeakWindow,
    PressureTimelineCoverage, PressureTimelineSummary, PressureWindow, RegressionMetric,
    ReportAnalysisJson, RuntimeSliceAnalysisSummary, RuntimeThreadSummary, ScanoutWindowEstimate,
    SpikeClusterAnalysis, SpikeClusterSource, SpikeDensityBucket, TaskHtmlRow,
    WaylandPresentationSummary,
};
pub(crate) use model::{
    ReportBuildResult, ReportInputModel, SpikeClusterCandidate, TextReportCorrelationSection,
    TextReportCorrelationSections,
};
#[cfg(test)]
pub use regression::check_percentile_regression;
pub use regression::{RegressionCheckSummary, RegressionViolation, check_regression};
#[cfg(test)]
pub(crate) use render::html::render_html_report;
pub(crate) use render::text::render_check_summary;
#[cfg(test)]
pub(crate) use render::text::{
    TextReportRenderInput, render_focus_summary_text,
    render_foreground_summary_text, render_report,
};
pub use text::{PrintReportInput, print_report};

const MIN_CLUSTER_TASKS: usize = 3;
const MAX_INLINE_CLUSTER_POINTS: usize = 8;
const MAX_CLUSTER_CANDIDATES: usize = 4096;
const PRESSURE_NOTE_CPU_SOME: f64 = 50.0;
const PRESSURE_NOTE_MEM_SOME: f64 = 20.0;
const PRESSURE_NOTE_MEM_FULL: f64 = 5.0;
const PRESSURE_NOTE_IO_SOME: f64 = 20.0;
const PRESSURE_NOTE_IO_FULL: f64 = 5.0;
const MAX_PRESSURE_PEAK_WINDOWS: usize = 8;

#[cfg(test)]
mod tests;
