use super::*;

#[test]
fn frame_stats_ignore_non_finite_negative_and_zero_values() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_frame(frame(1000, 0.0));
    window.push_frame(frame(1050, 16.0));
    window.push_frame(frame(1100, f64::NAN));
    window.push_frame(frame(1200, -1.0));
    window.push_frame(frame(1300, 33.0));

    assert_eq!(window.frame_count(), 2);
    assert_eq!(window.dropped_invalid_frame_count(), 3);
    assert_eq!(window.frame_max_ms(), 33.0);
    assert_eq!(window.frame_p99_ms(), 33.0);
}

#[test]
fn frame_stats_return_zero_when_every_frame_time_is_invalid() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_frame(frame(1000, 0.0));
    window.push_frame(frame(1100, -1.0));
    window.push_frame(frame(1200, f64::NAN));

    assert_eq!(window.frame_count(), 0);
    assert_eq!(window.dropped_invalid_frame_count(), 3);
    assert_eq!(window.frame_max_ms(), 0.0);
    assert_eq!(window.frame_p99_ms(), 0.0);
}

#[test]
fn window_score_aggregates_latency_frames_and_samples() {
    let mut window = RollingWindow::new(Duration::from_secs(5));
    window.push_interval(IntervalRecord {
        elapsed_ms: 1000,
        task: 42,
        samples: 30,
        over_1ms: 3,
        over_2ms: 2,
        over_5ms: 1,
        max_ns: 6_000_000,
        ..Default::default()
    });
    window.push_interval(IntervalRecord {
        elapsed_ms: 2000,
        task: 43,
        samples: 70,
        over_1ms: 4,
        over_2ms: 1,
        over_5ms: 0,
        max_ns: 4_000_000,
        ..Default::default()
    });
    window.push_frame(frame(1500, 16.0));
    window.push_frame(frame(1600, 33.0));

    let score = window.score();

    assert_eq!(score.duration_ms, 5000);
    assert_eq!(score.interval_count, 2);
    assert_eq!(score.scored_samples, 100);
    assert_eq!(score.over_1ms, 7);
    assert_eq!(score.over_2ms, 3);
    assert_eq!(score.over_5ms, 1);
    assert_eq!(score.diagnostic_score_total, 167);
    assert_eq!(score.max_latency_ns, 6_000_000);
    assert_eq!(score.frame_count, 2);
    assert_eq!(score.frame_p99_ms, 33.0);
    assert_eq!(score.frame_max_ms, 33.0);
    assert_eq!(score.dropped_invalid_frames, 0);
}

#[test]
fn window_score_quality_is_high_when_online_quality_gates_pass() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        window.push_interval(IntervalRecord {
            elapsed_ms,
            task: 42,
            samples: 20,
            over_1ms: 1,
            max_ns: 2_000_000,
            ..Default::default()
        });
    }

    let score = window.score();

    assert_eq!(score.interval_count, 5);
    assert_eq!(score.scored_samples, 100);
    assert_eq!(score.data_quality, OnlineDataQuality::High);
}

#[test]
fn window_score_default_quality_policy_does_not_require_frames() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        window.push_interval(IntervalRecord {
            elapsed_ms,
            task: 42,
            samples: 20,
            over_1ms: 1,
            max_ns: 2_000_000,
            ..Default::default()
        });
    }

    let score = window.score();

    assert_eq!(score.frame_count, 0);
    assert_eq!(score.data_quality, OnlineDataQuality::High);
}

#[test]
fn window_score_quality_is_low_when_policy_requires_frames_and_none_exist() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        window.push_interval(IntervalRecord {
            elapsed_ms,
            task: 42,
            samples: 20,
            over_1ms: 1,
            max_ns: 2_000_000,
            ..Default::default()
        });
    }

    let quality_policy = OnlineDataQualityPolicy {
        frame_data_policy: crate::autotune::quality::FrameDataPolicy::Required,
        ..OnlineDataQualityPolicy::default()
    };
    let score = window.score_with_quality_policy(&quality_policy);

    assert_eq!(score.frame_count, 0);
    assert!(score.data_quality.is_low());
    assert!(
        score
            .data_quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("no frame data"))
    );
}

#[test]
fn window_score_quality_is_low_for_empty_window() {
    let window = RollingWindow::new(Duration::from_secs(10));

    let score = window.score();

    assert_eq!(score.interval_count, 0);
    assert_eq!(score.scored_samples, 0);
    assert_eq!(score.diagnostic_score_total, 0);
    assert!(score.data_quality.is_low());
    assert!(
        score
            .data_quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("fewer than min_scored_intervals"))
    );
}

#[test]
fn window_score_quality_is_low_when_drop_counters_are_nonzero() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        window.push_interval(IntervalRecord {
            elapsed_ms,
            task: 42,
            samples: 20,
            drop_counters: crate::ebpf_loader::DropCountersSnapshot {
                ringbuf_reserve_failed: if elapsed_ms == 5000 { 1 } else { 0 },
                ..Default::default()
            },
            ..Default::default()
        });
    }

    let score = window.score();

    assert!(score.data_quality.is_low());
    assert!(
        score
            .data_quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("drop counters above policy max"))
    );
}

#[test]
fn recent_diagnoses_vec_returns_cloned_diagnoses_in_order() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_diagnosis(diagnosis(1000, StutterCause::Unknown));
    window.push_diagnosis(diagnosis(2000, StutterCause::GpuBoundCandidate));

    let diagnoses = window.recent_diagnoses_vec();

    assert_eq!(diagnoses.len(), 2);
    assert_eq!(diagnoses[0].elapsed_ms, 1000);
    assert_eq!(diagnoses[1].elapsed_ms, 2000);
}
