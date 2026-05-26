use super::*;
use crate::foreground::{ForegroundEvent, ForegroundEventInput};

#[test]
fn foreground_event_serializes_without_title_by_default() {
    let event = ForegroundEvent::new(ForegroundEventInput {
        elapsed_ms: 1_000,
        source: crate::foreground::ForegroundSource::Sway,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("steam_app_379430".to_owned()),
        class: Some("steam_app_379430".to_owned()),
        title: Some("Private game or browser title".to_owned()),
        include_title: false,
        window_id: Some("7".to_owned()),
        workspace: Some("gaming".to_owned()),
        confidence: 0.95,
        stale_ms: None,
        reason: "focused Sway node from swaymsg get_tree".to_owned(),
    });

    let value = serde_json::to_value(&event).unwrap();
    let decision = value.get("decision").unwrap();
    let target = decision.get("target").unwrap();

    assert_eq!(
        value.get("elapsed_ms").and_then(serde_json::Value::as_u64),
        Some(1_000)
    );
    assert_eq!(
        value.get("source").and_then(serde_json::Value::as_str),
        Some("sway")
    );
    assert_eq!(
        value.get("status").and_then(serde_json::Value::as_str),
        Some("available")
    );
    assert_eq!(
        target.get("pid").and_then(serde_json::Value::as_u64),
        Some(4242)
    );
    assert_eq!(
        target.get("app_id").and_then(serde_json::Value::as_str),
        Some("steam_app_379430")
    );
    assert_eq!(
        target.get("class").and_then(serde_json::Value::as_str),
        Some("steam_app_379430")
    );
    assert!(target.get("title").unwrap().is_null());
    assert_eq!(
        target.get("workspace").and_then(serde_json::Value::as_str),
        Some("gaming")
    );
}

#[test]
fn foreground_event_serializes_expected_fields() {
    let event = ForegroundEvent::new(ForegroundEventInput {
        elapsed_ms: 1234,
        source: crate::foreground::ForegroundSource::Sway,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("steam".to_owned()),
        class: Some("Steam".to_owned()),
        title: None,
        include_title: false,
        window_id: Some("123".to_owned()),
        workspace: Some("games".to_owned()),
        confidence: 0.95,
        stale_ms: None,
        reason: "focused Sway node from swaymsg get_tree".to_owned(),
    });

    let value = serde_json::to_value(&event).unwrap();
    let decision = value.get("decision").unwrap();
    let target = decision.get("target").unwrap();

    assert_eq!(
        value.get("elapsed_ms").and_then(serde_json::Value::as_u64),
        Some(1234)
    );
    assert_eq!(
        value.get("source").and_then(serde_json::Value::as_str),
        Some("sway")
    );
    assert_eq!(
        value.get("status").and_then(serde_json::Value::as_str),
        Some("available")
    );
    assert_eq!(
        target.get("pid").and_then(serde_json::Value::as_u64),
        Some(4242)
    );
    assert_eq!(
        target.get("app_id").and_then(serde_json::Value::as_str),
        Some("steam")
    );
    assert_eq!(
        target.get("class").and_then(serde_json::Value::as_str),
        Some("Steam")
    );
    assert!(target.get("title").unwrap().is_null());
    assert_eq!(
        target.get("window_id").and_then(serde_json::Value::as_str),
        Some("123")
    );
    assert_eq!(
        target.get("workspace").and_then(serde_json::Value::as_str),
        Some("games")
    );
    let confidence = decision
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .unwrap();
    assert!((confidence - 0.95).abs() < 0.000_001);
}

#[test]
fn foreground_event_serializes_stale_ms() {
    let event = ForegroundEvent::new(ForegroundEventInput {
        elapsed_ms: 2_000,
        source: crate::foreground::ForegroundSource::Sway,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid: Some(4242),
        app_id: Some("steam".to_owned()),
        class: Some("Steam".to_owned()),
        title: None,
        include_title: false,
        window_id: Some("42".to_owned()),
        workspace: Some("games".to_owned()),
        confidence: 0.50,
        stale_ms: Some(750),
        reason: "using stale foreground snapshot from 750ms ago".to_owned(),
    });

    let value = serde_json::to_value(&event).unwrap();

    assert_eq!(
        value.get("stale_ms").and_then(serde_json::Value::as_u64),
        Some(750)
    );
}

#[test]
fn foreground_event_deserializes_old_flat_events_without_stale_ms() {
    let value = serde_json::json!({
        "elapsed_ms": 1234,
        "source": "sway",
        "status": "available",
        "pid": 4242,
        "app_id": "steam",
        "class": "Steam",
        "title": null,
        "window_id": "42",
        "workspace": "games",
        "confidence": 0.95,
        "reason": "old foreground event"
    });

    let event: ForegroundEvent = serde_json::from_value(value).unwrap();

    assert_eq!(event.stale_ms, None);
    assert_eq!(
        event.decision.target.as_ref().and_then(|target| target.pid),
        Some(4242)
    );
    assert_eq!(
        event.decision.primary_reason(),
        Some("old foreground event")
    );
}

