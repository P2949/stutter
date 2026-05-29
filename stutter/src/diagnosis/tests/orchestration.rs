use super::*;

#[test]
fn diagnosis_report_summary_uses_candidate_wording() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    let summary = d.report_summary();

    assert!(summary.contains("GameThreadSchedulerDelay"));
    assert!(summary.contains("candidate"));
    assert!(summary.contains("inference"));
}

#[test]
fn scx_secondary_evidence_keeps_scheduler_primary() {
    let mut point = spike_point(456, TaskClass::Game, "RenderThread", 8_000_000);
    point.scx_ops = Some("scx_lavd".to_owned());
    point.scx_state = Some("enabled".to_owned());
    let cluster = spike_cluster(vec![point]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);

    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    let game = candidate(&d, StutterCause::GameThreadSchedulerDelay);
    assert!(
        game.evidence
            .iter()
            .any(|evidence| evidence.kind == EvidenceKind::ScxState)
    );
}

#[test]
fn cpu_perf_is_supporting_scheduler_evidence() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);
    let artifacts = RunArtifacts {
        intervals: vec![IntervalRecord {
            elapsed_ms: 100,
            task: 456,
            active: true,
            class: TaskClass::Game,
            comm: "RenderThread".to_owned(),
            process_pid: Some(456),
            process_comm: "game".into(),
            samples: 1,
            stored_samples: 1,
            cpu_perf: Some(crate::metrics::CpuPerfRecord {
                ipc: Some(0.50),
                cache_mpki: Some(45.0),
                cache_miss_rate: Some(0.10),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);

    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    let game = candidate(&d, StutterCause::GameThreadSchedulerDelay);
    assert!(
        game.evidence
            .iter()
            .any(|evidence| evidence.kind == EvidenceKind::CpuPerf
                && evidence.message.contains("ipc=0.50"))
    );
}

#[test]
fn classifies_sway_compositor_delay_by_task_class_not_comm_name() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Compositor,
        "sway",
        3_000_000,
    )]);
    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
}

#[test]
fn ranked_candidates_prefer_stronger_game_delay_over_weaker_compositor_delay() {
    let cluster = spike_cluster(vec![
        spike_point(123, TaskClass::Compositor, "sway", 3_000_000),
        spike_point(456, TaskClass::Game, "RenderThread", 10_000_000),
    ]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_eq!(
        d.primary.as_ref().unwrap().cause,
        StutterCause::GameThreadSchedulerDelay
    );
    assert!(
        d.secondary_causes
            .contains(&StutterCause::CompositorSchedulerDelay)
    );
    assert!(d.candidates.len() >= 2);

    let game = candidate(&d, StutterCause::GameThreadSchedulerDelay);
    let compositor = candidate(&d, StutterCause::CompositorSchedulerDelay);
    assert!(game.score > compositor.score);
    assert!(
        d.evidence
            .iter()
            .any(|e| e.contains("compositor thread 'sway' delayed by 3.000ms"))
    );
    assert!(
        d.evidence
            .iter()
            .any(|e| e.contains("game thread 'RenderThread' delayed by 10.000ms"))
    );
}

#[test]
fn compositor_wins_when_compositor_and_game_scores_are_similar() {
    let cluster = spike_cluster(vec![
        spike_point(123, TaskClass::Compositor, "sway", 6_000_000),
        spike_point(456, TaskClass::Game, "RenderThread", 5_000_000),
    ]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
}

#[test]
fn diagnosis_keeps_legacy_fields_populated() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_eq!(d.confidence, Confidence::High);
    assert!(!d.evidence.is_empty());
    assert!(d.primary.is_some());
    assert!(!d.candidates.is_empty());
    assert!(d.summary.contains("primary="));
}
