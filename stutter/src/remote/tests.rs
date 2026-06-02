use super::*;
use crate::{
    actions::SafetyClass,
    config::{FocusSource, ForegroundSource},
    daemon::policy::DaemonMode,
};

fn minimal_remote_monitor_request() -> RemoteMonitorRequest {
    RemoteMonitorRequest {
        target_pids: Vec::new(),
        tree_pids: Vec::new(),
        exclude_tree_pids: Vec::new(),
        duration_seconds: None,
        spike_us: None,
        summary_ms: None,
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
        record: false,
        run_name: None,
    }
}

#[test]
fn remote_monitor_request_round_trips_json() {
    let request = RemoteMonitorRequest {
        target_pids: vec![1234],
        tree_pids: vec![],
        exclude_tree_pids: vec![],
        duration_seconds: Some(5),
        spike_us: Some(1000),
        summary_ms: Some(500),
        include_comm: vec!["Game".to_string()],
        exclude_comm: vec![],
        hwmon: true,
        cpu_freq: true,
        faults: true,
        stat_wait: true,
        block_io: false,
        runtime_slices: true,
        runtime_slices_max_tasks: Some(64),
        irq_latency: false,
        irqs: vec![],
        foreground_window: true,
        focus_source: Some("hybrid".to_string()),
        foreground_source: Some("sway".to_string()),
        foreground_poll_ms: Some(1000),
        foreground_max_stale_ms: Some(2500),
        foreground_include_title: false,
        record: true,
        run_name: Some("test".to_string()),
    };

    let json = serde_json::to_string(&request).unwrap();
    let decoded: RemoteMonitorRequest = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded.target_pids, vec![1234]);
    assert_eq!(decoded.duration_seconds, Some(5));
    assert!(decoded.hwmon);
    assert!(decoded.runtime_slices);
    assert_eq!(decoded.runtime_slices_max_tasks, Some(64));
    assert!(decoded.foreground_window);
    assert_eq!(decoded.focus_source.as_deref(), Some("hybrid"));
    assert_eq!(decoded.foreground_source.as_deref(), Some("sway"));
    assert_eq!(decoded.foreground_poll_ms, Some(1000));
    assert_eq!(decoded.foreground_max_stale_ms, Some(2500));
    assert!(!decoded.foreground_include_title);
}

#[test]
fn remote_monitor_request_defaults_foreground_fields_for_old_clients() {
    let json = r#"{
         "target_pids": [],
         "tree_pids": [],
         "exclude_tree_pids": [],
         "duration_seconds": null,
         "spike_us": null,
         "summary_ms": null,
         "include_comm": [],
         "exclude_comm": [],
         "hwmon": false,
         "cpu_freq": false,
         "faults": false,
         "stat_wait": false,
         "block_io": false,
         "irq_latency": false,
         "irqs": [],
         "record": false,
         "run_name": null
     }"#;

    let decoded: RemoteMonitorRequest = serde_json::from_str(json).unwrap();

    assert!(!decoded.foreground_window);
    assert!(!decoded.runtime_slices);
    assert_eq!(decoded.runtime_slices_max_tasks, None);
    assert_eq!(decoded.focus_source, None);
    assert_eq!(decoded.foreground_source, None);
    assert_eq!(decoded.foreground_poll_ms, None);
    assert_eq!(decoded.foreground_max_stale_ms, None);
    assert!(!decoded.foreground_include_title);
}

#[test]
fn autotune_limits_default_to_low_risk_policy() {
    let limits = AgentAutotuneLimits::default();

    assert_eq!(limits.max_active_controllers, 1);
    assert_eq!(limits.max_mode, DaemonMode::ApplyLowRisk);
    assert_eq!(limits.max_safety_class, SafetyClass::ReversibleLowRisk);
    assert!(!limits.allow_high_risk);
    assert!(!limits.allow_system_wide_suggestions);
    assert!(!limits.allow_system_wide_apply);
}

#[test]
fn autotune_limits_accept_legacy_max_safety_class_json() {
    let json = r#"{
               "max_active_controllers": 1,
               "max_safety_class": "ReversibleLowRisk",
               "max_candidate_window_seconds": 120,
               "max_targets": 1,
               "allow_system_wide_suggestions": false,
               "allow_system_wide_apply": false
           }"#;

    let limits: AgentAutotuneLimits = serde_json::from_str(json).unwrap();

    assert_eq!(limits.max_mode, DaemonMode::ApplyLowRisk);
    assert_eq!(limits.max_safety_class, SafetyClass::ReversibleLowRisk);
    assert!(!limits.allow_high_risk);
}

#[test]
fn autotune_limits_accept_typed_max_mode_json() {
    let json = r#"{
               "max_active_controllers": 1,
               "max_mode": "apply-medium-risk",
               "max_safety_class": "ReversibleMediumRisk",
               "allow_high_risk": false,
               "max_candidate_window_seconds": 120,
               "max_targets": 1,
               "allow_system_wide_suggestions": false,
               "allow_system_wide_apply": false
           }"#;

    let limits: AgentAutotuneLimits = serde_json::from_str(json).unwrap();

    assert_eq!(limits.max_mode, DaemonMode::ApplyMediumRisk);
    assert_eq!(limits.max_safety_class, SafetyClass::ReversibleMediumRisk);
}

