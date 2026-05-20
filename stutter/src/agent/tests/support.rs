//! Shared state builders and request helpers for agent tests.

use super::*;

pub(super) fn minimal_remote_request() -> RemoteMonitorRequest {
    RemoteMonitorRequest {
        target_pids: vec![1234],
        tree_pids: Vec::new(),
        exclude_tree_pids: Vec::new(),
        duration_seconds: Some(5),
        spike_us: Some(1000),
        summary_ms: Some(500),
        include_comm: Vec::new(),
        exclude_comm: Vec::new(),
        hwmon: false,
        cpu_freq: false,
        faults: false,
        stat_wait: false,
        block_io: false,
        runtime_slices: false,
        runtime_slices_max_tasks: None,
        irq_latency: false,
        irqs: Vec::new(),
        foreground_window: false,
        focus_source: None,
        foreground_source: None,
        foreground_poll_ms: None,
        foreground_max_stale_ms: None,
        foreground_include_title: false,
        record: true,
        run_name: Some("remote-test".to_owned()),
    }
}

pub(super) fn test_health_thresholds_allow_apply() -> SystemHealthThresholds {
    SystemHealthThresholds {
        max_cpu_temp_millidegrees: i64::MAX,
        max_gpu_temp_millidegrees: i64::MAX,
        min_disk_available_bytes: 0,
        max_memory_pressure_some_avg10_millipercent: u32::MAX,
        max_load_per_cpu_milli: u32::MAX,
        max_ebpf_dropped_events: u64::MAX,
    }
}

pub(super) fn test_agent_state_custom(
    bind: SocketAddr,
    bearer_token: Option<String>,
) -> AgentState {
    AgentState {
        active_run: Mutex::new(None),
        active_autotune: Mutex::new(None),
        daemon_state: Mutex::new(DaemonState::default()),
        runs_dir: std::env::temp_dir(),
        auth: AgentAuth {
            bearer_token,
            ..AgentAuth::default()
        },
        bind,
        unix_socket: is_local_bind(&bind).then(|| PathBuf::from("/tmp/stutter-agent-test.sock")),
        limits: AgentLimits {
            max_duration_seconds: DEFAULT_AGENT_MAX_DURATION_SECONDS,
            max_targets: DEFAULT_AGENT_MAX_TARGETS,
            max_concurrent_recordings: DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
        },
        autotune_limits: AgentAutotuneLimits::default(),
        health_thresholds: test_health_thresholds_allow_apply(),
    }
}

pub(super) fn test_agent_state(bind: SocketAddr, token: Option<&str>) -> AgentState {
    AgentState {
        active_run: Mutex::new(None),
        active_autotune: Mutex::new(None),
        daemon_state: Mutex::new(DaemonState::default()),
        runs_dir: PathBuf::from("."),
        auth: AgentAuth {
            bearer_token: token.map(str::to_owned),
            ..AgentAuth::default()
        },
        bind,
        unix_socket: is_local_bind(&bind).then(|| PathBuf::from("/tmp/stutter-agent-test.sock")),
        limits: AgentLimits {
            max_duration_seconds: DEFAULT_AGENT_MAX_DURATION_SECONDS,
            max_targets: DEFAULT_AGENT_MAX_TARGETS,
            max_concurrent_recordings: DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
        },
        autotune_limits: AgentAutotuneLimits::default(),
        health_thresholds: test_health_thresholds_allow_apply(),
    }
}

pub(super) fn test_capabilities() -> DaemonCapabilities {
    DaemonCapabilities {
        kernel_release: Some("6.9.1-test".to_owned()),
        btf_available: true,
        sched_tracepoints_available: true,
        perf_permissions_likely: true,
        perf_event_paranoid: Some(1),
        cgroup_v2_available: true,
        sched_ext_available: false,
        uclamp_available: true,
        ionice_available: true,
        irq_affinity_available: false,
        gpu_sysfs_available: false,
        privileged_worker_socket_reachable: Some(false),
    }
}

pub(super) fn autotune_request(mode: &str) -> AutotuneStartRequest {
    AutotuneStartRequest {
        mode: mode.to_owned(),
        watch_process: Some("game".to_owned()),
        tree_pid: None,
        profiles: None,
        config: None,
        duration_seconds: Some(5),
        decision_log: None,
        summary_ms: None,
        preset: None,
        hwmon: false,
        mangohud_log: None,
        auto_focus: false,
        focus_source: None,
        foreground_window: false,
        foreground_source: None,
        foreground_poll_ms: None,
        foreground_max_stale_ms: None,
        washout_seconds: None,
        washout_verify_interval_ms: None,
    }
}

pub(super) fn autotune_headers(token: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(token) = token {
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
    }
    headers
}

pub(super) fn test_autotune_handle() -> AutotuneControllerHandle {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let join = tokio::spawn(async move {
        let _ = stop_rx.await;
        Ok(crate::autotune::runtime::AutotuneControllerExit {
            reason: "test stop".to_owned(),
            last_decision: None,
        })
    });

    AutotuneControllerHandle {
        mode: "observe".to_owned(),
        watch_process: Some("Game.exe".to_owned()),
        tree_pid: None,
        started_unix_nanos: crate::audit::unix_nanos_now(),
        stop_tx,
        join,
    }
}