#[test]
fn recorded_config_defaults_foreground_fields_for_old_sessions() {
    let config = RecordedConfig::default();

    assert!(!config.foreground_window);
    assert_eq!(config.foreground_source, "");
    assert_eq!(config.foreground_poll_ms, 0);
    assert_eq!(config.foreground_max_stale_ms, 0);
    assert!(!config.foreground_include_title);
}

#[test]
fn recorded_config_defaults_auto_focus_fields_for_old_sessions() {
    let config = RecordedConfig::default();

    assert!(!config.auto_focus);
    assert!(!config.foreground_window);
    assert_eq!(config.focus_source, "");
    assert_eq!(config.foreground_source, "");
    assert_eq!(config.foreground_poll_ms, 0);
    assert_eq!(config.foreground_max_stale_ms, 0);
    assert!(!config.foreground_include_title);
    assert_eq!(config.auto_focus_poll_ms, 0);
    assert_eq!(config.auto_focus_min_confidence, 0.0);
    assert_eq!(config.auto_focus_switch_cooldown_ms, 0);
    assert_eq!(config.auto_focus_switch_margin, 0.0);
    assert_eq!(config.auto_focus_required_polls, 0);
    assert_eq!(config.auto_focus_max_roots, 0);
}

#[test]
fn session_metadata_defaults_focus_fields_for_old_sessions() {
    let core = SessionMetadataCore::default();

    assert_eq!(core.focus_mode, None);
    assert_eq!(core.final_focus_kind, None);
    assert_eq!(core.focus_switch_count, 0);
    assert_eq!(core.focus_event_count, 0);
    assert_eq!(core.foreground_event_count, 0);
    assert_eq!(core.foreground_source, None);
    assert_eq!(core.final_foreground_pid, None);
    assert_eq!(core.final_foreground_app_id, None);
    assert_eq!(core.final_foreground_class, None);
    assert_eq!(core.final_foreground_status, None);
    assert_eq!(core.final_foreground_window_id, None);
    assert_eq!(core.final_foreground_workspace, None);
    assert_eq!(core.final_foreground_confidence, None);
    assert_eq!(core.final_foreground_stale_ms, None);
    assert_eq!(core.final_foreground_reason, None);
}

#[test]
fn recorded_probe_activation_warning_uses_stable_catalog_key() {
    let warning = crate::probe_activation::ProbeActivationWarning {
        key: Some(crate::probe_registry::ProbeKey::Faults),
        message: "minor_fault failed".to_owned(),
    };

    let recorded = RecordedProbeActivationWarning::from(&warning);

    assert_eq!(recorded.key.as_deref(), Some("faults"));
    assert_eq!(recorded.message, "minor_fault failed");
}

#[test]
fn session_artifact_serializes_block_io_correlation_basis() {
    let session = SessionFile {
        core: SessionMetadataCore {
            block_io_correlation_basis: crate::ebpf_loader::BlockIoCorrelationBasis::RequestPointer
                .as_str()
                .to_owned(),
            block_io_correlation_confidence:
                crate::ebpf_loader::BlockIoCorrelationBasis::RequestPointer
                    .confidence()
                    .to_owned(),
            ..SessionMetadataCore::default()
        },
        ..SessionFile::default()
    };

    let value = serde_json::to_value(&session.core).unwrap();

    assert_eq!(
        value
            .get("block_io_correlation_basis")
            .and_then(serde_json::Value::as_str),
        Some("request-pointer")
    );
    assert_eq!(
        value
            .get("block_io_correlation_confidence")
            .and_then(serde_json::Value::as_str),
        Some("high")
    );
}

#[test]
fn session_metadata_defaults_block_io_correlation_basis_for_old_sessions() {
    let json = serde_json::json!({
        "schema_version": 0,
        "run_name": null,
        "started_at": RecordedTime::default(),
        "ended_at": RecordedTime::default(),
        "monotonic_start_ns": null,
        "monotonic_end_ns": null,
        "duration_ms": 0,
        "metadata": SystemMetadata::default(),
        "target_pids_max": 0,
        "active_target_pids_count": 0,
        "active_expanded_tasks": [],
        "stop_reason": "",
        "config": RecordedConfig::default(),
        "tasks": [],
        "top_spikes": []
    });

    let session: SessionFile = serde_json::from_value(json).unwrap();

    assert_eq!(session.core.block_io_correlation_basis, "dev+sector");
}
