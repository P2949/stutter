use std::path::PathBuf;

use crate::{
    config::{FocusSource, ForegroundSource, schema::CURRENT_CONFIG_VERSION},
    config_file::*,
    error::ConfigError,
};

#[test]
fn parse_user_config_toml_versioned_accepts_missing_version_as_v1() {
    let toml = r#"
            summary_period_ms = 500
            spike_threshold_ns = 1000000
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();

    assert_eq!(parsed.version, 1);
    assert_eq!(parsed.file.config_version, Some(1));
    assert_eq!(parsed.file.summary_period_ms, Some(500));
    assert_eq!(parsed.file.spike_threshold_ns, Some(1_000_000));
    assert!(parsed.diagnostics.is_empty());
}

#[test]
fn parse_user_config_toml_versioned_accepts_explicit_v1() {
    let toml = r#"
            config_version = 1
            summary_period_ms = 250
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();

    assert_eq!(parsed.version, 1);
    assert_eq!(parsed.file.config_version, Some(1));
    assert_eq!(parsed.file.summary_period_ms, Some(250));
}

#[test]
fn parse_user_config_toml_versioned_accepts_schema_version_alias() {
    let toml = r#"
            schema_version = 1
            summary_period_ms = 250
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();

    assert_eq!(parsed.version, 1);
    assert_eq!(parsed.file.config_version, Some(1));
    assert_eq!(parsed.file.summary_period_ms, Some(250));
    assert!(parsed.diagnostics.is_empty());
}