#[test]
fn remote_monitor_request_converts_to_monitor_config_layer() {
    let request = RemoteMonitorRequest {
        target_pids: vec![1234],
        tree_pids: vec![5678],
        exclude_tree_pids: vec![9999],
        duration_seconds: Some(5),
        spike_us: Some(750),
        summary_ms: Some(500),
        include_comm: vec!["Game".to_owned()],
        exclude_comm: vec!["steamwebhelper".to_owned()],
        hwmon: true,
        cpu_freq: true,
        faults: true,
        stat_wait: true,
        block_io: true,
        runtime_slices: true,
        runtime_slices_max_tasks: Some(64),
        irq_latency: true,
        irqs: vec![44],
        foreground_window: false,
        focus_source: Some("hybrid".to_owned()),
        foreground_source: Some("sway".to_owned()),
        foreground_poll_ms: Some(750),
        foreground_max_stale_ms: Some(3000),
        foreground_include_title: true,
        record: true,
        run_name: Some("remote-test".to_owned()),
    };

    let layer = request.into_monitor_config_layer().unwrap();

    assert_eq!(layer.target_pids, Some(vec![1234]));
    assert_eq!(layer.tree_pids, Some(vec![5678]));
    assert_eq!(layer.exclude_tree_pids, Some(vec![9999]));
    assert_eq!(
        layer.max_duration,
        Some(Some(std::time::Duration::from_secs(5)))
    );
    assert_eq!(layer.spike_threshold_ns, Some(750_000));
    assert_eq!(layer.summary_period_ms, Some(500));
    assert_eq!(layer.include_comm, Some(vec!["Game".to_owned()]));
    assert_eq!(layer.exclude_comm, Some(vec!["steamwebhelper".to_owned()]));
    assert_eq!(layer.hwmon, Some(true));
    assert_eq!(layer.cpu_freq, Some(true));
    assert_eq!(layer.faults, Some(true));
    assert_eq!(layer.stat_wait, Some(true));
    assert_eq!(layer.block_io, Some(true));
    assert_eq!(layer.runtime_slices, Some(true));
    assert_eq!(layer.runtime_slices_max_tasks, Some(64));
    assert_eq!(layer.irq_latency, Some(true));
    assert_eq!(layer.irqs, Some(vec![44]));
    assert_eq!(layer.focus_source, Some(FocusSource::Hybrid));
    assert_eq!(layer.foreground_window, Some(true));
    assert_eq!(layer.foreground_source, Some(ForegroundSource::Sway));
    assert_eq!(layer.foreground_poll_ms, Some(750));
    assert_eq!(layer.foreground_max_stale_ms, Some(3000));
    assert_eq!(layer.foreground_include_title, Some(true));
    assert_eq!(layer.run_name, Some(Some("remote-test".to_owned())));
    assert_eq!(layer.output_dir, None);
}

#[test]
fn remote_monitor_request_preserves_explicit_default_valued_layer_fields() {
    let request = RemoteMonitorRequest {
        target_pids: Vec::new(),
        tree_pids: Vec::new(),
        exclude_tree_pids: Vec::new(),
        duration_seconds: None,
        spike_us: Some(1_000),
        summary_ms: Some(1_000),
        include_comm: Vec::new(),
        exclude_comm: Vec::new(),
        hwmon: false,
        cpu_freq: false,
        faults: false,
        stat_wait: false,
        block_io: false,
        runtime_slices: false,
        runtime_slices_max_tasks: Some(256),
        irq_latency: false,
        irqs: Vec::new(),
        foreground_window: false,
        focus_source: Some("heuristic".to_owned()),
        foreground_source: Some("auto".to_owned()),
        foreground_poll_ms: Some(1_000),
        foreground_max_stale_ms: Some(2_500),
        foreground_include_title: false,
        record: false,
        run_name: None,
    };

    let layer = request.into_monitor_config_layer().unwrap();

    assert_eq!(layer.summary_period_ms, Some(1_000));
    assert_eq!(layer.spike_threshold_ns, Some(1_000_000));
    assert_eq!(layer.runtime_slices_max_tasks, Some(256));
    assert_eq!(layer.focus_source, Some(FocusSource::Heuristic));
    assert_eq!(layer.foreground_source, Some(ForegroundSource::Auto));
    assert_eq!(layer.foreground_poll_ms, Some(1_000));
    assert_eq!(layer.foreground_max_stale_ms, Some(2_500));
    assert_eq!(layer.foreground_window, None);
}

#[test]
fn remote_monitor_request_accepts_gnome_kde_and_plasma_foreground_sources() {
    for (value, expected) in [
        ("gnome", ForegroundSource::Gnome),
        ("kde", ForegroundSource::Kde),
        ("plasma", ForegroundSource::Kde),
    ] {
        let request = RemoteMonitorRequest {
            foreground_source: Some(value.to_owned()),
            ..minimal_remote_monitor_request()
        };

        let layer = request.into_monitor_config_layer().unwrap();

        assert_eq!(layer.foreground_source, Some(expected));
    }
}
