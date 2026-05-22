use super::{assertions::*, fixture::*};
use crate::{
    artifacts::ArtifactSelection,
    recorder::SESSION_SCHEMA_VERSION,
    report::{self},
};

#[test]
fn pressure_timeline_marks_near_spike_windows() {
    let analysis = build_fixture_analysis("cpu_pressure");
    assert!(analysis.pressure_timeline.sample_count > 0);
    assert!(
        analysis
            .pressure_timeline
            .windows
            .iter()
            .any(|window| window.near_spike),
        "pressure timeline should mark at least one window near a spike"
    );
}

#[test]
fn pressure_timeline_reports_coverage() {
    let analysis = build_fixture_analysis("cpu_pressure");
    assert!(analysis.pressure_timeline.coverage.interval_records_loaded > 0);
    assert!(analysis.pressure_timeline.coverage.has_cpu_psi);
}

#[test]
fn frame_pacing_summary_finds_outliers() {
    let analysis = build_fixture_analysis("real_gpu_bound_looking");
    assert!(analysis.frame_pacing.frame_count > 0);
    assert!(analysis.frame_pacing.outlier_count > 0);
}

#[test]
fn frame_pacing_summary_links_outliers_to_clusters() {
    let analysis = build_fixture_analysis("real_compositor_scheduler_delay");
    assert!(
        analysis
            .frame_pacing
            .outliers
            .iter()
            .any(|outlier| matches!(
                outlier.nearest_cluster_anchor_class,
                Some(crate::process_tree::TaskClass::Compositor)
                    | Some(crate::process_tree::TaskClass::GameScope)
            )),
        "expected at least one frame outlier near a compositor/gamescope scheduler cluster"
    );
}

#[test]
fn cluster_diagnosis_explanation_contains_primary_evidence() {
    let analysis = build_fixture_analysis("real_game_thread_scheduler_delay");
    let cluster = analysis
        .cluster_analysis
        .clusters
        .iter()
        .find(|cluster| cluster.diagnosis.is_some())
        .expect("expected diagnosed cluster");

    let explanation = cluster
        .diagnosis_explanation
        .as_ref()
        .expect("expected diagnosis explanation");

    assert!(explanation.primary_cause.is_some());
    assert!(!explanation.evidence_items.is_empty());
}

#[test]
fn html_report_contains_new_report_views() {
    let path = fixture_path("real_gpu_bound_looking");
    let analysis = report::build_report_analysis(&path, 10, 5, None)
        .expect("analysis should build for HTML smoke test");
    let artifacts = crate::session_io::load_run_artifacts(&path, ArtifactSelection::report())
        .expect("artifacts should load for HTML smoke test");
    let model =
        report::build_html_report_model(&analysis.session, &artifacts, &analysis, 10, None, None)
            .expect("HTML report model should build");
    let html = report::render_html_report(&model).expect("HTML report should render");

    assert!(html.contains("Pressure Timeline"));
    assert!(html.contains("Frame Pacing"));
    assert!(html.contains("Why this diagnosis was chosen"));
    assert!(html.contains("Evidence missing"));
}

#[test]
fn validation_corpus_missing_evidence_unknown() {
    let analysis = assert_fixture_from_metadata("missing_evidence_unknown");
    let diagnosis = &analysis.display_path_diagnosis;

    assert_eq!(diagnosis.confidence, "missing");
    assert_eq!(diagnosis.suspicion_score, 0.0);
    assert!(!diagnosis.missing_evidence.is_empty());
}

#[test]
fn validation_corpus_clean_run_is_high_quality_without_false_diagnosis() {
    let analysis = assert_fixture_from_metadata("clean_run");

    assert!(analysis.data_quality.validation_errors.is_empty());
    assert!(analysis.cluster_analysis.clusters.is_empty());
}

#[test]
fn validation_corpus_truncated_drop_counters_is_not_high_quality() {
    let analysis = assert_fixture_from_metadata("truncated_drop_counters");

    assert!(analysis.data_quality.spike_events_truncated);
    assert!(analysis.data_quality.drop_counters_nonzero);
    assert!(analysis.data_quality.spike_events_dropped_count > 0);
    assert_eq!(
        analysis.data_quality.spike_events_retained_count,
        analysis.artifacts_summary.spike_count
    );
    assert_quality_reasons_contain(&analysis, &["truncated".to_owned(), "drop".to_owned()]);
}

#[test]
fn validation_corpus_reused_tid_no_contamination() {
    let analysis = assert_fixture_from_metadata("reused_tid_no_contamination");

    let reused_tasks = analysis
        .session
        .tasks
        .iter()
        .filter(|task| task.task == 4242)
        .collect::<Vec<_>>();
    assert_eq!(reused_tasks.len(), 2, "reused TID should remain split");

    let old_task = reused_tasks
        .iter()
        .find(|task| task.comm == "old-worker")
        .copied()
        .expect("missing old logical task");
    let new_task = reused_tasks
        .iter()
        .find(|task| task.comm == "new-worker")
        .copied()
        .expect("missing new logical task");

    assert_eq!(old_task.latency.samples, 2);
    assert_eq!(new_task.latency.samples, 3);
    assert_ne!(old_task.process_pid, new_task.process_pid);
    assert_ne!(
        old_task.process_starttime_ticks,
        new_task.process_starttime_ticks
    );
    assert_ne!(old_task.task_starttime_ticks, new_task.task_starttime_ticks);
    assert_ne!(old_task.exe_ino, new_task.exe_ino);
    assert!(
        !reused_tasks
            .iter()
            .any(|task| task.latency.samples == 5 || task.latency.max_ns > 1_200_000),
        "reused TID stats appear to be combined: {reused_tasks:?}"
    );
}

#[test]
fn validation_corpus_old_schema_warns_without_rejecting() {
    let analysis = assert_fixture_from_metadata("old_schema_warning");

    assert_eq!(
        analysis.data_quality.schema_version,
        SESSION_SCHEMA_VERSION - 1
    );
    assert_eq!(
        analysis.data_quality.expected_schema_version,
        SESSION_SCHEMA_VERSION
    );
    assert_quality_reasons_contain(&analysis, &["older than current".to_owned()]);
    assert!(
        analysis.data_quality.validation_errors.is_empty(),
        "old schema should warn, not error: {:?}",
        analysis.data_quality.validation_errors
    );
}