#[test]
fn parse_user_config_toml_versioned_rejects_conflicting_version_keys() {
    let toml = r#"
            config_version = 1
            schema_version = 2
        "#;

    let err = parse_user_config_toml_versioned(toml).unwrap_err();

    match err {
        ConfigError::InvalidConfigVersion { message } => {
            assert!(message.contains("must match"));
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn parse_user_config_toml_versioned_rejects_future_version() {
    let toml = r#"
            config_version = 2
        "#;

    let err = parse_user_config_toml_versioned(toml).unwrap_err();

    match err {
        ConfigError::UnsupportedConfigVersion { version, current } => {
            assert_eq!(version, 2);
            assert_eq!(current, CURRENT_CONFIG_VERSION);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn parse_user_config_toml_versioned_warns_for_deprecated_aliases() {
    let toml = r#"
            summary_ms = 500
            spike_us = 1000
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();
    let fields: Vec<_> = parsed
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.field.as_deref())
        .collect();

    assert!(fields.contains(&Some("summary_ms")));
    assert!(fields.contains(&Some("spike_us")));
    assert!(parsed.diagnostics.iter().all(|diagnostic| {
        diagnostic.level == crate::config::schema::ConfigDiagnosticLevel::Warning
    }));

    let layer = crate::config::layer::layer_from_user_file(&parsed.file).unwrap();
    assert_eq!(layer.summary_period_ms, Some(500));
    assert_eq!(layer.spike_threshold_ns, Some(1_000_000));
}

#[test]
fn parse_user_config_toml_versioned_warns_for_unknown_top_level_fields() {
    let toml = r#"
            mystery_toggle = true
            summary_period_ms = 500
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();

    assert!(parsed.diagnostics.iter().any(|diagnostic| {
        diagnostic.field.as_deref() == Some("mystery_toggle")
            && diagnostic
                .message
                .contains("unknown top-level config field")
    }));
}

#[test]
fn test_parse_valid_toml() {
    let toml = r#"
            summary_ms = 500
            spike_us = 1000
            hwmon = true
            cpu_freq = true
            include_comm = ["Game", "Render"]
            retention_max_run_count = 10
            retention_max_total_bytes = 2000000
            retention_max_age_seconds = 86400
            retention_min_free_bytes = 1000000
            daemon_preset = "gaming-low-risk"
            daemon_enabled_action_families = ["cpu_affinity_profile"]
            daemon_denied_action_families = ["gpu-power"]
            daemon_background_cgroup = "/user.slice/stutter-background.slice"
            daemon_compile_cgroup = "/user.slice/stutter-compile.slice"
            daemon_min_confidence = 0.91
            daemon_min_suggest_confidence = 0.55
            daemon_min_apply_low_risk_confidence = 0.82
            daemon_min_apply_medium_risk_confidence = 0.87
            daemon_min_high_risk_suggestion_confidence = 0.93
            daemon_max_cpu_temp_celsius = 83
            daemon_max_gpu_temp_celsius = 84
            daemon_min_disk_available_bytes = 1073741824
            daemon_max_memory_pressure_some_avg10_percent = 25.5

            [autotune]
            allow_cpu_power_on_battery = true
        "#;
    let config = parse_user_config_toml(toml).unwrap();
    assert_eq!(config.summary_ms, Some(500));
    assert_eq!(config.spike_us, Some(1000));
    assert_eq!(config.hwmon, Some(true));
    assert_eq!(config.cpu_freq, Some(true));
    assert_eq!(
        config.include_comm.as_deref(),
        Some(&["Game".to_owned(), "Render".to_owned()][..])
    );
    assert_eq!(config.retention_max_run_count, Some(10));
    assert_eq!(config.retention_max_total_bytes, Some(2_000_000));
    assert_eq!(config.retention_max_age_seconds, Some(86_400));
    assert_eq!(config.retention_min_free_bytes, Some(1_000_000));
    assert_eq!(config.daemon_preset.as_deref(), Some("gaming-low-risk"));
    assert_eq!(
        config.daemon_enabled_action_families.as_deref(),
        Some(&["cpu_affinity_profile".to_owned()][..])
    );
    assert_eq!(
        config.daemon_denied_action_families.as_deref(),
        Some(&["gpu-power".to_owned()][..])
    );
    assert_eq!(
        config.daemon_background_cgroup.as_deref(),
        Some(PathBuf::from("/user.slice/stutter-background.slice").as_path())
    );
    assert_eq!(
        config.daemon_compile_cgroup.as_deref(),
        Some(PathBuf::from("/user.slice/stutter-compile.slice").as_path())
    );
    assert_eq!(config.daemon_min_confidence, Some(0.91));
    assert_eq!(config.daemon_min_suggest_confidence, Some(0.55));
    assert_eq!(config.daemon_min_apply_low_risk_confidence, Some(0.82));
    assert_eq!(config.daemon_min_apply_medium_risk_confidence, Some(0.87));
    assert_eq!(
        config.daemon_min_high_risk_suggestion_confidence,
        Some(0.93)
    );
    assert_eq!(config.daemon_max_cpu_temp_celsius, Some(83));
    assert_eq!(config.daemon_max_gpu_temp_celsius, Some(84));
    assert_eq!(config.daemon_min_disk_available_bytes, Some(1_073_741_824));
    assert_eq!(
        config.daemon_max_memory_pressure_some_avg10_percent,
        Some(25.5)
    );
    assert_eq!(
        config
            .autotune
            .as_ref()
            .and_then(|autotune| autotune.allow_cpu_power_on_battery),
        Some(true)
    );
    validate_daemon_user_config(&config).unwrap();
}

#[test]
fn test_parse_focus_source_value() {
    assert_eq!(
        parse_focus_source_value("heuristic").unwrap(),
        FocusSource::Heuristic
    );
    assert_eq!(
        parse_focus_source_value("foreground").unwrap(),
        FocusSource::Foreground
    );
    assert_eq!(
        parse_focus_source_value("hybrid").unwrap(),
        FocusSource::Hybrid
    );
    assert!(parse_focus_source_value("invalid").is_err());
}

#[test]
fn test_parse_foreground_source_value() {
    assert_eq!(
        parse_foreground_source_value("auto").unwrap(),
        ForegroundSource::Auto
    );
    assert_eq!(
        parse_foreground_source_value("sway").unwrap(),
        ForegroundSource::Sway
    );
    assert_eq!(
        parse_foreground_source_value("hyprland").unwrap(),
        ForegroundSource::Hyprland
    );
    assert_eq!(
        parse_foreground_source_value("gnome").unwrap(),
        ForegroundSource::Gnome
    );
    assert_eq!(
        parse_foreground_source_value("kde").unwrap(),
        ForegroundSource::Kde
    );
    assert_eq!(
        parse_foreground_source_value("plasma").unwrap(),
        ForegroundSource::Kde
    );
    assert_eq!(
        parse_foreground_source_value("x11").unwrap(),
        ForegroundSource::X11
    );
    assert!(parse_foreground_source_value("invalid").is_err());
}

#[test]
fn test_parse_foreground_config_fields() {
    let toml = r#"
            foreground_window = true
            focus_source = "hybrid"
            foreground_source = "sway"
            foreground_poll_ms = 750
            foreground_max_stale_ms = 3000
            foreground_include_title = true
        "#;

    let config = parse_user_config_toml(toml).unwrap();

    assert_eq!(config.foreground_window, Some(true));
    assert_eq!(config.focus_source.as_deref(), Some("hybrid"));
    assert_eq!(config.foreground_source.as_deref(), Some("sway"));
    assert_eq!(config.foreground_poll_ms, Some(750));
    assert_eq!(config.foreground_max_stale_ms, Some(3000));
    assert_eq!(config.foreground_include_title, Some(true));
}

#[test]
fn test_parse_community_rules_config_fields() {
    let toml = r#"
            [community_rules]
            enabled = true
            sources = ["user"]
            paths = ["/tmp/stutter/rules/custom.generated.json"]
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let community_rules = config.community_rules.unwrap();

    assert_eq!(community_rules.enabled, Some(true));
    assert_eq!(community_rules.sources.unwrap(), vec!["user"]);
    assert_eq!(
        community_rules.paths.unwrap(),
        vec![PathBuf::from("/tmp/stutter/rules/custom.generated.json")]
    );
}

#[test]
fn config_file_parses_extended_ebpf_sizing_fields() {
    let toml = r#"
            [ebpf_sizing]
            ringbuf_size_kb = 1024
            wakeup_map_factor = 32
            target_pids_entries = 2048
            target_cgroup_ids_entries = 128
            target_irqs_entries = 256
            runnable_task_cpu_factor = 96
            prev_faults_factor = 160
            irq_start_entries = 4096
            block_start_entries = 32768
            kms_flip_start_entries = 8192
            drm_fence_wait_start_entries = 8192
            drm_fence_signal_entries = 8192
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();
    let ebpf_sizing = parsed.file.ebpf_sizing.as_ref().unwrap();

    assert_eq!(ebpf_sizing.ringbuf_size_kb, Some(1024));
    assert_eq!(ebpf_sizing.wakeup_map_factor, Some(32));
    assert_eq!(ebpf_sizing.target_pids_entries, Some(2048));
    assert_eq!(ebpf_sizing.target_cgroup_ids_entries, Some(128));
    assert_eq!(ebpf_sizing.target_irqs_entries, Some(256));
    assert_eq!(ebpf_sizing.runnable_task_cpu_factor, Some(96));
    assert_eq!(ebpf_sizing.prev_faults_factor, Some(160));
    assert_eq!(ebpf_sizing.irq_start_entries, Some(4096));
    assert_eq!(ebpf_sizing.block_start_entries, Some(32768));
    assert_eq!(ebpf_sizing.kms_flip_start_entries, Some(8192));
    assert_eq!(ebpf_sizing.drm_fence_wait_start_entries, Some(8192));
    assert_eq!(ebpf_sizing.drm_fence_signal_entries, Some(8192));
    assert!(
        parsed
            .diagnostics
            .iter()
            .all(|diagnostic| { diagnostic.field.as_deref() != Some("ebpf_sizing") })
    );

    let layer = crate::config::layer::layer_from_user_file(&parsed.file).unwrap();
    assert_eq!(layer.block_start_entries, Some(Some(32768)));
    assert_eq!(layer.drm_fence_signal_entries, Some(Some(8192)));
}

#[test]
fn config_file_parses_mangohud_timing_overrides() {
    let toml = r#"
            [mangohud]
            tail_idle_sleep_ms = 25
            alignment_poll_ms = 125
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();
    let mangohud = parsed.file.mangohud.as_ref().unwrap();

    assert_eq!(mangohud.tail_idle_sleep_ms, Some(25));
    assert_eq!(mangohud.alignment_poll_ms, Some(125));

    let layer = crate::config::layer::layer_from_user_file(&parsed.file).unwrap();
    assert_eq!(layer.mangohud_tail_idle_sleep_ms, Some(25));
    assert_eq!(layer.mangohud_alignment_poll_ms, Some(125));
}

#[test]
fn config_file_parses_alert_desktop_timeout_override() {
    let toml = r#"
            [alerts]
            desktop_timeout_ms = 2500
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();
    let alerts = parsed.file.alerts.as_ref().unwrap();

    assert_eq!(alerts.desktop_timeout_ms, Some(2500));

    let layer = crate::config::layer::layer_from_user_file(&parsed.file).unwrap();
    assert_eq!(layer.alert_desktop_timeout_ms, Some(2500));
}

#[test]
fn config_file_parses_autotune_privileged_worker_timing_overrides() {
    let toml = r#"
            [autotune]
            privileged_worker_socket_ready_timeout_ms = 1500
            privileged_worker_socket_ready_retry_ms = 40
            privileged_worker_shutdown_poll_ms = 15
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let autotune = config.autotune.as_ref().unwrap();

    assert_eq!(
        autotune.privileged_worker_socket_ready_timeout_ms,
        Some(1500)
    );
    assert_eq!(autotune.privileged_worker_socket_ready_retry_ms, Some(40));
    assert_eq!(autotune.privileged_worker_shutdown_poll_ms, Some(15));
}

#[test]
fn test_parse_agent_autotune_limits() {
    let toml = r#"
            [agent.autotune_limits]
            max_active_controllers = 1
            max_safety_class = "ReversibleLowRisk"
            max_candidate_window_seconds = 120
            max_targets = 1
            allow_system_wide_suggestions = false
            allow_system_wide_apply = false
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let limits = agent_autotune_limits_from_user_config(Some(&config)).unwrap();

    assert_eq!(limits.max_active_controllers, 1);
    assert_eq!(
        limits.max_mode,
        crate::daemon_policy::DaemonMode::ApplyLowRisk
    );
    assert_eq!(
        limits.max_safety_class,
        crate::actions::SafetyClass::ReversibleLowRisk
    );
    assert_eq!(limits.max_candidate_window_seconds, 120);
    assert_eq!(limits.max_targets, 1);
    assert!(!limits.allow_system_wide_suggestions);
    assert!(!limits.allow_system_wide_apply);
}

#[test]
fn test_parse_invalid_toml() {
    let toml = r#"
            summary_ms = "not a number"
        "#;
    let err = parse_user_config_toml(toml).unwrap_err();
    println!("Actual error: {}", err);
    assert!(
        err.to_string().to_lowercase().contains("integer")
            || err.to_string().to_lowercase().contains("invalid type")
    );
}

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        // SAFETY: this guard is used by tests that serialize environment
        // mutation through the crate test mutex.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(old) = &self.old {
            // SAFETY: this restores process environment while the owning
            // test still holds the serialization lock.
            unsafe {
                std::env::set_var(self.key, old);
            }
        } else {
            // SAFETY: this restores process environment while the owning
            // test still holds the serialization lock.
            unsafe {
                std::env::remove_var(self.key);
            }
        }
    }
}

#[test]
fn test_stutter_config_env_honored() {
    let _guard = EnvGuard::set("STUTTER_CONFIG", "/tmp/stutter.toml");
    let path = resolve_user_config_path().unwrap();
    assert_eq!(path, PathBuf::from("/tmp/stutter.toml"));
}
