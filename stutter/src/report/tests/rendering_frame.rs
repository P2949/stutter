//! HTML rendering, diagnosis wording, frame pacing, and density report tests.

use super::*;

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
