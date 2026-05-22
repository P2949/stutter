use std::path::PathBuf;

use crate::config_file::*;

#[test]
fn daemon_user_config_validation_rejects_invalid_confidence_and_family_names() {
    let invalid_confidence = UserConfigFile {
        daemon_min_confidence: Some(1.25),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&invalid_confidence)
            .unwrap_err()
            .to_string()
            .contains("daemon_min_confidence")
    );

    let invalid_provider_confidence = UserConfigFile {
        daemon_min_apply_medium_risk_confidence: Some(-0.01),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&invalid_provider_confidence)
            .unwrap_err()
            .to_string()
            .contains("daemon_min_apply_medium_risk_confidence")
    );

    let invalid_family = UserConfigFile {
        daemon_enabled_action_families: Some(vec![" gpu ".to_owned()]),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&invalid_family)
            .unwrap_err()
            .to_string()
            .contains("leading or trailing whitespace")
    );
}

#[test]
fn daemon_user_config_validation_rejects_invalid_health_guardrails() {
    let invalid_cpu_temp = UserConfigFile {
        daemon_max_cpu_temp_celsius: Some(20),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&invalid_cpu_temp)
            .unwrap_err()
            .to_string()
            .contains("daemon_max_cpu_temp_celsius")
    );

    let invalid_memory_pressure = UserConfigFile {
        daemon_max_memory_pressure_some_avg10_percent: Some(101.0),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&invalid_memory_pressure)
            .unwrap_err()
            .to_string()
            .contains("daemon_max_memory_pressure_some_avg10_percent")
    );

    let invalid_disk = UserConfigFile {
        daemon_min_disk_available_bytes: Some(0),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&invalid_disk)
            .unwrap_err()
            .to_string()
            .contains("daemon_min_disk_available_bytes")
    );
}

#[test]
fn user_config_diagnostics_report_invalid_workload_policy_rules() {
    let toml = r#"
            [autotune]

            [[autotune.workload_policy.rules]]
            situation = "browser_focused"
            allowed_families = ["nice"]
            allowed_objectives = ["not_real"]
            autonomous_families = []
        "#;
    let parsed = parse_user_config_toml_versioned(toml).unwrap();

    assert!(parsed.diagnostics.iter().any(|diagnostic| {
        diagnostic.level == crate::config::schema::ConfigDiagnosticLevel::Error
            && diagnostic.field.as_deref() == Some("autotune.workload_policy.rules")
            && diagnostic.message.contains("unknown objective")
    }));
    assert!(
        validate_daemon_user_config(&parsed.file)
            .unwrap_err()
            .to_string()
            .contains("invalid autotune.workload_policy.rules")
    );
}

#[test]
fn user_config_diagnostics_report_duplicate_workload_policy_rules() {
    let toml = r#"
            [autotune]

            [[autotune.workload_policy.rules]]
            situation = "browser_focused"
            allowed_families = ["nice"]
            allowed_objectives = ["browser_interactivity"]
            autonomous_families = []

            [[autotune.workload_policy.rules]]
            situation = "browser_focused"
            allowed_families = ["ionice"]
            allowed_objectives = ["io_latency"]
            autonomous_families = []
        "#;
    let parsed = parse_user_config_toml_versioned(toml).unwrap();

    assert!(parsed.diagnostics.iter().any(|diagnostic| {
        diagnostic.level == crate::config::schema::ConfigDiagnosticLevel::Error
            && diagnostic.field.as_deref() == Some("autotune.workload_policy.rules")
            && diagnostic
                .message
                .contains("duplicate workload policy rule")
    }));
}

#[test]
fn user_config_diagnostics_report_unknown_workload_policy_family() {
    let toml = r#"
            [autotune]

            [[autotune.workload_policy.rules]]
            situation = "browser_focused"
            allowed_families = ["mystery_knob"]
            allowed_objectives = ["browser_interactivity"]
            autonomous_families = []
        "#;
    let parsed = parse_user_config_toml_versioned(toml).unwrap();

    assert!(parsed.diagnostics.iter().any(|diagnostic| {
        diagnostic.level == crate::config::schema::ConfigDiagnosticLevel::Error
            && diagnostic.field.as_deref() == Some("autotune.workload_policy.rules")
            && diagnostic
                .message
                .contains("unknown workload policy action family")
    }));
    assert!(
        validate_daemon_user_config(&parsed.file)
            .unwrap_err()
            .to_string()
            .contains("invalid autotune.workload_policy.rules")
    );
}

