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
    ArtifactsSummary, DataQualityLevel, DataQualitySummary, DrmFenceTimingSummary,
    DrmFenceWaitSummary, FocusReportSummary, ForegroundReportSummary, FrameOutlierView,
    FramePacingSummary, HtmlChartArtifacts, HtmlReportModel, KmsTimingSummary, PressureKind,
    PressurePeakWindow, PressureTimelineCoverage, PressureTimelineSummary, PressureWindow,
    RegressionMetric, ReportAnalysisJson, RuntimeSliceAnalysisSummary, RuntimeThreadSummary,
    ScanoutWindowEstimate, SpikeClusterAnalysis, SpikeClusterSource, SpikeDensityBucket,
    TaskHtmlRow, WaylandPresentationSummary,
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
    TextReportRenderInput, render_cluster, render_focus_summary_text,
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
mod tests {
    use super::*;
    use crate::sched_state::classify_switch_prev_state;

    #[test]
    fn report_child_modules_are_not_public_submodules() {
        let source = include_str!("mod.rs");

        let public_child_modules: Vec<&str> = source
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub mod "))
            .collect();

        assert!(
            public_child_modules.is_empty(),
            "report child modules must stay crate-private and be exposed intentionally through api::report: {public_child_modules:?}"
        );
    }

    #[test]
    fn display_timing_summaries_handle_empty_optional_streams() {
        let kms = crate::report::analysis::build_kms_timing_summary(&[]);
        let fence = crate::report::analysis::build_drm_fence_timing_summary(&[], &[], &[]);
        let wayland = crate::report::analysis::build_wayland_presentation_summary(&[], &[], &[]);

        assert_eq!(kms.event_count, 0);
        assert_eq!(kms.notes, vec!["no KMS timing events present"]);
        assert_eq!(fence.event_count, 0);
        assert_eq!(fence.confidence, "missing");
        assert_eq!(wayland.event_count, 0);
        assert_eq!(
            wayland.notes,
            vec!["no Wayland presentation events present"]
        );
    }

    #[test]
    fn display_timing_summaries_compute_basic_percentiles() {
        let kms_events = vec![
            crate::recorder::KmsFlipEventRecord {
                elapsed_ms: 1_000,
                duration_ns: Some(2_000_000),
                done_ns: Some(1_000_000_000),
                ..Default::default()
            },
            crate::recorder::KmsFlipEventRecord {
                elapsed_ms: 1_016,
                duration_ns: Some(4_000_000),
                done_ns: Some(1_016_666_667),
                ..Default::default()
            },
        ];
        let fence_events = vec![crate::recorder::DrmFenceEventRecord {
            elapsed_ms: 1_001,
            duration_ns: Some(3_000_000),
            source: "i915".to_owned(),
            gpu_role: Some("display".to_owned()),
            importer_driver: Some("i915".to_owned()),
            exporter_driver: Some("amdgpu".to_owned()),
            context: Some(7),
            seqno: Some(9),
            correlation_basis: "context_seqno".to_owned(),
            confidence: "high".to_owned(),
            ..Default::default()
        }];
        let frame_events = vec![
            crate::recorder::FrameEvent {
                elapsed_ms: 980,
                frametime_ms: 16.0,
            },
            crate::recorder::FrameEvent {
                elapsed_ms: 1_000,
                frametime_ms: 40.0,
            },
        ];
        let wayland_events = vec![crate::recorder::WaylandPresentationEventRecord {
            elapsed_ms: 1_002,
            source: "gamescope".to_owned(),
            surface_role: Some("game".to_owned()),
            commit_to_present_ns: Some(4_000_000),
            presented_ns: Some(10),
            zero_copy: Some(true),
            output_name: Some("DP-1".to_owned()),
            ..Default::default()
        }];

        let kms = crate::report::analysis::build_kms_timing_summary(&kms_events);
        let fence = crate::report::analysis::build_drm_fence_timing_summary(
            &fence_events,
            &kms_events,
            &frame_events,
        );
        let wayland = crate::report::analysis::build_wayland_presentation_summary(
            &wayland_events,
            &kms_events,
            &frame_events,
        );

        assert_eq!(kms.duration_count, 2);
        assert_eq!(kms.median_flip_ms, Some(3.0));
        assert_eq!(
            kms.scanout_window_estimate.refresh_period_ns,
            Some(16_666_667)
        );
        assert_eq!(
            kms.scanout_window_estimate
                .first_estimated_top_of_screen_visible_ns,
            Some(1_000_000_000)
        );
        assert!(
            kms.scanout_window_estimate
                .notes
                .iter()
                .any(|note| note.contains("not photon latency"))
        );
        assert_eq!(fence.wait_interval_count, 1);
        assert_eq!(fence.max_wait_ms, Some(3.0));
        assert_eq!(fence.display_gpu_wait_count, 1);
        assert_eq!(fence.cross_gpu_candidate_count, 1);
        assert_eq!(fence.waits_near_frame_outliers, 1);
        assert_eq!(fence.waits_near_kms_delays, 1);
        assert_eq!(fence.top_waits.len(), 1);
        assert_eq!(wayland.presented_count, 1);
        assert_eq!(wayland.zero_copy_ratio, Some(1.0));
        assert_eq!(wayland.p99_commit_to_present_ms, Some(4.0));
        assert_eq!(wayland.outputs_seen, vec!["DP-1"]);
        assert_eq!(wayland.source_counts.get("gamescope"), Some(&1));
        assert_eq!(wayland.surface_role_counts.get("game"), Some(&1));
        assert_eq!(wayland.delays_near_frame_outliers, 1);
        assert_eq!(wayland.delays_near_kms_delays, 1);
        assert_eq!(wayland.compositor_queue_candidate_count, 1);
    }

