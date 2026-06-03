//! Version, capability, artifact allowlist, and health-threshold response tests.

use super::*;

#[test]
fn version_response_reports_schema_version() {
    let resp = version_response();
    assert_eq!(resp.name, "stutter");
    assert!(resp.schema_version > 0);
}

#[test]
fn capabilities_report_limits_and_artifacts() {
    let state = AgentState {
        active_run: Mutex::new(None),
        active_autotune: Mutex::new(None),
        daemon_state: Mutex::new(DaemonState::default()),
        runs_dir: PathBuf::from("."),
        auth: AgentAuth::default(),
        bind: "127.0.0.1:9899".parse().unwrap(),
        unix_socket: None,
        limits: AgentLimits {
            max_duration_seconds: 123,
            max_targets: 456,
            max_concurrent_recordings: 1,
        },
        autotune_limits: AgentAutotuneLimits::default(),
        health_thresholds: SystemHealthThresholds::default(),
    };
    let resp = capabilities_response(&state);
    assert_eq!(resp.max_duration_seconds, 123);
    assert_eq!(resp.max_targets, 456);
    assert_eq!(resp.remote_access.remote_transport, "loopback_tcp");
    assert!(!resp.remote_access.remote_auth_configured);
    assert!(!resp.remote_access.remote_apply_enabled);
    assert!(
        resp.supported_artifacts
            .contains(&"session.json".to_owned())
    );
}

#[test]
fn capabilities_report_remote_access_status() {
    let mut state = AgentState {
        active_run: Mutex::new(None),
        active_autotune: Mutex::new(None),
        daemon_state: Mutex::new(DaemonState::default()),
        runs_dir: PathBuf::from("."),
        auth: AgentAuth {
            apply_token: Some("apply-secret".to_owned()),
            ..AgentAuth::default()
        },
        bind: "127.0.0.1:9899".parse().unwrap(),
        unix_socket: None,
        limits: AgentLimits {
            max_duration_seconds: 123,
            max_targets: 456,
            max_concurrent_recordings: 1,
        },
        autotune_limits: AgentAutotuneLimits::default(),
        health_thresholds: SystemHealthThresholds::default(),
    };

    let loopback = capabilities_response(&state);
    assert_eq!(loopback.remote_access.remote_transport, "loopback_tcp");
    assert!(loopback.remote_access.remote_auth_configured);
    assert!(loopback.remote_access.remote_apply_enabled);

    state.unix_socket = Some(PathBuf::from("/tmp/stutter-agent-test.sock"));
    let unix = capabilities_response(&state);
    assert_eq!(unix.remote_access.remote_transport, "unix_socket");
    assert!(unix.remote_access.remote_apply_enabled);

    state.unix_socket = None;
    state.bind = "0.0.0.0:9899".parse().unwrap();
    let unsafe_tcp = capabilities_response(&state);
    assert_eq!(unsafe_tcp.remote_access.remote_transport, "unsafe_tcp");
    assert!(unsafe_tcp.remote_access.remote_auth_configured);
    assert!(!unsafe_tcp.remote_access.remote_apply_enabled);

    state.bind = "127.0.0.1:9899".parse().unwrap();
    state.autotune_limits.max_mode = DaemonMode::Suggest;
    state.autotune_limits.max_safety_class = SafetyClass::ObserveOnly;
    let observe_only = capabilities_response(&state);
    assert_eq!(observe_only.remote_access.remote_transport, "loopback_tcp");
    assert!(!observe_only.remote_access.remote_apply_enabled);
}

#[test]
fn agent_health_monitor_uses_configured_thresholds() {
    let state = AgentState {
        active_run: Mutex::new(None),
        active_autotune: Mutex::new(None),
        daemon_state: Mutex::new(DaemonState::default()),
        runs_dir: PathBuf::from("."),
        auth: AgentAuth::default(),
        bind: "127.0.0.1:9899".parse().unwrap(),
        unix_socket: None,
        limits: AgentLimits {
            max_duration_seconds: 123,
            max_targets: 456,
            max_concurrent_recordings: 1,
        },
        autotune_limits: AgentAutotuneLimits::default(),
        health_thresholds: SystemHealthThresholds {
            max_cpu_temp_millidegrees: 70_000,
            max_gpu_temp_millidegrees: 71_000,
            min_disk_available_bytes: 3_000_000_000,
            max_memory_pressure_some_avg10_millipercent: 12_250,
            ..SystemHealthThresholds::default()
        },
    };

    let monitor = system_health_monitor_for_agent_state(&state);

    assert_eq!(monitor.thresholds().max_cpu_temp_millidegrees, 70_000);
    assert_eq!(monitor.thresholds().max_gpu_temp_millidegrees, 71_000);
    assert_eq!(monitor.thresholds().min_disk_available_bytes, 3_000_000_000);
    assert_eq!(
        monitor
            .thresholds()
            .max_memory_pressure_some_avg10_millipercent,
        12_250
    );
}

#[test]
fn agent_artifact_allowlist_includes_foreground_events() {
    assert!(AGENT_ARTIFACT_ALLOWLIST.contains(&"foreground_events.json"));
    assert!(validate_artifact_name("foreground_events.json").is_ok());
}
