//! Agent module tests.

use super::{autotune::*, daemon::*, recording::*, routes::*, *};

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_remote_request() -> RemoteMonitorRequest {
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

    fn test_health_thresholds_allow_apply() -> SystemHealthThresholds {
        SystemHealthThresholds {
            max_cpu_temp_millidegrees: i64::MAX,
            max_gpu_temp_millidegrees: i64::MAX,
            min_disk_available_bytes: 0,
            max_memory_pressure_some_avg10_millipercent: u32::MAX,
            max_load_per_cpu_milli: u32::MAX,
            max_ebpf_dropped_events: u64::MAX,
        }
    }

    fn test_agent_state_custom(bind: SocketAddr, bearer_token: Option<String>) -> AgentState {
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
            unix_socket: is_local_bind(&bind)
                .then(|| PathBuf::from("/tmp/stutter-agent-test.sock")),
            limits: AgentLimits {
                max_duration_seconds: DEFAULT_AGENT_MAX_DURATION_SECONDS,
                max_targets: DEFAULT_AGENT_MAX_TARGETS,
                max_concurrent_recordings: DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
            },
            autotune_limits: AgentAutotuneLimits::default(),
            health_thresholds: test_health_thresholds_allow_apply(),
        }
    }

    fn test_agent_state(bind: SocketAddr, token: Option<&str>) -> AgentState {
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
            unix_socket: is_local_bind(&bind)
                .then(|| PathBuf::from("/tmp/stutter-agent-test.sock")),
            limits: AgentLimits {
                max_duration_seconds: DEFAULT_AGENT_MAX_DURATION_SECONDS,
                max_targets: DEFAULT_AGENT_MAX_TARGETS,
                max_concurrent_recordings: DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
            },
            autotune_limits: AgentAutotuneLimits::default(),
            health_thresholds: test_health_thresholds_allow_apply(),
        }
    }

    fn test_capabilities() -> DaemonCapabilities {
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

    fn autotune_request(mode: &str) -> AutotuneStartRequest {
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

    #[test]
    fn test_agent_state_uses_permissive_health_thresholds() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let monitor = system_health_monitor_for_agent_state(&state);
        let thresholds = monitor.thresholds();

        assert_eq!(thresholds.max_cpu_temp_millidegrees, i64::MAX);
        assert_eq!(thresholds.max_gpu_temp_millidegrees, i64::MAX);
        assert_eq!(thresholds.min_disk_available_bytes, 0);
        assert_eq!(
            thresholds.max_memory_pressure_some_avg10_millipercent,
            u32::MAX
        );
        assert_eq!(thresholds.max_load_per_cpu_milli, u32::MAX);
        assert_eq!(thresholds.max_ebpf_dropped_events, u64::MAX);
    }

    #[test]
    fn daemon_status_response_exposes_state_capabilities_and_restore_command() {
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Observe,
            ..DaemonState::default()
        };

        let response = daemon_status_response(
            false,
            true,
            state,
            test_capabilities(),
            SystemHealthSnapshot::default(),
        );

        assert!(!response.active_recording);
        assert!(response.active_autotune);
        assert_eq!(response.daemon_state.mode, DaemonMode::ApplyLowRisk);
        assert!(response.health.ok_for_apply);
        assert!(response.watchdog.ok);
        assert_eq!(
            response.capabilities.kernel_release.as_deref(),
            Some("6.9.1-test")
        );
        assert_eq!(
            response.manual_restore_command,
            "stutter daemon emergency-restore"
        );
        assert_eq!(response.message, "autotune controller active");
    }

    #[test]
    fn daemon_policy_from_faulted_state_keeps_mode_policy_explainable() {
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Faulted,
            ..DaemonState::default()
        };

        let policy = daemon_policy_from_state(&state);
        let explanation =
            policy.explain_action(PolicyIntent::Observe, &daemon_control_plane_descriptor());

        assert_eq!(policy.mode, DaemonMode::ApplyLowRisk);
        assert!(matches!(explanation.decision, PolicyDecisionKind::Allowed));
        assert_eq!(explanation.action_kind, "daemon-control-plane");
    }

    #[test]
    fn daemon_policy_response_reports_live_safety_context() {
        let health = SystemHealthSnapshot {
            ok_for_apply: false,
            reason_code: Some("cpu_overheated".to_owned()),
            ..SystemHealthSnapshot::default()
        };
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Decide,
            cooldown_until_unix_nanos: Some(crate::audit::unix_nanos_now() + 3_600_000_000_000),
            degraded: vec![crate::daemon::state::DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message: "insufficient samples".to_owned(),
            }],
            ..DaemonState::default()
        };

        let response = daemon_policy_response(state, test_capabilities(), health);

        assert_eq!(response.policy.mode, DaemonMode::ApplyLowRisk);
        assert!(matches!(
            response.explanation.decision,
            PolicyDecisionKind::Allowed
        ));
        assert!(!response.health.ok_for_apply);
        assert_eq!(
            response.health.reason_code.as_deref(),
            Some("cpu_overheated")
        );
        assert!(!response.watchdog.ok);
        assert_eq!(
            response.capabilities.kernel_release.as_deref(),
            Some("6.9.1-test")
        );
        assert_eq!(
            response.manual_restore_command,
            "stutter daemon emergency-restore"
        );
        assert!(response.policy_explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity:data_quality_gate"
                && line.outcome == "failed"
                && line.reason.contains("insufficient samples")
        }));
        assert!(response.policy_explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity:system_health_gate"
                && line.outcome == "failed"
                && line.reason.contains("cpu_overheated")
        }));
        assert!(response.policy_explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity:cooldown_gate"
                && line.outcome == "failed"
        }));
    }

    #[test]
    fn daemon_explain_response_reports_no_optimize_reasons_and_changes() {
        let state = DaemonState {
            mode: DaemonMode::Observe,
            phase: DaemonPhase::Paused,
            cooldown_until_unix_nanos: Some(42),
            active_target: Some(DaemonTargetState {
                root_pid: Some(1234),
                active_targets: 2,
                comm: Some("game".to_owned()),
            }),
            degraded: vec![crate::daemon::state::DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message: "insufficient samples".to_owned(),
            }],
            last_decision: Some(DaemonDecisionState {
                decision: "noop".to_owned(),
                reason: "insufficient data".to_owned(),
                unix_nanos: Some(1),
                score_total: None,
                candidate_count: None,
                top_denied_reason: None,
                planner: None,
                situation: None,
                focus_kind: None,
            }),
            ..DaemonState::default()
        };

        let response =
            daemon_explain_response(state, test_capabilities(), SystemHealthSnapshot::default());

        assert!(
            response
                .why_no_optimize
                .iter()
                .any(|reason| reason == "observe_only_mode")
        );
        assert!(
            response
                .why_no_optimize
                .iter()
                .any(|reason| reason == "daemon_paused")
        );
        assert!(
            response
                .why_no_optimize
                .iter()
                .any(|reason| reason.contains("insufficient samples"))
        );
        assert!(
            response
                .what_changed
                .iter()
                .any(|change| change == "phase:paused")
        );
        assert!(
            response
                .what_changed
                .iter()
                .any(|change| change.contains("active_workload:root_pid=1234"))
        );
        assert!(
            response
                .policy_explanation
                .lines
                .iter()
                .any(|line| line.rule == "action:observe_status")
        );
        assert!(response.policy_explanation.lines.iter().any(|line| {
            line.rule == "action:apply_low_risk_cpu_affinity:data_quality_gate"
                && line.outcome == "failed"
                && line.reason.contains("insufficient samples")
        }));
        assert_eq!(
            response.manual_restore_command,
            "stutter daemon emergency-restore"
        );
    }

    #[test]
    fn capabilities_response_lists_daemon_control_routes() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);

        let response = capabilities_response(&state);

        assert!(
            response
                .supported_routes
                .contains(&"/daemon/status".to_owned())
        );
        assert!(
            response
                .supported_routes
                .contains(&"/daemon/health".to_owned())
        );
        assert!(
            response
                .supported_routes
                .contains(&"/daemon/policy".to_owned())
        );
        assert!(
            response
                .supported_routes
                .contains(&"/daemon/explain".to_owned())
        );
        assert!(
            response
                .supported_routes
                .contains(&"/daemon/pause".to_owned())
        );
        assert!(
            response
                .supported_routes
                .contains(&"/daemon/resume".to_owned())
        );
        assert!(
            response
                .supported_routes
                .contains(&"/daemon/restore".to_owned())
        );
    }

    #[tokio::test]
    async fn daemon_pause_and_resume_handlers_update_in_memory_state() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

        let pause_response = daemon_pause_handler(State(state.clone()), HeaderMap::new())
            .await
            .into_response();
        assert_eq!(pause_response.status(), StatusCode::OK);
        assert_eq!(state.daemon_state.lock().await.phase, DaemonPhase::Paused);

        let resume_response = daemon_resume_handler(State(state.clone()), HeaderMap::new())
            .await
            .into_response();
        assert_eq!(resume_response.status(), StatusCode::OK);
        assert_eq!(state.daemon_state.lock().await.phase, DaemonPhase::Observe);
    }

    #[test]
    fn daemon_restore_messages_include_autotune_and_profile_restore_results() {
        let autotune = crate::autotune::emergency_restore::AutotuneRestoreOutcome {
            status: AutotuneRestoreStatus::Clean,
            restored_actions: 0,
            failed_actions: 0,
            skipped_actions: 0,
            messages: vec!["autotune restore: no active autotune action".to_owned()],
        };
        let profile = restore::ProfileRestoreCommandOutcome {
            messages: vec!["found profile restore state: affinity=0 nice=1 ionice=0".to_owned()],
            profile_nice_records: 1,
            ..restore::ProfileRestoreCommandOutcome::default()
        };

        let messages = daemon_restore_messages(&autotune, &profile);

        assert_eq!(messages.len(), 3);
        assert!(messages[0].contains("autotune restore"));
        assert!(messages[1].contains("profile restore state"));
        assert!(messages[2].contains("daemon restore summary"));
        assert!(messages[2].contains("status=Clean"));
        assert!(messages[2].contains("profile_found=true"));
        assert!(messages[2].contains("profile_restored=0"));
        assert!(messages[2].contains("profile_skipped_total=0"));
    }

    #[test]
    fn remote_autotune_observe_builds_observe_policy_without_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        let policy =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("observe"))
                .unwrap();

        assert_eq!(policy.mode, DaemonMode::Observe);
        assert_eq!(policy.source, ActionSource::RemoteAgent);
    }

    #[test]
    fn remote_autotune_apply_requires_configured_bearer_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        let rejection =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
                .unwrap_err();

        assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
        assert!(rejection.response_message.contains("bearer token"));
    }

    #[test]
    fn remote_autotune_apply_requires_valid_bearer_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let headers = HeaderMap::new();

        let rejection =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
                .unwrap_err();

        assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
        assert!(rejection.response_message.contains("valid bearer token"));
    }

    #[test]
    fn remote_autotune_all_apply_modes_require_bearer_token_before_limit_rejection() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        for mode in ["apply-low-risk", "apply-medium-risk", "apply-high-risk"] {
            let rejection =
                policy_for_remote_autotune_start(&headers, &state, &autotune_request(mode))
                    .unwrap_err();

            assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
            assert!(rejection.response_message.contains("bearer token"));
        }
    }

    #[test]
    fn remote_autotune_apply_requires_loopback_bind() {
        let state = test_agent_state("0.0.0.0:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );

        let rejection =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
                .unwrap_err();

        assert_eq!(rejection.status, StatusCode::FORBIDDEN);
        assert!(rejection.response_message.contains("loopback"));
    }

    #[test]
    fn remote_autotune_apply_low_risk_builds_low_risk_policy_with_valid_auth() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );

        let policy =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
                .unwrap();

        assert_eq!(policy.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(policy.source, ActionSource::RemoteAgent);
        assert!(!policy.allow_system_wide_suggestions);
        assert!(!policy.allow_system_wide_apply);
        assert!(!policy.allow_high_risk);
    }

    #[test]
    fn remote_autotune_apply_rejects_when_system_health_blocks_apply() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );
        let health = SystemHealthSnapshot {
            ok_for_apply: false,
            reason_code: Some("cpu_overheated".to_owned()),
            ..SystemHealthSnapshot::default()
        };

        let rejection = policy_for_remote_autotune_start_with_safety_context(
            &headers,
            &state,
            &autotune_request("apply-low-risk"),
            &DaemonState::default(),
            &health,
            &test_capabilities(),
        )
        .unwrap_err();

        assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
        assert!(rejection.response_message.contains("cpu_overheated"));
        assert!(rejection.audit_message.contains("cpu_overheated"));
    }

    #[test]
    fn remote_autotune_policy_build_is_deterministic_for_same_context() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );
        let request = autotune_request("apply-low-risk");
        let first_context =
            remote_policy_context_for_request(&state, authorize(&headers, &state.auth).is_ok());
        let second_context =
            remote_policy_context_for_request(&state, authorize(&headers, &state.auth).is_ok());

        let first = daemon_policy_for_remote_mode(
            DaemonMode::ApplyLowRisk,
            &state,
            &request,
            first_context,
        );
        let second = daemon_policy_for_remote_mode(
            DaemonMode::ApplyLowRisk,
            &state,
            &request,
            second_context,
        );

        assert_eq!(first, second);
        assert!(first.remote_apply.allow_remote_apply);
        assert_eq!(
            first.remote_apply.max_remote_targets,
            state.autotune_limits.max_targets
        );
    }

    #[test]
    fn remote_autotune_high_risk_rejected_by_default_limits() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );

        let rejection = policy_for_remote_autotune_start(
            &headers,
            &state,
            &autotune_request("apply-high-risk"),
        )
        .unwrap_err();

        assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
        assert!(
            rejection
                .response_message
                .contains("configured remote limits")
        );
    }

    #[test]
    fn autotune_start_rejects_wrong_bearer_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer wrong"),
        );

        let rejection =
            policy_for_remote_autotune_start(&headers, &state, &autotune_request("observe"))
                .unwrap_err();

        assert_eq!(rejection.status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn remote_autotune_observe_target_count_rejection_uses_policy_explanation() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );
        let mut request = autotune_request("observe");
        request.tree_pid = Some(1234);
        request.watch_process = Some("game".to_owned());

        let rejection = policy_for_remote_autotune_start(&headers, &state, &request).unwrap_err();

        assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
        assert!(rejection.response_message.contains("max_targets"));
        assert!(rejection.audit_message.contains("max_targets"));
    }

    #[test]
    fn remote_autotune_target_count_rejection_uses_policy_explanation() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );
        let mut request = autotune_request("apply-low-risk");
        request.tree_pid = Some(1234);
        request.watch_process = Some("game".to_owned());

        let rejection = policy_for_remote_autotune_start(&headers, &state, &request).unwrap_err();

        assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
        assert!(rejection.response_message.contains("max_targets"));
        assert!(rejection.audit_message.contains("max_targets"));
    }

    #[test]
    fn supported_remote_modes_derive_from_limits() {
        let limits = AgentAutotuneLimits::default();

        assert_eq!(
            supported_remote_mode_labels(&limits),
            vec![
                "observe".to_owned(),
                "suggest".to_owned(),
                "apply-low-risk".to_owned()
            ]
        );
    }

    #[tokio::test]
    async fn autotune_start_accepts_observe_mode_with_tree_pid() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "observe".to_owned(),
                watch_process: None,
                tree_pid: Some(1234),
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);

        let active = state.active_autotune.lock().await;
        assert!(active.is_some());
        assert_eq!(active.as_ref().unwrap().mode, "observe");
        assert_eq!(active.as_ref().unwrap().tree_pid, Some(1234));
    }

    #[tokio::test]
    async fn autotune_start_accepts_suggest_mode_with_watch_process() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "suggest".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);

        let active = state.active_autotune.lock().await;
        assert!(active.is_some());
        assert_eq!(active.as_ref().unwrap().mode, "suggest");
        assert_eq!(
            active.as_ref().unwrap().watch_process.as_deref(),
            Some("Game.exe")
        );
    }

    #[tokio::test]
    async fn autotune_start_allows_apply_low_risk_mode_with_valid_auth() {
        let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        state_value.auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
            ..AgentAuth::default()
        };
        let state = Arc::new(state_value);
        let response = autotune_start_handler(
            State(state.clone()),
            autotune_headers(Some("secret")),
            Json(AutotuneStartRequest {
                mode: "apply-low-risk".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);

        assert_eq!(status, StatusCode::OK, "body={body}");
        assert!(state.active_autotune.lock().await.is_some());
    }

    #[test]
    fn autotune_headers_test() {
        let headers = autotune_headers(Some("secret"));
        assert_eq!(
            headers.get(axum::http::header::AUTHORIZATION).unwrap(),
            "Bearer secret"
        );
    }

    fn autotune_headers(token: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(token) = token {
            headers.insert(
                axum::http::header::AUTHORIZATION,
                format!("Bearer {token}").parse().unwrap(),
            );
        }
        headers
    }

    fn test_autotune_handle() -> AutotuneControllerHandle {
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

    #[tokio::test]
    async fn autotune_status_includes_serializable_daemon_state_without_task_handles() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        *state.active_autotune.lock().await = Some(test_autotune_handle());
        *state.daemon_state.lock().await = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Apply,
            active_target: Some(DaemonTargetState {
                root_pid: Some(1234),
                active_targets: 1,
                comm: Some("Game.exe".to_owned()),
            }),
            active_experiment: Some(DaemonExperimentState {
                experiment_id: "experiment-1".to_owned(),
                action_id: "remote-autotune-start:apply-low-risk".to_owned(),
                candidate_name: Some("Game.exe".to_owned()),
                mode: DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: Some(100),
            }),
            active_rollback: Some(DaemonRollbackState {
                action_id: "remote-autotune-start:apply-low-risk".to_owned(),
                mode: DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::ReversibleLowRisk,
                rollback_available: false,
                token: None,
                manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            }),
            ..DaemonState::default()
        };

        let response = AutotuneStatusResponse {
            active: true,
            mode: Some("apply-low-risk".to_owned()),
            watch_process: Some("Game.exe".to_owned()),
            tree_pid: Some(1234),
            started_unix_nanos: Some(100),
            focus_group: None,
            target_root: Some(1234),
            current_score: None,
            active_profile: None,
            last_decision: None,
            rollback_available: false,
            cooldown_remaining_seconds: None,
            data_quality: None,
            last_fault: None,
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            daemon_state: state.daemon_state.lock().await.clone(),
            message: "test".to_owned(),
        };

        let json = serde_json::to_string(&response).unwrap();

        assert!(json.contains("\"daemon_state\""));
        assert!(json.contains("\"active_experiment\""));
        assert!(json.contains("\"active_rollback\""));
        assert!(!json.contains("stop_tx"));
        assert!(!json.contains("join"));
    }

    #[tokio::test]
    async fn autotune_start_updates_daemon_state_with_target_experiment_and_rollback_availability()
    {
        let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        state_value.auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
            ..AgentAuth::default()
        };
        let state = Arc::new(state_value);

        let response = autotune_start_handler(
            State(state.clone()),
            autotune_headers(Some("secret")),
            Json(AutotuneStartRequest {
                mode: "apply-low-risk".to_owned(),
                watch_process: None,
                tree_pid: Some(1234),
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);

        assert_eq!(status, StatusCode::OK, "body={body}");

        let daemon_state = state.daemon_state.lock().await.clone();
        assert_eq!(daemon_state.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(daemon_state.phase, DaemonPhase::Apply);
        assert_eq!(
            daemon_state
                .active_target
                .as_ref()
                .and_then(|target| target.root_pid),
            Some(1234)
        );
        assert_eq!(
            daemon_state
                .active_experiment
                .as_ref()
                .map(|experiment| experiment.safety_class.clone()),
            Some(SafetyClass::ReversibleLowRisk)
        );
        assert_eq!(
            daemon_state
                .active_rollback
                .as_ref()
                .map(|rollback| rollback.rollback_available),
            Some(false)
        );
        assert_eq!(
            daemon_state
                .active_rollback
                .as_ref()
                .and_then(|rollback| rollback.manual_restore_command.as_deref()),
            Some("stutter daemon emergency-restore")
        );
        assert!(daemon_state.faulted.is_none());
    }

    #[tokio::test]
    async fn autotune_policy_rejection_updates_faulted_daemon_state() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "apply-low-risk".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let daemon_state = state.daemon_state.lock().await.clone();
        assert_eq!(daemon_state.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(daemon_state.phase, DaemonPhase::Faulted);
        assert!(
            daemon_state
                .faulted
                .as_ref()
                .map(|fault| fault.reason.contains("bearer token"))
                .unwrap_or(false)
        );
    }

    #[tokio::test]
    async fn autotune_policy_rejection_preserves_active_rollback_state() {
        let state = Arc::new(test_agent_state(
            "127.0.0.1:0".parse().unwrap(),
            Some("secret"),
        ));
        {
            let mut daemon_state = state.daemon_state.lock().await;
            daemon_state.mode = DaemonMode::ApplyLowRisk;
            daemon_state.phase = DaemonPhase::Rollback;
            daemon_state.active_rollback = Some(DaemonRollbackState {
                action_id: "cpu-affinity:game".to_owned(),
                mode: DaemonMode::ApplyLowRisk,
                safety_class: SafetyClass::ReversibleLowRisk,
                rollback_available: true,
                token: None,
                manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            });
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret"),
        );

        let response = autotune_start_handler(
            State(state.clone()),
            headers,
            Json(AutotuneStartRequest {
                mode: "apply-low-risk".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let daemon_state = state.daemon_state.lock().await.clone();
        assert_eq!(daemon_state.phase, DaemonPhase::Faulted);
        assert_eq!(
            daemon_state
                .active_rollback
                .as_ref()
                .map(|rollback| rollback.action_id.as_str()),
            Some("cpu-affinity:game")
        );
        assert!(
            daemon_state
                .faulted
                .as_ref()
                .and_then(|fault| fault.manual_restore_command.as_deref())
                .is_some_and(|command| command.contains("emergency-restore"))
        );
    }

    #[tokio::test]
    async fn autotune_start_apply_low_risk_without_auth_is_rejected_before_mode_validation() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "apply-low-risk".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_start_apply_low_risk_on_non_loopback_is_rejected_even_with_valid_token() {
        let mut state_value = test_agent_state("0.0.0.0:9899".parse().unwrap(), Some("secret"));
        state_value.auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
            ..AgentAuth::default()
        };
        state_value.bind = "0.0.0.0:9899".parse().unwrap();
        let state = Arc::new(state_value);

        let response = autotune_start_handler(
            State(state.clone()),
            autotune_headers(Some("secret")),
            Json(AutotuneStartRequest {
                mode: "apply-low-risk".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_start_rejects_request_above_candidate_window_cap() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "observe".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(121),
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
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_start_rejects_more_than_one_target() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "observe".to_owned(),
                watch_process: Some("Game.exe".to_owned()),
                tree_pid: Some(1234),
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_start_requires_target_selector() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutotuneStartRequest {
                mode: "observe".to_owned(),
                watch_process: None,
                tree_pid: None,
                profiles: None,
                config: None,
                duration_seconds: Some(30),
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
            }),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(state.active_autotune.lock().await.is_none());
    }

    #[tokio::test]
    async fn autotune_stop_clears_active_session() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
        *state.active_autotune.lock().await = Some(test_autotune_handle());

        let response = autotune_stop_handler(State(state.clone()), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(state.active_autotune.lock().await.is_none());

        let daemon_state = state.daemon_state.lock().await.clone();
        assert_eq!(daemon_state.phase, DaemonPhase::Disabled);
        assert_eq!(
            daemon_state
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("autotune_stopped")
        );
    }

    #[tokio::test]
    async fn autotune_restore_is_safe_noop_until_remote_apply_exists() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

        let response = autotune_restore_handler(State(state), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn autotune_config_response_includes_limits() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let response = AutotuneConfigResponse {
            default_mode: "observe".to_owned(),
            supported_modes: vec!["observe".to_owned(), "suggest".to_owned()],
            apply_low_risk_remote_enabled: false,
            local_only_by_default: true,
            history_path: crate::autotune::history::default_autotune_history_path()
                .display()
                .to_string(),
            autotune_limits: state.autotune_limits.clone(),
            daemon_scope: "focused".to_owned(),
            allow_system_wide_suggestions: false,
            allow_system_wide_apply: false,
            minimum_focus_confidence: 0.70,
            required_stable_focus_polls: 3,
        };

        assert_eq!(response.autotune_limits, AgentAutotuneLimits::default());
        assert_eq!(response.autotune_limits.max_active_controllers, 1);
        assert_eq!(
            response.autotune_limits.max_safety_class,
            SafetyClass::ReversibleLowRisk
        );
        assert_eq!(response.autotune_limits.max_candidate_window_seconds, 120);
        assert_eq!(response.autotune_limits.max_targets, 1);
        assert!(!response.autotune_limits.allow_system_wide_suggestions);
        assert!(!response.autotune_limits.allow_system_wide_apply);
    }

    #[tokio::test]
    async fn autotune_config_reports_apply_low_risk_disabled() {
        let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

        let response = autotune_config_handler(State(state), HeaderMap::new())
            .await
            .into_response();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn capabilities_includes_autotune_routes() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        let resp = capabilities_response(&state);
        assert!(
            resp.supported_routes
                .contains(&"/autotune/start".to_owned())
        );
        assert!(
            resp.supported_routes
                .contains(&"/autotune/status".to_owned())
        );
        assert!(resp.supported_routes.contains(&"/autotune/stop".to_owned()));
    }

    #[tokio::test]
    async fn autotune_start_accepts_system_wide_suggestions_by_default() {
        let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        state_value.autotune_limits.allow_system_wide_suggestions = true;
        state_value.autotune_limits.allow_system_wide_apply = false;
        let state = Arc::new(state_value);

        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(autotune_request("observe")),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn autotune_start_rejects_system_wide_apply_by_default() {
        let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
        state_value.autotune_limits.allow_system_wide_suggestions = false;
        state_value.autotune_limits.allow_system_wide_apply = true;
        let state = Arc::new(state_value);

        let response = autotune_start_handler(
            State(state.clone()),
            HeaderMap::new(),
            Json(autotune_request("observe")),
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_id_rejects_path_traversal() {
        assert!(validate_id("../evil").is_err());
        assert!(validate_id("foo/bar").is_err());
        assert!(validate_id("run-123").is_ok());
    }

    #[test]
    fn artifact_allowlist_rejects_unknown_file() {
        assert!(validate_artifact_name("shadow").is_err());
        assert!(validate_artifact_name("session.json.bak").is_err());
    }

    #[test]
    fn artifact_allowlist_rejects_path_traversal() {
        assert!(validate_artifact_name("../session.json").is_err());
    }

    #[test]
    fn artifact_allowlist_accepts_known_artifacts() {
        assert!(validate_artifact_name("session.json").is_ok());
        assert!(validate_artifact_name("metadata.json").is_ok());
        assert!(validate_artifact_name("spike_events.json").is_ok());
    }

    #[test]
    fn local_bind_allowed_without_unsafe_flag() {
        let addr: SocketAddr = "127.0.0.1:9899".parse().unwrap();
        assert!(validate_agent_bind_policy(addr, false).is_ok());
    }

    #[test]
    fn non_loopback_bind_rejected_without_unsafe_flag() {
        let addr: SocketAddr = "0.0.0.0:9899".parse().unwrap();
        assert!(validate_agent_bind_policy(addr, false).is_err());
    }

    #[test]
    fn non_loopback_bind_allowed_with_unsafe_flag() {
        let addr: SocketAddr = "0.0.0.0:9899".parse().unwrap();
        assert!(validate_agent_bind_policy(addr, true).is_ok());
    }

    #[test]
    fn unix_socket_state_counts_as_local() {
        let mut state = test_agent_state("0.0.0.0:9899".parse().unwrap(), None);
        assert!(!agent_state_is_local(&state));

        state.unix_socket = Some(PathBuf::from("/tmp/stutter-agent.sock"));
        assert!(agent_state_is_local(&state));
    }

    #[test]
    fn listen_audit_message_reports_unix_socket() {
        let config = AgentConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            unix_socket: Some(PathBuf::from("/tmp/stutter-agent.sock")),
            runs_dir: PathBuf::from("/tmp/stutter-runs"),
            allow_unsafe_bind: false,
            bearer_token: None,
            read_token: None,
            apply_token: None,
            max_duration_seconds: DEFAULT_AGENT_MAX_DURATION_SECONDS,
            max_targets: DEFAULT_AGENT_MAX_TARGETS,
            max_concurrent_recordings: DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
            autotune_limits: AgentAutotuneLimits::default(),
            health_thresholds: SystemHealthThresholds::default(),
            rollback_on_crash_recovery: true,
        };

        let message = agent_listen_audit_message(&config, false);
        assert!(message.contains("bind=unix:/tmp/stutter-agent.sock"));
        assert!(message.contains("auth_enabled=false"));
    }

    fn validate_agent_bind_policy(bind: SocketAddr, allow_unsafe_bind: bool) -> anyhow::Result<()> {
        if !is_local_bind(&bind) && !allow_unsafe_bind {
            anyhow::bail!("refusing to bind");
        }
        Ok(())
    }

    #[test]
    fn normalize_bearer_token_trims_newline() {
        assert_eq!(
            normalize_bearer_token("  secret\n  ".to_owned()).unwrap(),
            Some("secret".to_owned())
        );
    }

    #[test]
    fn normalize_bearer_token_rejects_empty() {
        assert!(normalize_bearer_token("  ".to_owned()).is_err());
    }

    #[test]
    fn authorize_allows_without_configured_token() {
        let auth = AgentAuth::default();
        let headers = HeaderMap::new();
        assert!(authorize(&headers, &auth).is_ok());
    }

    #[test]
    fn authorize_rejects_missing_bearer_token() {
        let auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
            ..AgentAuth::default()
        };
        let headers = HeaderMap::new();
        assert_eq!(authorize(&headers, &auth), Err(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn authorize_rejects_wrong_bearer_token() {
        let auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
            ..AgentAuth::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer wrong".parse().unwrap(),
        );
        assert_eq!(authorize(&headers, &auth), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn authorize_accepts_correct_bearer_token() {
        let auth = AgentAuth {
            bearer_token: Some("secret".to_owned()),
            ..AgentAuth::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );
        assert!(authorize(&headers, &auth).is_ok());
    }

    #[test]
    fn read_token_can_read_but_cannot_apply() {
        let auth = AgentAuth {
            read_token: Some("read-secret".to_owned()),
            apply_token: Some("apply-secret".to_owned()),
            ..AgentAuth::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer read-secret".parse().unwrap(),
        );

        assert!(authorize(&headers, &auth).is_ok());
        assert_eq!(authorize_apply(&headers, &auth), Err(StatusCode::FORBIDDEN));
    }

    #[test]
    fn apply_token_can_read_and_apply() {
        let auth = AgentAuth {
            read_token: Some("read-secret".to_owned()),
            apply_token: Some("apply-secret".to_owned()),
            ..AgentAuth::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer apply-secret".parse().unwrap(),
        );

        assert!(authorize(&headers, &auth).is_ok());
        assert!(authorize_apply(&headers, &auth).is_ok());
    }

    #[test]
    fn tcp_state_change_requires_apply_token() {
        let mut state = test_agent_state("127.0.0.1:9899".parse().unwrap(), None);
        state.unix_socket = None;

        assert_eq!(
            authorize_state_change(&HeaderMap::new(), &state),
            Err(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn unix_socket_state_change_allows_socket_credentials_without_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);

        assert!(state.unix_socket.is_some());
        assert!(authorize_state_change(&HeaderMap::new(), &state).is_ok());
    }

    #[test]
    fn unix_socket_state_change_respects_configured_apply_token() {
        let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );

        assert!(state.unix_socket.is_some());
        assert_eq!(
            authorize_state_change(&HeaderMap::new(), &state),
            Err(StatusCode::UNAUTHORIZED)
        );
        assert!(authorize_state_change(&headers, &state).is_ok());
    }

    #[test]
    fn remote_tcp_privileged_operation_is_rejected_even_with_apply_token() {
        let state = test_agent_state("0.0.0.0:9899".parse().unwrap(), Some("secret"));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );

        assert_eq!(
            authorize_agent_privileged_operation(
                &headers,
                &state,
                PrivilegedOperation::StartRecording
            ),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn loopback_tcp_privileged_operation_requires_apply_scope() {
        let mut state = test_agent_state("127.0.0.1:9899".parse().unwrap(), None);
        state.unix_socket = None;
        state.auth = AgentAuth {
            read_token: Some("read-secret".to_owned()),
            apply_token: Some("apply-secret".to_owned()),
            ..AgentAuth::default()
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer read-secret".parse().unwrap(),
        );

        assert_eq!(
            authorize_agent_privileged_operation(
                &headers,
                &state,
                PrivilegedOperation::ControlDaemon
            ),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn agent_privilege_transport_prefers_unix_socket_when_configured() {
        let mut state = test_agent_state("0.0.0.0:9899".parse().unwrap(), None);
        state.unix_socket = Some(PathBuf::from("/tmp/stutter-agent-test.sock"));

        assert_eq!(
            agent_privilege_transport(&state),
            PrivilegeTransport::UnixSocket
        );
    }

    #[tokio::test]
    async fn agent_rate_limiter_drops_until_window_expires() {
        let limiter = AgentRateLimiter::new(2, Duration::from_secs(10));
        let now = Instant::now();

        assert!(limiter.accept(now).await);
        assert!(limiter.accept(now + Duration::from_secs(1)).await);
        assert!(!limiter.accept(now + Duration::from_secs(2)).await);
        assert!(limiter.accept(now + Duration::from_secs(11)).await);
    }

    #[test]
    fn remote_request_rejects_duration_above_limit() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let mut request = minimal_remote_request();
        request.target_pids = vec![1];
        request.duration_seconds = Some(120);
        request.summary_ms = Some(1000);
        request.run_name = None;
        assert!(validate_remote_request_limits(&request, &limits).is_err());

        request.duration_seconds = Some(30);
        assert!(validate_remote_request_limits(&request, &limits).is_ok());
    }

    #[test]
    fn remote_request_rejects_too_many_targets() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 2,
            max_concurrent_recordings: 1,
        };
        let mut request = minimal_remote_request();
        request.target_pids = vec![1, 2, 3];
        request.duration_seconds = Some(30);
        request.summary_ms = Some(1000);
        request.run_name = None;
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn remote_request_rejects_zero_summary_ms() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let mut request = minimal_remote_request();
        request.target_pids = vec![1];
        request.duration_seconds = Some(30);
        request.summary_ms = Some(0);
        request.run_name = None;
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn remote_request_rejects_zero_spike_us() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let mut request = minimal_remote_request();
        request.target_pids = vec![1];
        request.duration_seconds = Some(30);
        request.spike_us = Some(0);
        request.summary_ms = Some(1000);
        request.run_name = None;
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

    #[test]
    fn remote_request_rejects_irq_latency_without_irqs() {
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 10,
            max_concurrent_recordings: 1,
        };
        let mut request = minimal_remote_request();
        request.target_pids = vec![1];
        request.duration_seconds = Some(30);
        request.summary_ms = Some(1000);
        request.irq_latency = true;
        request.run_name = None;
        assert!(validate_remote_request_limits(&request, &limits).is_err());
    }

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
        assert!(
            resp.supported_artifacts
                .contains(&"session.json".to_owned())
        );
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

    #[test]
    fn validate_remote_request_accepts_foreground_defaults() {
        let request = minimal_remote_request();
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        validate_remote_request_limits(&request, &limits).unwrap();
        assert!(!remote_foreground_enabled(&request).unwrap());
    }

    #[test]
    fn validate_remote_request_rejects_invalid_focus_source() {
        let mut request = minimal_remote_request();
        request.focus_source = Some("dbus".to_owned());
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let err = validate_remote_request_limits(&request, &limits)
            .unwrap_err()
            .to_string();

        assert!(err.contains("focus_source must be heuristic, foreground, or hybrid"));
    }

    #[test]
    fn validate_remote_request_rejects_invalid_foreground_source() {
        let mut request = minimal_remote_request();
        request.foreground_source = Some("gnome".to_owned());
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let err = validate_remote_request_limits(&request, &limits)
            .unwrap_err()
            .to_string();

        assert!(err.contains("foreground_source must be auto, sway, hyprland, or x11"));
    }

    #[test]
    fn validate_remote_request_rejects_too_fast_foreground_poll() {
        let mut request = minimal_remote_request();
        request.foreground_window = true;
        request.foreground_poll_ms = Some(99);
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let err = validate_remote_request_limits(&request, &limits)
            .unwrap_err()
            .to_string();

        assert!(err.contains("foreground_poll_ms must be >= 100"));
    }

    #[test]
    fn monitor_config_from_remote_request_applies_foreground_fields() {
        let mut request = minimal_remote_request();
        request.foreground_window = true;
        request.focus_source = Some("hybrid".to_owned());
        request.foreground_source = Some("sway".to_owned());
        request.foreground_poll_ms = Some(750);
        request.foreground_max_stale_ms = Some(3000);
        request.foreground_include_title = false;

        let dir = std::env::temp_dir().join("stutter-agent-foreground-config-test");
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let config = monitor_config_from_remote_request(&request, &dir, &limits).unwrap();

        assert!(config.focus.foreground_window);
        assert_eq!(config.focus.focus_source, FocusSource::Hybrid);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
        assert_eq!(config.focus.foreground_poll_ms, 750);
        assert_eq!(config.focus.foreground_max_stale_ms, 3000);
        assert!(!config.focus.foreground_include_title);
    }

    #[test]
    fn monitor_config_from_remote_request_enables_foreground_window_for_non_heuristic_focus() {
        let mut request = minimal_remote_request();
        request.foreground_window = false;
        request.focus_source = Some("foreground".to_owned());

        let dir = std::env::temp_dir().join("stutter-agent-foreground-focus-test");
        let limits = AgentLimits {
            max_duration_seconds: 60,
            max_targets: 4,
            max_concurrent_recordings: 1,
        };

        let config = monitor_config_from_remote_request(&request, &dir, &limits).unwrap();

        assert!(config.focus.foreground_window);
        assert_eq!(config.focus.focus_source, FocusSource::Foreground);
    }

    #[test]
    fn foreground_title_capture_allowed_on_loopback_without_token() {
        let mut request = minimal_remote_request();
        request.foreground_include_title = true;
        let state = test_agent_state_custom("127.0.0.1:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        assert!(validate_foreground_title_capture_security(&request, &state, &headers).is_ok());
    }

    #[test]
    fn foreground_title_capture_rejected_on_non_loopback_without_token() {
        let mut request = minimal_remote_request();
        request.foreground_include_title = true;
        let state = test_agent_state_custom("0.0.0.0:0".parse().unwrap(), None);
        let headers = HeaderMap::new();

        assert_eq!(
            validate_foreground_title_capture_security(&request, &state, &headers),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn foreground_title_capture_allowed_on_non_loopback_with_valid_bearer_token() {
        let mut request = minimal_remote_request();
        request.foreground_include_title = true;
        let state =
            test_agent_state_custom("0.0.0.0:0".parse().unwrap(), Some("secret".to_owned()));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer secret".parse().unwrap(),
        );

        assert!(validate_foreground_title_capture_security(&request, &state, &headers).is_ok());
    }

    #[test]
    fn foreground_title_capture_rejected_on_non_loopback_with_invalid_bearer_token() {
        let mut request = minimal_remote_request();
        request.foreground_include_title = true;
        let state =
            test_agent_state_custom("0.0.0.0:0".parse().unwrap(), Some("secret".to_owned()));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer wrong".parse().unwrap(),
        );

        assert_eq!(
            validate_foreground_title_capture_security(&request, &state, &headers),
            Err(StatusCode::FORBIDDEN)
        );
    }
}
#[cfg(test)]
mod remote_policy_tests {
    use axum::http::{HeaderMap, StatusCode};

    use super::*;
    use crate::{
        actions::SafetyClass,
        daemon_policy::DaemonMode,
        remote::{AgentAutotuneLimits, AutotuneStartRequest},
    };

    fn test_agent_state(token: Option<&str>, bind_loopback: bool) -> AgentState {
        AgentState {
            active_run: Mutex::new(None),
            active_autotune: Mutex::new(None),
            daemon_state: Mutex::new(DaemonState::default()),
            runs_dir: PathBuf::from("/tmp"),
            auth: AgentAuth {
                bearer_token: token.map(str::to_owned),
                ..AgentAuth::default()
            },
            bind: if bind_loopback {
                "127.0.0.1:0".parse().unwrap()
            } else {
                "0.0.0.0:0".parse().unwrap()
            },
            unix_socket: None,
            limits: AgentLimits {
                max_duration_seconds: 300,
                max_targets: 128,
                max_concurrent_recordings: 1,
            },
            autotune_limits: AgentAutotuneLimits {
                max_active_controllers: 1,
                max_mode: DaemonMode::ApplyLowRisk,
                max_safety_class: SafetyClass::ReversibleLowRisk,
                allow_high_risk: false,
                max_candidate_window_seconds: 60,
                max_targets: 10,
                allow_system_wide_suggestions: false,
                allow_system_wide_apply: false,
            },
            health_thresholds: SystemHealthThresholds::default(),
        }
    }

    fn autotune_start_request(mode: &str) -> AutotuneStartRequest {
        AutotuneStartRequest {
            mode: mode.to_owned(),
            watch_process: None,
            tree_pid: Some(1234),
            profiles: None,
            config: None,
            duration_seconds: Some(30),
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

    #[test]
    fn remote_policy_rejects_apply_without_bearer_token() {
        let state = test_agent_state(Some("secret"), true);
        let request = autotune_start_request("apply-low-risk");

        let res = policy_for_remote_autotune_start(&HeaderMap::new(), &state, &request);
        assert_eq!(res.unwrap_err().status, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer wrong".parse().unwrap());
        let res = policy_for_remote_autotune_start(&headers, &state, &request);
        assert_eq!(res.unwrap_err().status, StatusCode::FORBIDDEN);

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer secret".parse().unwrap());
        let res = policy_for_remote_autotune_start(&headers, &state, &request);
        assert!(res.is_ok());
    }

    #[test]
    fn remote_policy_rejects_apply_on_non_loopback_bind() {
        let state = test_agent_state(Some("secret"), false);
        let request = autotune_start_request("apply-low-risk");

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer secret".parse().unwrap());

        let res = policy_for_remote_autotune_start(&headers, &state, &request);
        assert_eq!(res.unwrap_err().status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn remote_policy_response_uses_policy_explanation_text() {
        let state = test_agent_state(Some("secret"), false);
        let request = autotune_start_request("apply-low-risk");

        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer secret".parse().unwrap());

        let err = policy_for_remote_autotune_start(&headers, &state, &request).unwrap_err();

        assert_eq!(err.status, StatusCode::FORBIDDEN);
        assert!(err.response_message.contains("loopback"));
        assert!(err.audit_message.contains("loopback"));
    }
}