    fn foreground_event(
        elapsed_ms: u64,
        pid: Option<u32>,
        app_id: Option<&str>,
        class: Option<&str>,
        title: Option<&str>,
        workspace: Option<&str>,
        confidence: f32,
    ) -> ForegroundEvent {
        ForegroundEvent {
            elapsed_ms,
            source: crate::foreground::ForegroundSource::Sway,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid,
            app_id: app_id.map(str::to_owned),
            class: class.map(str::to_owned),
            title: title.map(str::to_owned),
            window_id: Some("7".to_owned()),
            workspace: workspace.map(str::to_owned),
            confidence,
            reason: "test foreground event".to_owned(),
        }
    }

    fn cluster_at(elapsed_ms: u64) -> SpikeCluster {
        SpikeCluster {
            points: vec![SpikePoint {
                elapsed_ms: Some(elapsed_ms),
                ..SpikePoint::default()
            }],
            distinct_tasks: 1,
            min_switch_ns: 0,
            max_switch_ns: 0,
            max_latency_ns: 0,
            diagnosis: None,
            diagnosis_explanation: None,
            anchor_task: None,
            anchor_class: None,
            anchor_comm: None,
            anchor_kind: None,
            foreground_pid: None,
            foreground_app_id: None,
            foreground_class: None,
            foreground_confidence: None,
            wake_graph: Vec::new(),
        }
    }

    #[test]
    fn report_includes_foreground_summary_when_events_present() {
        let mut session = minimal_session_for_report_test();
        session.config.foreground_window = true;
        session.config.foreground_source = "sway".to_owned();
        session.core.foreground_event_count = 1;

        let summary = foreground_report_summary(
            &session,
            &[foreground_event(
                1_000,
                Some(4242),
                Some("steam_app_379430"),
                Some("steam_app_379430"),
                None,
                Some("gaming"),
                0.95,
            )],
        );

        assert!(summary.enabled);
        assert_eq!(summary.source.as_deref(), Some("sway"));
        assert_eq!(summary.final_pid, Some(4242));
        assert_eq!(summary.final_app_id.as_deref(), Some("steam_app_379430"));
        assert_eq!(summary.final_class.as_deref(), Some("steam_app_379430"));
        assert_eq!(summary.final_workspace.as_deref(), Some("gaming"));
        assert_eq!(summary.event_count, 1);
        assert_eq!(summary.confidence, Some(0.95));
    }

    #[test]
    fn report_redacts_missing_title_cleanly() {
        let summary = ForegroundReportSummary {
            enabled: true,
            source: Some("sway".to_owned()),
            final_pid: Some(4242),
            final_app_id: Some("steam_app_379430".to_owned()),
            final_class: Some("steam_app_379430".to_owned()),
            final_title: None,
            final_workspace: Some("gaming".to_owned()),
            event_count: 1,
            confidence: Some(0.95),
            provider_status: Some("available".to_owned()),
            reasons: Vec::new(),
        };

        let text = render_foreground_summary_text(&summary);

        assert!(text.contains("Foreground window:"));
        assert!(text.contains("title: redacted (pass --foreground-include-title to record it)"));
        assert!(!text.contains("Private"));
    }

