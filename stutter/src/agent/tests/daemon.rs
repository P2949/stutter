//! Daemon status, policy explanation, and restore response tests.

use super::{support::*, *};

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
        line.rule == "action:apply_low_risk_cpu_affinity:cooldown_gate" && line.outcome == "failed"
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
            diagnostic_current_raw_score_total: None,
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
        restored_records: 0,
        skipped_missing: 0,
        skipped_identity_mismatch: 0,
        failed_records: 0,
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