#[test]
fn user_config_diagnostics_report_conflicting_workload_policy_locations() {
    let toml = r#"
            [autotune]

            [[autotune.workload_policy.rules]]
            situation = "browser_focused"
            allowed_families = ["nice"]
            allowed_objectives = ["browser_interactivity"]
            autonomous_families = []

            [[autotune.workload_policy_rules]]
            situation = "browser_focused"
            allowed_families = ["ionice"]
            allowed_objectives = ["io_latency"]
            autonomous_families = []
        "#;
    let parsed = parse_user_config_toml_versioned(toml).unwrap();

    assert!(parsed.diagnostics.iter().any(|diagnostic| {
            diagnostic.level == crate::config::schema::ConfigDiagnosticLevel::Error
                && diagnostic.field.as_deref() == Some("autotune.workload_policy")
                && diagnostic.message.contains(
                    "configure either autotune.workload_policy.rules or autotune.workload_policy_rules, not both",
                )
        }));
    assert!(
            validate_daemon_user_config(&parsed.file)
                .unwrap_err()
                .to_string()
                .contains("configure either autotune.workload_policy.rules or autotune.workload_policy_rules, not both")
        );
}

#[test]
fn daemon_user_config_validation_rejects_invalid_workload_policy_rules() {
    let toml = r#"
            [autotune]

            [[autotune.workload_policy_rules]]
            situation = "browser_focused"
            allowed_families = ["not_real"]
            allowed_objectives = ["browser_interactivity"]
            autonomous_families = []
        "#;
    let config = parse_user_config_toml(toml).unwrap();

    assert!(
        validate_daemon_user_config(&config)
            .unwrap_err()
            .to_string()
            .contains("invalid autotune.workload_policy_rules")
    );
}

#[test]
fn daemon_user_config_validation_guards_experimental_high_risk_fields() {
    let blocked = UserConfigFile {
        daemon_allow_high_risk: Some(true),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&blocked)
            .unwrap_err()
            .to_string()
            .contains("experimental = true")
    );

    let suggestions_allowed_without_experimental = UserConfigFile {
        daemon_allow_system_wide_suggestions: Some(true),
        ..Default::default()
    };
    validate_daemon_user_config(&suggestions_allowed_without_experimental).unwrap();

    let apply_blocked_without_experimental = UserConfigFile {
        daemon_allow_system_wide_apply: Some(true),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&apply_blocked_without_experimental)
            .unwrap_err()
            .to_string()
            .contains("experimental = true")
    );

    let allowed = UserConfigFile {
        experimental: Some(true),
        daemon_allow_high_risk: Some(true),
        daemon_allow_system_wide_suggestions: Some(true),
        daemon_allow_system_wide_apply: Some(true),
        ..Default::default()
    };
    validate_daemon_user_config(&allowed).unwrap();
}

#[test]
fn daemon_user_config_validation_rejects_invalid_cgroup_targets() {
    let relative = UserConfigFile {
        daemon_background_cgroup: Some(PathBuf::from("stutter-background.slice")),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&relative)
            .unwrap_err()
            .to_string()
            .contains("cgroup")
    );

    let traversal = UserConfigFile {
        daemon_compile_cgroup: Some(PathBuf::from("/user.slice/../bad.slice")),
        ..Default::default()
    };
    assert!(
        validate_daemon_user_config(&traversal)
            .unwrap_err()
            .to_string()
            .contains("parent traversal")
    );
}

#[test]
fn test_agent_autotune_limits_reject_system_wide_actions() {
    let toml = r#"
            [agent.autotune_limits]
            allow_system_wide_apply = true
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let err = agent_autotune_limits_from_user_config(Some(&config))
        .unwrap_err()
        .to_string();

    assert!(err.contains("allow_system_wide_apply must be false"));
}

#[test]
fn test_agent_autotune_limits_reject_too_many_targets() {
    let toml = r#"
            [agent.autotune_limits]
            max_targets = 2
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let err = agent_autotune_limits_from_user_config(Some(&config))
        .unwrap_err()
        .to_string();

    assert!(err.contains("max_targets = 1"));
}

#[test]
fn test_agent_autotune_limits_reject_too_long_candidate_window() {
    let toml = r#"
            [agent.autotune_limits]
            max_candidate_window_seconds = 121
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let err = agent_autotune_limits_from_user_config(Some(&config))
        .unwrap_err()
        .to_string();

    assert!(err.contains("max_candidate_window_seconds must be <= 120"));
}

#[test]
fn test_agent_autotune_limits_reject_high_risk_ceiling() {
    let toml = r#"
            [agent.autotune_limits]
            max_safety_class = "HighRisk"
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let err = agent_autotune_limits_from_user_config(Some(&config))
        .unwrap_err()
        .to_string();

    assert!(err.contains("apply-low-risk only") || err.contains("ReversibleLowRisk only"));
}

#[test]
fn test_agent_autotune_limits_reject_high_mode_ceiling() {
    let toml = r#"
            [agent.autotune_limits]
            max_mode = "apply-medium-risk"
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let err = agent_autotune_limits_from_user_config(Some(&config))
        .unwrap_err()
        .to_string();

    assert!(err.contains("apply-low-risk only"));
}

#[test]
fn test_agent_autotune_limits_reject_invalid_safety_class() {
    let toml = r#"
            [agent.autotune_limits]
            max_safety_class = "Invalid"
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let err = agent_autotune_limits_from_user_config(Some(&config)).unwrap_err();

    let err_str = err.to_string();
    assert!(err_str.contains("max_safety_class"));
}