    #[test]
    fn spike_cluster_gets_nearest_foreground_context() {
        let mut clusters = vec![cluster_at(1_500)];
        let events = vec![
            foreground_event(
                1_000,
                Some(1111),
                Some("steamwebhelper"),
                Some("steamwebhelper"),
                None,
                None,
                0.60,
            ),
            foreground_event(
                1_400,
                Some(4242),
                Some("steam_app_379430"),
                Some("steam_app_379430"),
                None,
                Some("gaming"),
                0.95,
            ),
            foreground_event(
                1_600,
                Some(9999),
                Some("future"),
                Some("future"),
                None,
                None,
                0.95,
            ),
        ];

        annotate_clusters_with_foreground(&mut clusters, &events, 1_000);

        assert_eq!(clusters[0].foreground_pid, Some(4242));
        assert_eq!(
            clusters[0].foreground_app_id.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(
            clusters[0].foreground_class.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(clusters[0].foreground_confidence, Some(0.95));
    }

    #[test]
    fn foreground_report_summary_uses_final_event_and_redacted_title() {
        let mut session = minimal_session_for_report_test();
        session.config.foreground_window = true;
        session.config.foreground_source = "sway".to_owned();
        session.core.foreground_event_count = 2;
        session.core.foreground_source = Some("sway".to_owned());
        session.core.final_foreground_pid = Some(12345);
        session.core.final_foreground_app_id = Some("steam_app_379430".to_owned());
        session.core.final_foreground_class = Some("steam_app_379430".to_owned());

        let events = vec![
            foreground_event(
                100,
                Some(1000),
                Some("steam"),
                Some("Steam"),
                None,
                Some("gaming"),
                0.90,
            ),
            foreground_event(
                200,
                Some(12345),
                Some("steam_app_379430"),
                Some("steam_app_379430"),
                None,
                Some("gaming"),
                0.95,
            ),
        ];

        let summary = foreground_report_summary(&session, &events);

        assert!(summary.enabled);
        assert_eq!(summary.source.as_deref(), Some("sway"));
        assert_eq!(summary.final_pid, Some(12345));
        assert_eq!(summary.final_app_id.as_deref(), Some("steam_app_379430"));
        assert_eq!(summary.final_class.as_deref(), Some("steam_app_379430"));
        assert_eq!(summary.final_title, None);
        assert_eq!(summary.final_workspace.as_deref(), Some("gaming"));
        assert_eq!(summary.event_count, 2);
        assert_eq!(summary.confidence, Some(0.95));
        assert_eq!(summary.provider_status.as_deref(), Some("available"));
    }

    #[test]
    fn render_foreground_summary_text_mentions_redacted_title() {
        let summary = ForegroundReportSummary {
            enabled: true,
            source: Some("sway".to_owned()),
            final_pid: Some(12345),
            final_app_id: Some("steam_app_379430".to_owned()),
            final_class: Some("steam_app_379430".to_owned()),
            final_title: None,
            final_workspace: Some("gaming".to_owned()),
            event_count: 7,
            confidence: Some(0.95),
            provider_status: Some("available".to_owned()),
            reasons: vec!["focused Sway node from swaymsg get_tree".to_owned()],
        };

        let text = render_foreground_summary_text(&summary);

        assert!(text.contains("Foreground window:"));
        assert!(text.contains("  source: sway"));
        assert!(text.contains("  final pid: 12345"));
        assert!(text.contains("  app_id/class: steam_app_379430"));
        assert!(text.contains("  workspace: gaming"));
        assert!(text.contains("  confidence: 0.95"));
        assert!(text.contains("  events: 7"));
        assert!(text.contains("  title: redacted (pass --foreground-include-title to record it)"));
    }

    #[test]
    fn foreground_for_cluster_uses_nearest_event_at_or_before_cluster_time() {
        let cluster = cluster_at(1_500);
        let events = vec![
            foreground_event(500, Some(1), Some("old"), Some("Old"), None, None, 0.50),
            foreground_event(
                1_200,
                Some(2),
                Some("game"),
                Some("Game"),
                None,
                Some("gaming"),
                0.95,
            ),
            foreground_event(
                1_600,
                Some(3),
                Some("future"),
                Some("Future"),
                None,
                None,
                0.95,
            ),
        ];

        let selected = foreground_for_cluster(&cluster, &events, 1_000).unwrap();

        assert_eq!(selected.pid, Some(2));
        assert_eq!(selected.app_id.as_deref(), Some("game"));
    }

    #[test]
    fn foreground_for_cluster_respects_max_stale_ms() {
        let cluster = cluster_at(2_000);
        let events = vec![foreground_event(
            500,
            Some(1),
            Some("old"),
            Some("Old"),
            None,
            None,
            0.50,
        )];

        assert!(foreground_for_cluster(&cluster, &events, 1_000).is_none());
    }

    #[test]
    fn annotate_clusters_with_foreground_sets_cluster_fields() {
        let mut clusters = vec![cluster_at(1_500)];
        let events = vec![foreground_event(
            1_200,
            Some(12345),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        )];

        annotate_clusters_with_foreground(&mut clusters, &events, 1_000);

        assert_eq!(clusters[0].foreground_pid, Some(12345));
        assert_eq!(
            clusters[0].foreground_app_id.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(
            clusters[0].foreground_class.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(clusters[0].foreground_confidence, Some(0.95));
    }

    #[test]
    fn report_analysis_json_contains_foreground_summary() {
        let mut session = minimal_session_for_report_test();
        session.config.foreground_window = true;
        session.config.foreground_source = "sway".to_owned();
        session.core.foreground_event_count = 1;
        let summary = foreground_report_summary(
            &session,
            &[foreground_event(
                100,
                Some(12345),
                Some("steam_app_379430"),
                Some("steam_app_379430"),
                None,
                Some("gaming"),
                0.95,
            )],
        );

        let json = serde_json::to_string(&summary).unwrap();

        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"source\":\"sway\""));
        assert!(json.contains("\"final_pid\":12345"));
        assert!(json.contains("\"event_count\":1"));
    }
    use crate::{
        autotune::state::SituationKind,
        recorder::{FocusEvent, RecordedConfig, SessionMetadataCore},
    };

    #[test]
    fn focus_report_summary_prefers_latest_changed_focus_event() {
        let session = SessionFile {
            core: SessionMetadataCore {
                focus_mode: Some("auto-focus".to_owned()),
                final_focus_kind: Some("Browser".to_owned()),
                focus_switch_count: 2,
                ..Default::default()
            },
            config: RecordedConfig {
                auto_focus: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let events = vec![
            FocusEvent {
                elapsed_ms: 100,
                action: "changed".to_owned(),
                kind: Some("Browser".to_owned()),
                confidence: 0.62,
                situation: Some(SituationKind::BrowserFocused),
                root_pids: vec![111],
                member_pids: vec![111, 112],
                reasons: vec!["browser parent with active renderer".to_owned()],
                ..Default::default()
            },
            FocusEvent {
                elapsed_ms: 200,
                action: "changed".to_owned(),
                kind: Some("Compile".to_owned()),
                confidence: 0.87,
                score: 0.91,
                situation: Some(SituationKind::CompileLoad),
                root_pids: vec![1234],
                member_pids: vec![1234, 1235],
                reasons: vec![
                    "cargo root with 14 active compiler descendants".to_owned(),
                    "linker/write IO evidence observed".to_owned(),
                ],
                ..Default::default()
            },
        ];

        let summary = focus_report_summary(&session, &events);

        assert_eq!(summary.mode.as_deref(), Some("auto-focus"));
        assert_eq!(summary.final_focus.as_deref(), Some("Compile"));
        assert_eq!(summary.situation.as_deref(), Some("CompileLoad"));
        assert_eq!(summary.confidence, Some(0.87));
        assert_eq!(summary.score, Some(0.91));
        assert_eq!(summary.roots, vec![1234]);
        assert_eq!(summary.member_pids, vec![1234, 1235]);
        assert_eq!(summary.focus_switches, 2);
        assert_eq!(summary.reasons.len(), 2);
    }

    #[test]
    fn render_focus_summary_text_includes_visible_reasons() {
        let summary = FocusReportSummary {
            mode: Some("auto-focus".to_owned()),
            final_focus: Some("Compile".to_owned()),
            display_name: Some("cargo build".to_owned()),
            situation: Some("CompileLoad".to_owned()),
            confidence: Some(0.87),
            score: Some(0.91),
            roots: vec![1234],
            member_pids: vec![1234, 1235],
            focus_switches: 2,
            reasons: vec![
                "cargo root with 14 active compiler descendants".to_owned(),
                "CPU delta 780% over 1s".to_owned(),
            ],
        };

        let text = render_focus_summary_text(&summary);

        assert!(text.contains("Auto focus:"));
        assert!(text.contains("  mode: auto-focus"));
        assert!(text.contains("  final focus: Compile"));
        assert!(text.contains("  situation: CompileLoad"));
        assert!(text.contains("  confidence: 0.87"));
        assert!(text.contains("  roots: [1234]"));
        assert!(text.contains("  focus switches: 2"));
        assert!(text.contains("    - cargo root with 14 active compiler descendants"));
        assert!(text.contains("    - CPU delta 780% over 1s"));
    }

    #[test]
    fn event_stream_warning_is_absent_without_errors() {
        assert!(event_stream_warning(0, None).is_none());
    }

    #[test]
    fn event_stream_warning_includes_count_and_first_error() {
        let warning =
            event_stream_warning(2, Some("spike_events: No space left on device")).unwrap();

        assert!(warning.contains("2 write error"));
        assert!(warning.contains("event artifact files may be incomplete"));
        assert!(warning.contains("spike_events: No space left on device"));
    }

    #[test]
    fn event_stream_warning_handles_missing_first_error() {
        let warning = event_stream_warning(1, None).unwrap();

        assert!(warning.contains("1 write error"));
        assert!(warning.contains("first error was not recorded"));
    }

    #[test]
    fn classify_switch_prev_state_zero_is_running() {
        assert_eq!(classify_switch_prev_state(0), "running");
    }

    #[test]
    fn classify_switch_prev_state_interruptible() {
        assert_eq!(classify_switch_prev_state(1), "interruptible_sleep");
    }

    #[test]
    fn classify_switch_prev_state_uninterruptible() {
        assert_eq!(classify_switch_prev_state(2), "uninterruptible_sleep");
    }

    #[test]
    fn classify_switch_prev_state_other_sleep() {
        assert_eq!(classify_switch_prev_state(8), "traced");
    }

    #[test]
    fn classify_switch_prev_state_interruptible_wins_when_multiple_bits_set() {
        assert_eq!(classify_switch_prev_state(3), "interruptible_sleep");
    }

    #[test]
    fn test_build_wake_graph_grouping_and_sorting() {
        let points = vec![
            SpikePoint {
                task: 101,
                comm: "wakee1".to_owned(),
                waker_tid: 201,
                waker_comm: "waker1".to_owned(),
                latency_ns: 1000,
                ..SpikePoint::default()
            },
            SpikePoint {
                task: 101,
                comm: "wakee1".to_owned(),
                waker_tid: 201,
                waker_comm: "waker1".to_owned(),
                latency_ns: 2000,
                ..SpikePoint::default()
            },
            SpikePoint {
                task: 102,
                comm: "wakee2".to_owned(),
                waker_tid: 201,
                waker_comm: "waker1".to_owned(),
                latency_ns: 500,
                ..SpikePoint::default()
            },
            SpikePoint {
                task: 101,
                comm: "wakee1".to_owned(),
                waker_tid: 202,
                waker_comm: "waker2".to_owned(),
                latency_ns: 5000,
                ..SpikePoint::default()
            },
        ];

        let graph = build_wake_graph(&points);

        // Should have 3 edges:
        // 1. (201, waker1) -> (101, wakee1) count=2 max_lat=2000
        // 2. (202, waker2) -> (101, wakee1) count=1 max_lat=5000
        // 3. (201, waker1) -> (102, wakee2) count=1 max_lat=500

        // Sorted by count desc, then max_lat desc
        assert_eq!(graph.len(), 3);

        assert_eq!(graph[0].waker_tid, 201);
        assert_eq!(graph[0].wakee_tid, 101);
        assert_eq!(graph[0].count, 2);
        assert_eq!(graph[0].max_latency_ns, 2000);

        assert_eq!(graph[1].waker_tid, 202);
        assert_eq!(graph[1].count, 1);
        assert_eq!(graph[1].max_latency_ns, 5000);

        assert_eq!(graph[2].waker_tid, 201);
        assert_eq!(graph[2].wakee_tid, 102);
        assert_eq!(graph[2].count, 1);
        assert_eq!(graph[2].max_latency_ns, 500);
    }

    fn minimal_session_for_report_test() -> SessionFile {
        SessionFile {
            core: crate::recorder::SessionMetadataCore {
                schema_version: SESSION_SCHEMA_VERSION,
                duration_ms: 1000,
                interval_record_count: 1,
                active_target_pids_count: 1,
                block_io_correlation_basis: "request-pointer".to_owned(),
                ..Default::default()
            },
            stop_reason: "test".to_owned(),
            ..Default::default()
        }
    }

    fn spike_point_for_report_test(
        task: u32,
        class: TaskClass,
        comm: &str,
        latency_ns: u64,
    ) -> SpikePoint {
        SpikePoint {
            task,
            class,
            process_pid: Some(task),
            comm: comm.to_owned(),
            latency_ns,
            wakeup_ns: 10_000_000,
            switch_ns: 10_000_000 + latency_ns,
            elapsed_ms: Some(100),
            ..Default::default()
        }
    }

    fn pressure_interval(
        elapsed_ms: u64,
        cpu_some: f64,
        mem_some: f64,
        mem_full: f64,
        io_some: f64,
        io_full: f64,
    ) -> IntervalRecord {
        IntervalRecord {
            elapsed_ms,
            task: 42,
            active: true,
            class: TaskClass::Unknown,
            comm: "worker".to_owned(),
            process_pid: Some(42),
            process_comm: "worker".into(),
            cpu_psi_some: cpu_some,
            mem_psi_some: mem_some,
            mem_psi_full: mem_full,
            io_psi_some: io_some,
            io_psi_full: io_full,
            percentile_scope: "exact".to_owned(),
            ..Default::default()
        }
    }

    fn pressure_cluster() -> SpikeCluster {
        cluster_from_points(
            vec![
                SpikePoint {
                    elapsed_ms: Some(100),
                    ..spike_point_for_report_test(1, TaskClass::Unknown, "worker-a", 2_000_000)
                },
                SpikePoint {
                    elapsed_ms: Some(110),
                    ..spike_point_for_report_test(2, TaskClass::Unknown, "worker-b", 2_000_000)
                },
            ],
            2,
        )
    }

    #[test]
    fn data_quality_is_high_for_clean_minimal_session() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport::default();

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::High);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("no data-quality problems"))
        );
    }

