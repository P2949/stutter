use super::*;

#[test]
fn insufficient_evidence_returns_unknown_but_keeps_candidates() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);
    let artifacts = RunArtifacts {
        irq_events: vec![irq_event(300_000)],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);

    assert_eq!(d.cause, StutterCause::Unknown);
    assert!(d.primary.is_none());
    assert!(candidate_index(&d, StutterCause::IrqDelayCandidate).is_some());
    assert!(
        d.secondary_causes
            .contains(&StutterCause::IrqDelayCandidate)
    );
    assert!(!d.evidence.is_empty());
    assert!(d.summary.starts_with("insufficient evidence"));
    assert_missing_contains(&d, "score below");
    assert_missing_contains(&d, "confidence below");
}

#[test]
fn missing_evidence_is_serialized() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);
    let artifacts = RunArtifacts {
        irq_events: vec![irq_event(300_000)],
        ..Default::default()
    };
    let d = diagnose_cluster(&cluster, &artifacts, 0);

    let json = serde_json::to_value(&d).unwrap();
    let missing = json
        .get("missing_evidence")
        .and_then(|value| value.as_array())
        .expect("missing_evidence should serialize as an array");

    assert!(
        missing
            .iter()
            .filter_map(|value| value.as_str())
            .any(|message| message.contains("score below")),
        "serialized missing_evidence={missing:?}"
    );
}

#[test]
fn ignores_tiny_irq_events() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "other",
        3_000_000,
    )]);

    // 10us IRQ should be ignored (threshold 250us)
    let irq = irq_event(10_000);

    let artifacts = RunArtifacts {
        irq_events: vec![irq],
        ..Default::default()
    };
    let d = diagnose_cluster(&cluster, &artifacts, 0);
    assert!(
        !d.secondary_causes
            .contains(&StutterCause::IrqDelayCandidate)
    );
    assert_ne!(d.cause, StutterCause::IrqDelayCandidate);

    // 1ms IRQ should be caught
    let irq_big = irq_event(1_000_000);
    let artifacts2 = RunArtifacts {
        irq_events: vec![irq_big],
        ..Default::default()
    };
    let d2 = diagnose_cluster(&cluster, &artifacts2, 0);
    assert!(
        d2.secondary_causes
            .contains(&StutterCause::IrqDelayCandidate)
            || d2.cause == StutterCause::IrqDelayCandidate
    );
}

#[test]
fn tiny_irq_is_low_or_absent_when_scheduler_delay_dominates() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);
    let irq = irq_event(300_000);
    let artifacts = RunArtifacts {
        irq_events: vec![irq],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);
    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_ne!(d.cause, StutterCause::IrqDelayCandidate);

    if let Some(irq_candidate) = d
        .candidates
        .iter()
        .find(|candidate| candidate.cause == StutterCause::IrqDelayCandidate)
    {
        assert!(irq_candidate.score < 0.40 || irq_candidate.confidence == Confidence::Low);
    }
}

#[test]
fn large_irq_can_be_primary_when_no_scheduler_anchor_exists() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "other",
        3_000_000,
    )]);
    let irq = irq_event(4_000_000);
    let artifacts = RunArtifacts {
        irq_events: vec![irq],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);
    assert_eq!(d.cause, StutterCause::IrqDelayCandidate);
    assert!(matches!(
        d.confidence,
        Confidence::Medium | Confidence::High
    ));
    assert_eq!(
        d.primary.as_ref().unwrap().cause,
        StutterCause::IrqDelayCandidate
    );
}
