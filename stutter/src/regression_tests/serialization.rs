//! Regression coverage for backward-compatible session and event serialization.

use super::*;

#[test]
fn old_session_task_without_starttime_fields_deserializes() {
    let task: recorder::SessionTask = serde_json::from_value(serde_json::json!({
        "task": 7,
        "active": true,
        "first_seen_ms": 0,
        "last_seen_ms": 1,
        "removed_ms": null,
        "class": "Game",
        "process_pid": 7,
        "process_comm": "game",
        "comm": "worker",
        "latency": {
            "samples": 1,
            "stored_samples": 1,
            "truncated_samples": 0,
            "percentile_scope": "exact",
            "histogram": [],
            "min_ns": 1,
            "avg_ns": 1,
            "p95_ns": 1,
            "p99_ns": 1,
            "max_ns": 1,
            "over_1ms": 0,
            "over_2ms": 0,
            "over_5ms": 0
        },
        "cpu": {
            "busiest_cpu": null,
            "busiest_cpu_samples": 0,
            "worst_cpu": null,
            "worst_cpu_max_ns": 0,
            "spikiest_cpu": null,
            "spikiest_cpu_spikes": 0,
            "per_cpu": []
        },
        "top_spikes": []
    }))
    .unwrap();

    assert_eq!(task.process_starttime_ticks, None);
    assert_eq!(task.task_starttime_ticks, None);
}

#[test]
fn recorded_time_accepts_legacy_local_field() {
    let recorded: recorder::RecordedTime = serde_json::from_str(
        r#"{"unix_seconds":0,"unix_nanos":0,"local":"SystemTime { tv_sec: 0, tv_nsec: 0 }"}"#,
    )
    .unwrap();

    assert_eq!(
        recorded.system_time_debug,
        "SystemTime { tv_sec: 0, tv_nsec: 0 }"
    );
}
#[test]
fn scx_correlation_spike_event_serialization() {
    let event = SpikeEvent {
        elapsed_ms: Some(100),
        task: 123.into(),
        active: true,
        class: TaskClass::Game,
        process_pid: Some(123.into()),
        process_comm: "game".into(),
        comm: "game".to_owned(),
        cpu: 1,
        wakeup_target_cpu: 1,
        prio: 120,
        latency_ns: 1_000_000,
        wakeup_ns: 2000,
        switch_ns: 3000,
        major_faults: 1,
        minor_faults: 2,
        scx_ops: Some("scx_lavd".to_owned()),
        scx_state: Some("enabled".to_owned()),
        scx_enable_seq: Some("1".to_owned()),
        ..Default::default()
    };

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"scx_ops\":\"scx_lavd\""));
    assert!(json.contains("\"scx_state\":\"enabled\""));

    let deserialized: SpikeEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.scx_ops.as_deref(), Some("scx_lavd"));
    assert_eq!(deserialized.scx_state.as_deref(), Some("enabled"));
    assert_eq!(deserialized.scx_enable_seq.as_deref(), Some("1"));
}

#[test]
fn scx_correlation_backward_compatibility() {
    let json = r#"{"elapsed_ms":100,"task":123,"active":true,"class":"Game","process_pid":123,"process_comm":"game","comm":"game-main","cpu":1,"wakeup_target_cpu":1,"prio":120,"latency_ns":5000000,"wakeup_ns":100,"switch_ns":5000100,"target_pending_wakeups":0,"major_faults":0,"minor_faults":0}"#;
    let deserialized: SpikeEvent = serde_json::from_str(json).unwrap();
    assert_eq!(deserialized.scx_ops, None);
    assert_eq!(deserialized.scx_state, None);
    assert_eq!(deserialized.scx_enable_seq, None);
}
#[test]
fn session_file_serializes_metadata_core_flat() {
    let mut session = crate::recorder::SessionFile::default();
    session.core.schema_version = crate::recorder::ArtifactSchemaVersion::new(123);
    session.core.run_name = Some("flat-test".to_string());
    session.stop_reason = "test".to_string();

    let value = serde_json::to_value(&session).unwrap();

    assert_eq!(value["schema_version"], 123);
    assert_eq!(value["run_name"], "flat-test");
    assert_eq!(value["stop_reason"], "test");
    assert!(
        value.get("core").is_none(),
        "core must be flattened, not nested"
    );
}

#[test]
fn metadata_file_serializes_metadata_core_flat() {
    let mut metadata = crate::recorder::MetadataFile::default();
    metadata.core.schema_version = crate::recorder::ArtifactSchemaVersion::new(123);
    metadata.core.run_name = Some("flat-test".to_string());

    let value = serde_json::to_value(&metadata).unwrap();

    assert_eq!(value["schema_version"], 123);
    assert_eq!(value["run_name"], "flat-test");
    assert!(
        value.get("core").is_none(),
        "core must be flattened, not nested"
    );
}

#[test]
fn metadata_file_deserializes_flat_json_into_core() {
    let mut metadata = crate::recorder::MetadataFile::default();
    metadata.core.schema_version = crate::recorder::ArtifactSchemaVersion::new(123);
    metadata.core.run_name = Some("flat-test".to_string());

    let json = serde_json::to_value(&metadata).unwrap();

    let deserialized: crate::recorder::MetadataFile = serde_json::from_value(json).unwrap();

    assert_eq!(deserialized.core.schema_version, 123);
    assert_eq!(deserialized.core.run_name.as_deref(), Some("flat-test"));
}
