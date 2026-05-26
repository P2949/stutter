//! Agent response JSON schema snapshot tests.

use serde_json::{Value, json};

use super::{support::*, *};

#[test]
fn capabilities_response_schema_snapshot() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));

    assert_schema_snapshot(
        "capabilities",
        serde_json::to_value(capabilities_response(&state)).unwrap(),
        json!({
            "auth_required": "boolean",
            "features": {
                "autotune_apply_low_risk": "boolean",
                "autotune_observe": "boolean",
                "autotune_suggest": "boolean",
                "block_io_request": "boolean",
                "cpu_freq_request": "boolean",
                "download_artifacts": "boolean",
                "download_session": "boolean",
                "faults_request": "boolean",
                "foreground_window_request": "boolean",
                "hwmon_request": "boolean",
                "irq_latency_request": "boolean",
                "list_runs": "boolean",
                "record_start_stop": "boolean",
                "stat_wait_request": "boolean"
            },
            "max_concurrent_recordings": "number",
            "max_duration_seconds": "number",
            "max_targets": "number",
            "supported_artifacts": ["string"],
            "supported_routes": ["string"],
            "version": "string"
        }),
    );
}

#[test]
fn daemon_status_response_schema_snapshot() {
    let response = daemon_status_response(
        false,
        true,
        DaemonState::default(),
        test_capabilities(),
        SystemHealthSnapshot::default(),
    );

    assert_schema_snapshot(
        "daemon_status",
        serde_json::to_value(response).unwrap(),
        json!({
            "active_autotune": "boolean",
            "active_recording": "boolean",
            "capabilities": daemon_capabilities_schema(),
            "daemon_state": daemon_state_schema(),
            "health": system_health_schema(),
            "manual_restore_command": "string",
            "message": "string",
            "watchdog": daemon_watchdog_schema()
        }),
    );
}

#[tokio::test]
async fn autotune_start_rejection_response_schema_snapshot() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let mut request = autotune_request("observe");
    request.watch_process = None;
    request.tree_pid = None;
    request.auto_focus = false;

    let (status, value) = response_json(
        autotune_start_handler(State(state), HeaderMap::new(), Json(request))
            .await
            .into_response(),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_schema_snapshot(
        "autotune_start_rejection",
        value,
        json!({
            "error": "string"
        }),
    );
}

#[tokio::test]
async fn autotune_restore_response_schema_snapshot() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let stem = format!("stutter-agent-schema-{}", crate::audit::unix_nanos_now());
    let input = AutotuneRestoreCommandInput {
        journal_path: Some(std::env::temp_dir().join(format!("{stem}-journal.json"))),
        audit_path: Some(std::env::temp_dir().join(format!("{stem}-audit.jsonl"))),
        history_path: Some(std::env::temp_dir().join(format!("{stem}-history.jsonl"))),
        dry_run: true,
    };

    let (status, value) = response_json(autotune_restore_authorized(state, input).await).await;

    assert_eq!(status, StatusCode::OK);
    assert_schema_snapshot(
        "autotune_restore",
        value,
        json!({
            "failed_actions": "number",
            "failed_records": "number",
            "message": "string",
            "restore_messages": ["string"],
            "restored_actions": "number",
            "restored_records": "number",
            "skipped_actions": "number",
            "skipped_identity_mismatch": "number",
            "skipped_missing": "number",
            "status": "string"
        }),
    );
}

#[tokio::test]
async fn autotune_config_response_schema_snapshot() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let (status, value) = response_json(
        autotune_config_handler(State(state), HeaderMap::new())
            .await
            .into_response(),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_schema_snapshot(
        "autotune_config",
        value,
        json!({
            "allow_system_wide_apply": "boolean",
            "allow_system_wide_suggestions": "boolean",
            "apply_low_risk_remote_enabled": "boolean",
            "autotune_limits": agent_autotune_limits_schema(),
            "daemon_scope": "string",
            "default_mode": "string",
            "history_path": "string",
            "local_only_by_default": "boolean",
            "minimum_focus_confidence": "number",
            "required_stable_focus_polls": "number",
            "supported_modes": ["string"]
        }),
    );
}

async fn response_json(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

fn assert_schema_snapshot(name: &str, actual: Value, expected: Value) {
    assert_eq!(schema_snapshot(actual), expected, "{name} schema drifted");
}

fn schema_snapshot(value: Value) -> Value {
    match value {
        Value::Null => Value::String("null".to_owned()),
        Value::Bool(_) => Value::String("boolean".to_owned()),
        Value::Number(_) => Value::String("number".to_owned()),
        Value::String(_) => Value::String("string".to_owned()),
        Value::Array(values) => values
            .into_iter()
            .next()
            .map(|value| Value::Array(vec![schema_snapshot(value)]))
            .unwrap_or_else(|| Value::Array(Vec::new())),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, schema_snapshot(value)))
                .collect(),
        ),
    }
}

fn daemon_capabilities_schema() -> Value {
    json!({
        "btf_available": "boolean",
        "cgroup_v2_available": "boolean",
        "gpu_sysfs_available": "boolean",
        "ionice_available": "boolean",
        "irq_affinity_available": "boolean",
        "kernel_release": "string",
        "perf_event_paranoid": "number",
        "perf_permissions_likely": "boolean",
        "privileged_worker_socket_reachable": "boolean",
        "sched_ext_available": "boolean",
        "sched_tracepoints_available": "boolean",
        "uclamp_available": "boolean"
    })
}

fn daemon_state_schema() -> Value {
    json!({
        "active_experiment": "null",
        "active_rollback": "null",
        "active_target": "null",
        "cooldown_until_unix_nanos": "null",
        "degraded": [],
        "faulted": "null",
        "health": system_health_schema(),
        "last_decision": "null",
        "mode": "string",
        "phase": "string",
        "profile_memory": {
            "profiles": []
        },
        "schema_version": "number"
    })
}

fn system_health_schema() -> Value {
    json!({
        "inputs": {
            "ac_online": "null",
            "battery_present": "boolean",
            "cpu_count": "null",
            "disk_available_bytes": "null",
            "ebpf_dropped_events": "number",
            "load_average_1m_milli": "null",
            "max_cpu_temp_millidegrees": "null",
            "max_gpu_temp_millidegrees": "null",
            "memory_pressure_some_avg10_millipercent": "null",
            "probe_errors": [],
            "suspended_or_resumed": "boolean"
        },
        "issues": [],
        "ok_for_apply": "boolean",
        "reason_code": "null",
        "state": "string",
        "unix_nanos": "null"
    })
}

fn daemon_watchdog_schema() -> Value {
    json!({
        "issues": [],
        "ok": "boolean",
        "phase": "string",
        "recommended_actions": []
    })
}

fn agent_autotune_limits_schema() -> Value {
    json!({
        "allow_high_risk": "boolean",
        "allow_system_wide_apply": "boolean",
        "allow_system_wide_suggestions": "boolean",
        "max_active_controllers": "number",
        "max_candidate_window_seconds": "number",
        "max_mode": "string",
        "max_safety_class": "string",
        "max_targets": "number"
    })
}