    #[test]
    fn report_model_types_remain_available_through_legacy_reexports() {
        assert_eq!(DataQualityLevel::High, model::DataQualityLevel::High);
        assert!(!FocusReportSummary::default().is_visible());
        assert!(!model::FocusReportSummary::default().is_visible());
        assert!(!ForegroundReportSummary::default().is_visible());
        assert!(!model::ForegroundReportSummary::default().is_visible());
    }

    #[test]
    fn analysis_from_report_input_model_preserves_existing_summary_behavior() {
        let session = minimal_session_for_report_test();
        let artifacts = session_io::RunArtifacts {
            session: session.clone(),
            validation: crate::session_io::RunValidationReport::default(),
            ..Default::default()
        };

        let result = build_report_analysis_from_input(
            ReportInputModel::from_artifacts(artifacts),
            10,
            5,
            None,
        )
        .unwrap();

        assert_eq!(
            result.analysis.session.core.duration_ms,
            session.core.duration_ms
        );
        assert_eq!(result.analysis.data_quality.level, DataQualityLevel::High);
        assert!(
            result
                .analysis
                .data_quality
                .reasons
                .iter()
                .any(|reason| reason.contains("no data-quality problems"))
        );
    }

    #[test]
    fn renderers_accept_report_model_values_without_loading_or_analysis() {
        use super::render::{
            json::render_json_pretty,
            text::{TextReportRenderInput, render_report},
        };

        let session = minimal_session_for_report_test();
        let focus = FocusReportSummary::default();
        let foreground = ForegroundReportSummary::default();
        let artifacts = session_io::RunArtifacts {
            session: session.clone(),
            validation: crate::session_io::RunValidationReport::default(),
            ..Default::default()
        };
        let data_quality = data_quality_summary(&session, &artifacts.validation);
        let pressure_timeline = PressureTimelineSummary::default();
        let runtime_slices = RuntimeSliceAnalysisSummary::default();

        let correlation_sections = TextReportCorrelationSections::new();
        let rendered = render_report(TextReportRenderInput {
            path: Path::new("runs/example"),
            session: &session,
            cluster_analysis: &SpikeClusterAnalysis {
                source: SpikeClusterSource::TopSpikesFallback,
                source_count: 0,
                clusters: Vec::new(),
            },
            frame_diagnoses: &[],
            data_quality: &data_quality,
            pressure_timeline: &pressure_timeline,
            runtime_slice_summary: &runtime_slices,
            correlation_sections: &correlation_sections,
            focus_summary: &focus,
            foreground_summary: &foreground,
            top: 10,
            cluster_window_ms: 5,
            filter_class: None,
        });

        assert!(rendered.contains("file: runs/example"));
        assert!(
            render_json_pretty(&session)
                .unwrap()
                .contains("\"schema_version\"")
        );
    }

