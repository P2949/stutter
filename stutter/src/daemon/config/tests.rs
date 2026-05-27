use super::*;
use crate::{
    actions::SafetyClass,
    daemon::policy::{ActionSource, DaemonMode},
};

#[test]
fn daemon_config_default_serializes() {
    let config = DaemonConfig::default();

    let json = serde_json::to_string(&config).unwrap();

    assert!(json.contains("\"mode\":\"observe\""));
    assert!(json.contains("\"preset\":\"observe-only\""));
    assert!(json.contains("\"source\":\"cli\""));
    assert!(json.contains("\"retain_crash_diagnostics\":true"));
}

#[test]
fn daemon_autotune_config_defaults_privileged_worker_timing() {
    let config = DaemonAutotuneConfig::default();

    assert_eq!(
        config.privileged_worker_socket_ready_timeout_ms,
        DEFAULT_PRIVILEGED_WORKER_SOCKET_READY_TIMEOUT_MS
    );
    assert_eq!(
        config.privileged_worker_socket_ready_retry_ms,
        DEFAULT_PRIVILEGED_WORKER_SOCKET_READY_RETRY_MS
    );
    assert_eq!(
        config.privileged_worker_shutdown_poll_ms,
        DEFAULT_PRIVILEGED_WORKER_SHUTDOWN_POLL_MS
    );
}

#[test]
fn daemon_config_owns_user_intent_fields() {
    let mut config = DaemonConfig {
        preset: DaemonPreset::GamingLowRisk,
        mode: DaemonMode::ApplyLowRisk,
        source: ActionSource::RemoteAgent,
        ..DaemonConfig::default()
    };
    config.target.tree_pids.push(1234);
    config.target.require_explicit_target = true;
    config.safety.max_safety_class = SafetyClass::ReversibleLowRisk;
    config.safety.allow_system_wide_suggestions = true;
    config.safety.allow_system_wide_apply = false;
    config.retention.max_state_snapshots = 4;
    config.remote.allow_remote_apply = true;
    config.autotune.candidate_window_seconds = 60;

    assert_eq!(config.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(config.preset, DaemonPreset::GamingLowRisk);
    assert_eq!(config.source, ActionSource::RemoteAgent);
    assert_eq!(config.target.tree_pids, vec![1234]);
    assert!(config.target.require_explicit_target);
    assert_eq!(
        config.safety.max_safety_class,
        SafetyClass::ReversibleLowRisk
    );
    assert!(config.safety.allow_system_wide_suggestions);
    assert!(!config.safety.allow_system_wide_apply);
    assert_eq!(config.retention.max_state_snapshots, 4);
    assert!(config.remote.allow_remote_apply);
    assert_eq!(config.autotune.candidate_window_seconds, 60);
    assert_eq!(config.autotune.confidence.min_suggest_confidence, 0.50);
    assert_eq!(
        config.autotune.confidence.min_apply_low_risk_confidence,
        crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
    );
    assert_eq!(
        config.autotune.confidence.min_apply_medium_risk_confidence,
        0.85
    );
    assert_eq!(
        config
            .autotune
            .confidence
            .min_high_risk_suggestion_confidence,
        0.90
    );
}

#[test]
fn daemon_presets_map_to_expected_safe_policy_defaults() {
    let observe = DaemonConfig::from_preset(DaemonPreset::ObserveOnly, ActionSource::Cli);
    assert_eq!(observe.mode, DaemonMode::Observe);
    assert_eq!(observe.safety.max_safety_class, SafetyClass::ObserveOnly);
    assert!(observe.safety.enabled_action_families.is_empty());

    let gaming = DaemonConfig::from_preset(DaemonPreset::GamingLowRisk, ActionSource::Cli);
    assert_eq!(gaming.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(
        gaming.safety.max_safety_class,
        SafetyClass::ReversibleLowRisk
    );
    assert!(
        gaming
            .safety
            .enabled_action_families
            .contains("cpu_affinity_profile")
    );
    assert!(!gaming.safety.allow_system_wide_suggestions);
    assert!(!gaming.safety.allow_system_wide_apply);
    assert!(gaming.safety.min_confidence >= 0.85);

    let laptop = DaemonConfig::from_preset(DaemonPreset::GamingLaptopSafe, ActionSource::Cli);
    assert_eq!(laptop.mode, DaemonMode::ApplyLowRisk);
    assert!(laptop.safety.min_confidence > gaming.safety.min_confidence);
    assert!(laptop.health.max_cpu_temp_celsius < gaming.health.max_cpu_temp_celsius);

    let debug = DaemonConfig::from_preset(DaemonPreset::DebugAggressive, ActionSource::Cli);
    assert_eq!(debug.mode, DaemonMode::ApplyMediumRisk);
    assert_eq!(
        debug.safety.max_safety_class,
        SafetyClass::ReversibleMediumRisk
    );
    assert!(!debug.safety.allow_high_risk);
    assert!(debug.safety.enabled_action_families.contains("uclamp"));
}

#[test]
fn daemon_health_config_maps_to_system_health_thresholds() {
    let config = DaemonHealthConfig {
        max_cpu_temp_celsius: 80,
        max_gpu_temp_celsius: 81,
        min_disk_available_bytes: 1_000_000_000,
        max_memory_pressure_some_avg10_percent: 12.5,
    };

    let thresholds = config.thresholds();

    assert_eq!(thresholds.max_cpu_temp_millidegrees, 80_000);
    assert_eq!(thresholds.max_gpu_temp_millidegrees, 81_000);
    assert_eq!(thresholds.min_disk_available_bytes, 1_000_000_000);
    assert_eq!(
        thresholds.max_memory_pressure_some_avg10_millipercent,
        12_500
    );
}

#[test]
fn daemon_preset_parser_accepts_documented_names() {
    assert_eq!(
        "observe-only".parse::<DaemonPreset>().unwrap(),
        DaemonPreset::ObserveOnly
    );
    assert_eq!(
        DaemonPreset::DebugAggressive.to_string(),
        "debug-aggressive"
    );
    assert!("risky".parse::<DaemonPreset>().is_err());
}
