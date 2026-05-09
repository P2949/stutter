use crate::{
    diagnosis::{self, DiagnosisConfig, EvidenceKind, StutterCause},
    process_tree::TaskClass,
    recorder::{RecordedSpike, SpikeEvent},
    session_io::RunArtifacts,
    spike::{SpikeCluster, SpikePoint},
};

#[test]
fn test_spike_event_serialization_backward_compatibility() {
    // JSON without observed_runnable_depth
    let json = r#"{
        "task": 123,
        "class": "Game",
        "process_pid": 122,
        "process_comm": "game.exe",
        "comm": "main",
        "cpu": 0,
        "wakeup_target_cpu": 0,
        "prio": 120,
        "latency_ns": 1000000,
        "wakeup_ns": 1000,
        "switch_ns": 1001000,
        "target_pending_wakeups": 2,
        "major_faults": 0,
        "minor_faults": 0,
        "active": true
    }"#;

    let event: SpikeEvent = serde_json::from_str(json).expect("Should deserialize legacy JSON");
    assert_eq!(event.observed_runnable_depth, 0);
    assert_eq!(event.target_pending_wakeups, 2);
    assert_eq!(event.switch_prev_pid, 0);
    assert_eq!(event.switch_prev_state, 0);
}

#[test]
fn test_recorded_spike_serialization_backward_compatibility() {
    let json = r#"{
        "class": "Game",
        "process_pid": 122,
        "process_comm": "game.exe",
        "cpu": 0,
        "wakeup_target_cpu": 0,
        "prio": 120,
        "latency_ns": 1000000,
        "wakeup_ns": 1000,
        "switch_ns": 1001000,
        "target_pending_wakeups": 2
    }"#;

    let spike: RecordedSpike = serde_json::from_str(json).expect("Should deserialize legacy JSON");
    assert_eq!(spike.observed_runnable_depth, 0);
    assert_eq!(spike.target_pending_wakeups, 2);
    assert_eq!(spike.switch_prev_pid, 0);
    assert_eq!(spike.switch_prev_state, 0);
}

#[test]
fn test_diagnosis_uses_runnable_depth() {
    let mut point = SpikePoint {
        task: 123,
        class: TaskClass::Game,
        process_pid: Some(122),
        comm: "main".to_owned(),
        latency_ns: 10_000_000, // 10ms spike
        wakeup_ns: 1000,
        switch_ns: 10_001_000,
        observed_runnable_depth: 5, // High depth
        elapsed_ms: Some(100),
        ..Default::default()
    };

    let cluster = SpikeCluster {
        points: vec![point.clone()],
        distinct_tasks: 1,
        min_switch_ns: 10_001_000,
        max_switch_ns: 10_001_000,
        max_latency_ns: 10_000_000,
        ..Default::default()
    };

    let artifacts = RunArtifacts::default();
    let diagnosis = diagnosis::diagnose_cluster_with_config(
        &cluster,
        &artifacts,
        0,
        DiagnosisConfig::default(),
    );

    let candidate = diagnosis
        .candidates
        .iter()
        .find(|c| c.cause == StutterCause::GameThreadSchedulerDelay)
        .expect("Should have GameThreadSchedulerDelay candidate");

    let evidence = candidate
        .evidence
        .iter()
        .find(|e| e.message.contains("high monitored runnable depth"))
        .expect("Should have runnable depth evidence");

    assert!(evidence.message.contains("target-local CPU contention"));
    assert!(!evidence.message.contains("global runnable depth"));

    assert_eq!(evidence.kind, EvidenceKind::SchedulerDelay);
    assert!(evidence.strength >= 0.40);

    // Test diagnostic-only evidence
    point.observed_runnable_depth = 0;
    point.target_pending_wakeups = 5;

    let cluster_diag = SpikeCluster {
        points: vec![point],
        diagnosis: None,
        anchor_task: None,
        anchor_class: None,
        anchor_comm: None,
        anchor_kind: None,
        ..cluster
    };

    let diagnosis_diag = diagnosis::diagnose_cluster_with_config(
        &cluster_diag,
        &artifacts,
        0,
        DiagnosisConfig::default(),
    );

    let candidate_diag = diagnosis_diag
        .candidates
        .iter()
        .find(|c| c.cause == StutterCause::GameThreadSchedulerDelay)
        .unwrap();

    let evidence_diag = candidate_diag
        .evidence
        .iter()
        .find(|e| e.message.contains("monitored wakeup backlog"))
        .expect("Should have diagnostic-only evidence");

    assert!(evidence_diag.message.contains("(diagnostic-only)"));
}