    #[test]
    fn report_text_rendering_matches_snapshot_fixture() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport::default();
        let data_quality = data_quality_summary(&session, &validation);
        let cluster_analysis = SpikeClusterAnalysis {
            source: SpikeClusterSource::TopSpikesFallback,
            source_count: 0,
            clusters: Vec::new(),
        };
        let pressure_timeline = PressureTimelineSummary::default();
        let runtime_slices = RuntimeSliceAnalysisSummary::default();
        let correlation_sections = TextReportCorrelationSections::new();

        let rendered = render_report(TextReportRenderInput {
            path: Path::new("snapshot/session.json"),
            session: &session,
            cluster_analysis: &cluster_analysis,
            frame_diagnoses: &[],
            data_quality: &data_quality,
            pressure_timeline: &pressure_timeline,
            runtime_slice_summary: &runtime_slices,
            correlation_sections: &correlation_sections,
            focus_summary: &FocusReportSummary::default(),
            foreground_summary: &ForegroundReportSummary::default(),
            top: 10,
            cluster_window_ms: 5,
            filter_class: None,
        });

        assert_eq!(rendered, include_str!("snapshots/text_report_minimal.snap"));
    }

    #[test]
    fn data_quality_is_low_for_validation_errors() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport {
            errors: vec!["bad session".to_owned()],
            ..Default::default()
        };

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::Low);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("validation errors"))
        );
    }

    #[test]
    fn data_quality_warns_on_degraded_drm_fence_evidence() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport {
            warnings: vec![
                "DRM fence events contain only signal/marker evidence; wait duration attribution is low confidence"
                    .to_owned(),
            ],
            ..Default::default()
        };

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::Medium);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("DRM fence latency evidence"))
        );
    }

    #[test]
    fn data_quality_warns_on_truncated_spikes() {
        let mut session = minimal_session_for_report_test();
        session.core.spike_events_truncated = true;
        session.core.spike_events_retained_count = 500_000;
        session.core.spike_events_dropped_count = 1;

        let validation = crate::session_io::RunValidationReport::default();

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::Medium);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("spike event stream was truncated"))
        );
    }

    #[test]
    fn data_quality_warns_on_missing_optional_artifacts() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport {
            missing_optional_files: vec!["frame_correlation.json".to_owned()],
            ..Default::default()
        };

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::Medium);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("optional correlation artifacts"))
        );
    }

    #[test]
    fn data_quality_warns_on_cpu_perf_errors() {
        let mut session = minimal_session_for_report_test();
        session.config.cpu_perf = true;
        session.core.cpu_perf_open_errors = 1;
        session.core.cpu_perf_skipped_tasks = 2;
        let validation = crate::session_io::RunValidationReport::default();

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::Medium);
        assert!(summary.cpu_perf_requested);
        assert_eq!(summary.cpu_perf_open_errors, 1);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("CPU perf counters had open/read errors"))
        );
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("CPU perf skipped 2 active tasks"))
        );
    }

    #[test]
    fn render_report_includes_data_quality_section() {
        let session = minimal_session_for_report_test();
        let artifacts = session_io::RunArtifacts::default();
        let data_quality = data_quality_summary(&session, &artifacts.validation);
        let pressure_timeline = PressureTimelineSummary::default();
        let runtime_slices = RuntimeSliceAnalysisSummary::default();

        let correlation_sections = TextReportCorrelationSections::new();
        let output = render_report(TextReportRenderInput {
            path: Path::new("session.json"),
            session: &session,
            cluster_analysis: &SpikeClusterAnalysis {
                source: SpikeClusterSource::TopSpikesFallback,
                source_count: 0,
                clusters: vec![],
            },
            frame_diagnoses: &[],
            data_quality: &data_quality,
            pressure_timeline: &pressure_timeline,
            runtime_slice_summary: &runtime_slices,
            correlation_sections: &correlation_sections,
            focus_summary: &FocusReportSummary::default(),
            foreground_summary: &ForegroundReportSummary::default(),
            top: 10,
            cluster_window_ms: 500,
            filter_class: None,
        });

        assert!(output.contains("data quality"));
        assert!(output.contains("level: High"));
    }

    #[test]
    fn pressure_timeline_empty_without_intervals() {
        let summary = build_pressure_timeline(&[], &[pressure_cluster()], 5);

        assert_eq!(summary.sample_count, 0);
        assert_eq!(summary.max_cpu_some, 0.0);
        assert_eq!(summary.max_mem_some, None);
        assert!(summary.windows.is_empty());
    }

    #[test]
    fn pressure_timeline_marks_near_spike() {
        let intervals = vec![
            pressure_interval(96, 10.0, 0.0, 0.0, 0.0, 0.0),
            pressure_interval(120, 20.0, 0.0, 0.0, 0.0, 0.0),
        ];

        let summary = build_pressure_timeline(&intervals, &[pressure_cluster()], 5);

        assert!(summary.windows[0].near_spike);
        assert!(!summary.windows[1].near_spike);
    }

    #[test]
    fn pressure_timeline_sorts_windows() {
        let intervals = vec![
            pressure_interval(300, 1.0, 0.0, 0.0, 0.0, 0.0),
            pressure_interval(100, 2.0, 0.0, 0.0, 0.0, 0.0),
        ];

        let summary = build_pressure_timeline(&intervals, &[], 5);

        assert_eq!(
            summary
                .windows
                .iter()
                .map(|window| window.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![100, 300]
        );
    }

    #[test]
    fn pressure_timeline_max_cpu_some() {
        let intervals = vec![
            pressure_interval(100, 1.0, 0.0, 0.0, 0.0, 0.0),
            pressure_interval(200, 42.0, 0.0, 0.0, 0.0, 0.0),
            pressure_interval(300, 3.0, 0.0, 0.0, 0.0, 0.0),
        ];

        let summary = build_pressure_timeline(&intervals, &[], 5);

        assert_eq!(summary.max_cpu_some, 42.0);
    }

    #[test]
    fn pressure_timeline_includes_memory_io_fields() {
        let intervals = vec![pressure_interval(100, 1.0, 2.0, 3.0, 4.0, 5.0)];

        let summary = build_pressure_timeline(&intervals, &[], 5);
        let window = &summary.windows[0];

        assert_eq!(summary.max_mem_some, Some(2.0));
        assert_eq!(summary.max_mem_full, Some(3.0));
        assert_eq!(summary.max_io_some, Some(4.0));
        assert_eq!(summary.max_io_full, Some(5.0));
        assert_eq!(window.mem_some, Some(2.0));
        assert_eq!(window.mem_full, Some(3.0));
        assert_eq!(window.io_some, Some(4.0));
        assert_eq!(window.io_full, Some(5.0));
    }

    #[test]
    fn render_report_includes_pressure_timeline_when_pressure_present() {
        let session = minimal_session_for_report_test();
        let cluster_analysis = SpikeClusterAnalysis {
            source: SpikeClusterSource::TopSpikesFallback,
            source_count: 2,
            clusters: vec![pressure_cluster()],
        };
        let artifacts = session_io::RunArtifacts {
            intervals: vec![pressure_interval(100, 40.0, 2.0, 0.0, 0.0, 0.0)],
            ..Default::default()
        };
        let data_quality = data_quality_summary(&session, &artifacts.validation);
        let pressure_timeline =
            build_pressure_timeline(&artifacts.intervals, &cluster_analysis.clusters, 5);
        let runtime_slices = RuntimeSliceAnalysisSummary::default();

        let correlation_sections = TextReportCorrelationSections::new();
        let output = render_report(TextReportRenderInput {
            path: Path::new("session.json"),
            session: &session,
            cluster_analysis: &cluster_analysis,
            frame_diagnoses: &[],
            data_quality: &data_quality,
            pressure_timeline: &pressure_timeline,
            runtime_slice_summary: &runtime_slices,
            correlation_sections: &correlation_sections,
            focus_summary: &FocusReportSummary::default(),
            foreground_summary: &ForegroundReportSummary::default(),
            top: 10,
            cluster_window_ms: 5,
            filter_class: None,
        });

        assert!(output.contains("pressure timeline"));
        assert!(output.contains("samples=1"));
        assert!(output.contains("windows_near_spikes=1"));
        assert!(output.contains("max_cpu_some=40.00"));
    }

    #[test]
    fn analysis_json_contains_pressure_timeline() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport::default();
        let analysis = ReportAnalysisJson {
            session: session.clone(),
            cluster_analysis: SpikeClusterAnalysis {
                source: SpikeClusterSource::TopSpikesFallback,
                source_count: 0,
                clusters: vec![],
            },
            frame_diagnoses: vec![],
            frame_pacing: FramePacingSummary::default(),
            pressure_timeline: build_pressure_timeline(
                &[pressure_interval(100, 10.0, 0.0, 0.0, 0.0, 0.0)],
                &[],
                5,
            ),
            runtime_slices: RuntimeSliceAnalysisSummary::default(),
            diagnosis_thresholds: crate::diagnosis::DiagnosisConfig::default().threshold_table(),
            artifacts_summary: artifacts_summary_from_session(&session),
            data_quality: data_quality_summary(&session, &validation),
            focus_summary: FocusReportSummary::default(),
            foreground_summary: ForegroundReportSummary::default(),
            kms_timing: KmsTimingSummary::default(),
            drm_fence_timing: DrmFenceTimingSummary::default(),
            wayland_presentation: WaylandPresentationSummary::default(),
        };

        let value = serde_json::to_value(&analysis).unwrap();

        assert!(value.get("pressure_timeline").is_some());
        assert_eq!(value["pressure_timeline"]["sample_count"].as_u64(), Some(1));
    }

    #[test]
    fn runtime_slice_summary_reports_sources_and_top_threads() {
        let mut session = minimal_session_for_report_test();
        session.config.runtime_slices = true;
        session.core.runtime_slice_count = 1;
        let artifacts = session_io::RunArtifacts {
            session: session.clone(),
            runtime_slices: vec![crate::metrics::RuntimeSliceRecord {
                elapsed_ms: 1000,
                task: 42,
                process_pid: Some(40),
                class: TaskClass::Game,
                comm: "RenderThread".to_owned(),
                process_comm: "Game.exe".into(),
                source: crate::metrics::RuntimeSliceSource::ProcSchedstat,
                interval_ms: 1000,
                runtime_delta_ns: 850_000_000,
                runqueue_wait_delta_ns: Some(75_000_000),
                timeslices_delta: Some(12),
                runtime_ratio: Some(0.85),
                wait_ratio: Some(0.075),
                valid: true,
                ..Default::default()
            }],
            ..Default::default()
        };

        let summary = runtime_slice_analysis_summary(&session, &artifacts);

        assert!(summary.available);
        assert_eq!(summary.sample_count, 1);
        assert_eq!(summary.source_counts.get("proc_schedstat"), Some(&1));
        assert_eq!(summary.high_runtime_threads[0].task, 42);
    }

    fn test_html_report_model() -> HtmlReportModel {
        let mut session = minimal_session_for_report_test();
        session.tasks.push(SessionTask {
            task: 42,
            active: true,
            class: TaskClass::Game,
            process_pid: Some(42),
            process_comm: "test-game".into(),
            comm: "test-game".to_owned(),
            latency: crate::recorder::RecordedLatency {
                samples: 100,
                stored_samples: 100,
                percentile_scope: "exact".to_owned(),
                avg_ns: 750_000,
                p99_ns: 2_000_000,
                max_ns: 5_000_000,
                over_1ms: 7,
                over_2ms: 3,
                over_5ms: 1,
                ..Default::default()
            },
            ..Default::default()
        });

        let validation = crate::session_io::RunValidationReport::default();
        let analysis = ReportAnalysisJson {
            session: session.clone(),
            cluster_analysis: SpikeClusterAnalysis {
                source: SpikeClusterSource::TopSpikesFallback,
                source_count: 0,
                clusters: vec![],
            },
            frame_diagnoses: vec![],
            frame_pacing: FramePacingSummary::default(),
            pressure_timeline: PressureTimelineSummary::default(),
            runtime_slices: RuntimeSliceAnalysisSummary::default(),
            diagnosis_thresholds: crate::diagnosis::DiagnosisConfig::default().threshold_table(),
            artifacts_summary: artifacts_summary_from_session(&session),
            data_quality: data_quality_summary(&session, &validation),
            focus_summary: FocusReportSummary::default(),
            foreground_summary: ForegroundReportSummary::default(),
            kms_timing: KmsTimingSummary::default(),
            drm_fence_timing: DrmFenceTimingSummary::default(),
            wayland_presentation: WaylandPresentationSummary::default(),
        };

        build_html_report_model(
            &session,
            &session_io::RunArtifacts::default(),
            &analysis,
            10,
            None,
            Some("stutter report\n==============".to_owned()),
        )
        .unwrap()
    }

    #[test]
    fn render_html_report_uses_structured_sections() {
        let model = test_html_report_model();

        let html = render_html_report(&model).unwrap();

        assert!(html.contains(r#"id="summary-section""#));
        assert!(html.contains(r#"id="data-quality-section""#));
        assert!(html.contains(r#"id="top-tasks-section""#));
        assert!(html.contains(r#"id="spike-charts-section""#));
        assert!(html.contains(r#"id="pressure-timeline-section""#));
        assert!(html.contains(r#"id="frame-pacing-section""#));
        assert!(html.contains(r#"id="cluster-analysis-section""#));
        assert!(html.contains("Why this diagnosis was chosen"));
        assert!(html.contains("Evidence missing / not strong enough"));
        assert!(html.contains(r#"id="frame-diagnoses-section""#));
        assert!(html.contains(r#"id="artifacts-section""#));
        assert!(html.contains(r#"id="data-report-model""#));
        assert!(html.contains("test-game"));
        assert!(html.contains("<summary>Legacy text report</summary>"));
        assert!(!html.contains("<pre>stutter report"));
    }

    #[test]
    fn render_cluster_uses_cautious_diagnosis_wording() {
        let mut cluster = cluster_from_points(
            vec![spike_point_for_report_test(
                456,
                TaskClass::Game,
                "RenderThread",
                8_000_000,
            )],
            1,
        );
        cluster.diagnosis = Some(diagnose_cluster(
            &cluster,
            &session_io::RunArtifacts::default(),
            0,
        ));

        let output = render_cluster(1, &cluster);

        assert!(output.contains("diagnosis: GameThreadSchedulerDelay: strong candidate"));
        assert!(output.contains("profiler inference"));
        assert!(output.contains("diagnosis_candidate cause=GameThreadSchedulerDelay"));
        assert!(output.contains("evidence kind=SchedulerDelay"));
        assert!(!output.contains("diagnosis: primary="));
    }

    #[test]
    fn test_identify_frame_spikes() {
        let frames = vec![
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: 16.0,
            },
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: 24.1,
            },
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: 30.0,
            },
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: f64::NAN,
            },
        ];

        // median 16.0 => threshold 24.0 (1.5 * 16 = 24.0, which is < 33.3)
        let spikes = identify_frame_spikes(&frames, 16.0);
        assert_eq!(spikes.len(), 2);
        assert_eq!(spikes[0].frametime_ms, 24.1);
        assert_eq!(spikes[1].frametime_ms, 30.0);

        // median 30.0 => threshold 33.3 (1.5 * 30 = 45.0, but capped at 33.3)
        let spikes = identify_frame_spikes(&frames, 30.0);
        assert!(spikes.is_empty());

        // median 0.0 => threshold 33.3
        let spikes = identify_frame_spikes(&frames, 0.0);
        assert!(spikes.is_empty());

        let frames_with_long = vec![FrameEvent {
            elapsed_ms: 0,
            frametime_ms: 33.4,
        }];
        let spikes = identify_frame_spikes(&frames_with_long, 0.0);
        assert_eq!(spikes.len(), 1);
        assert_eq!(spikes[0].frametime_ms, 33.4);
    }

    #[test]
    fn frame_pacing_summary_finds_outliers_and_links_clusters() {
        let mut cluster = cluster_from_points(
            vec![SpikePoint {
                elapsed_ms: Some(100),
                ..spike_point_for_report_test(1, TaskClass::Compositor, "kwin_wayland", 6_000_000)
            }],
            1,
        );
        cluster.anchor_class = Some(TaskClass::Compositor);
        cluster.anchor_comm = Some("kwin_wayland".to_owned());
        cluster.diagnosis = Some(diagnose_cluster(
            &cluster,
            &session_io::RunArtifacts::default(),
            0,
        ));

        let frames = vec![
            FrameEvent {
                elapsed_ms: 84,
                frametime_ms: 16.6,
            },
            FrameEvent {
                elapsed_ms: 100,
                frametime_ms: 48.5,
            },
            FrameEvent {
                elapsed_ms: 117,
                frametime_ms: 16.7,
            },
        ];

        let summary = build_frame_pacing_summary(&frames, &[cluster], &[], 2_500);

        assert_eq!(summary.frame_count, 3);
        assert_eq!(summary.outlier_count, 1);
        assert_eq!(summary.compositor_cluster_count, 1);
        assert!(summary.outliers[0].nearest_cluster_delta_ms.is_some());
        assert_eq!(
            summary.outliers[0].nearest_cluster_anchor_class,
            Some(TaskClass::Compositor)
        );
    }

    #[test]
    fn build_spike_density_counts_and_max_latency_by_bucket() {
        let spikes = vec![
            SpikeEvent {
                elapsed_ms: Some(0),
                latency_ns: 1_000_000,
                ..Default::default()
            }, // 1 ms latency, bucket 0
            SpikeEvent {
                elapsed_ms: Some(10),
                latency_ns: 5_000_000,
                ..Default::default()
            }, // 5 ms latency, bucket 0
            SpikeEvent {
                elapsed_ms: Some(99),
                latency_ns: 2_000_000,
                ..Default::default()
            }, // 2 ms latency, bucket 0
            SpikeEvent {
                elapsed_ms: Some(100),
                latency_ns: 7_000_000,
                ..Default::default()
            }, // 7 ms latency, bucket 1
        ];

        let buckets = build_spike_density(&spikes, 100);

        assert_eq!(buckets.len(), 2);

        assert_eq!(buckets[0].start_ms, 0);
        assert_eq!(buckets[0].end_ms, 100);
        assert_eq!(buckets[0].count, 3);
        assert_eq!(buckets[0].max_latency_ms, 5.0);
        // p99 of [1, 5, 2] -> sorted [1, 2, 5]. len=3. rank = (3-1)*0.99 = 1.98 -> round to 2. values[2] = 5.
        assert_eq!(buckets[0].p99_latency_ms, 5.0);

        assert_eq!(buckets[1].start_ms, 100);
        assert_eq!(buckets[1].end_ms, 200);
        assert_eq!(buckets[1].count, 1);
        assert_eq!(buckets[1].max_latency_ms, 7.0);
        assert_eq!(buckets[1].p99_latency_ms, 7.0);
    }
}
