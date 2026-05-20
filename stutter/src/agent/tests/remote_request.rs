//! Remote monitor request validation and config conversion tests.

use super::{support::*, *};

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
