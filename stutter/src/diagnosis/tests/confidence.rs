use super::*;

#[test]
fn confidence_words_are_cautious() {
    assert_eq!(Confidence::High.as_report_word(), "strong candidate");
    assert_eq!(Confidence::Medium.as_report_word(), "candidate");
    assert_eq!(Confidence::Low.as_report_word(), "weak candidate");

    assert!(Confidence::High.caution_text().contains("inference"));
    assert!(Confidence::Medium.caution_text().contains("mixed"));
    assert!(Confidence::Low.caution_text().contains("weak"));
}

#[test]
fn diagnosis_threshold_table_covers_all_config_fields() {
    let table = DiagnosisConfig::default().threshold_table();
    let keys = table.iter().map(|entry| entry.key).collect::<BTreeSet<_>>();
    let expected = [
        "irq_significant_ns",
        "block_io_significant_ns",
        "gpu_busy_bound_percent",
        "sched_delay_significant_ns",
        "cpu_psi_some_significant",
        "cpu_freq_drop_percent",
        "migration_window_ms",
        "page_fault_delta_threshold",
        "low_ipc_threshold",
        "high_cache_mpki_threshold",
        "min_primary_score",
        "min_primary_confidence",
        "min_primary_evidence_items",
        "min_scheduler_latency_for_primary_ns",
        "min_non_scheduler_score_for_primary",
        "runtime_high_ratio",
        "runtime_wait_high_ratio",
        "runtime_min_samples_for_primary_support",
    ];

    assert_eq!(
        table.len(),
        keys.len(),
        "duplicate threshold keys: {table:?}"
    );
    for key in expected {
        assert!(keys.contains(key), "missing threshold key {key}");
    }
    assert_eq!(table.len(), expected.len());
    for entry in table {
        assert!(entry.value > 0.0, "non-positive threshold value: {entry:?}");
        assert!(
            !entry.description.trim().is_empty(),
            "missing threshold description: {entry:?}"
        );
    }
}

#[test]
fn weak_scheduler_delay_is_not_forced_high_confidence() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        2_100_000,
    )]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);

    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_ne!(d.confidence, Confidence::High);
    assert_eq!(
        candidate(&d, StutterCause::GameThreadSchedulerDelay).confidence,
        Confidence::Medium
    );
}

#[test]
fn strict_primary_confidence_leaves_weak_scheduler_unknown() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        2_100_000,
    )]);
    let config = DiagnosisConfig {
        min_primary_confidence: Confidence::High,
        ..DiagnosisConfig::default()
    };

    let d = diagnose_cluster_with_config(&cluster, &RunArtifacts::default(), 0, config);

    assert_eq!(d.cause, StutterCause::Unknown);
    assert!(d.primary.is_none());
    assert!(candidate_index(&d, StutterCause::GameThreadSchedulerDelay).is_some());
    assert_missing_contains(&d, "confidence below");
    assert!(
        d.candidate_rejections.iter().any(|rejection| {
            rejection.cause == StutterCause::GameThreadSchedulerDelay
                && rejection
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("confidence below"))
        }),
        "expected rejected primary explanation, got {:?}",
        d.candidate_rejections
    );
}

#[test]
fn strong_scheduler_delay_still_reaches_high_confidence() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);

    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_eq!(d.confidence, Confidence::High);
}
