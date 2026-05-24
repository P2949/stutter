use std::path::PathBuf;

use crate::{
    config_file::*,
    daemon::{DaemonPreset, policy::ActionSource},
    remote::AgentAutotuneLimits,
};

#[test]
fn daemon_config_from_user_config_applies_policy_and_health_overrides() {
    let config = UserConfigFile {
        daemon_preset: Some("gaming-low-risk".to_owned()),
        daemon_enabled_action_families: Some(vec!["cpu_affinity_profile".to_owned()]),
        daemon_denied_action_families: Some(vec!["ionice".to_owned()]),
        daemon_background_cgroup: Some(PathBuf::from("/user.slice/stutter-background.slice")),
        daemon_min_confidence: Some(0.92),
        daemon_max_cpu_temp_celsius: Some(76),
        daemon_max_gpu_temp_celsius: Some(77),
        daemon_min_disk_available_bytes: Some(2_500_000_000),
        daemon_max_memory_pressure_some_avg10_percent: Some(18.5),
        ..Default::default()
    };

    let daemon_config =
        daemon_config_from_user_config(Some(&config), None, ActionSource::Cli).unwrap();

    assert_eq!(daemon_config.preset, DaemonPreset::GamingLowRisk);
    assert!(
        daemon_config
            .safety
            .enabled_action_families
            .contains("cpu_affinity_profile")
    );
    assert!(
        daemon_config
            .safety
            .denied_action_families
            .contains("ionice")
    );
    assert_eq!(
        daemon_config
            .safety
            .cgroup_targets
            .background_cgroup
            .as_deref(),
        Some(PathBuf::from("/user.slice/stutter-background.slice").as_path())
    );
    assert_eq!(daemon_config.safety.min_confidence, 0.92);
    assert_eq!(daemon_config.health.max_cpu_temp_celsius, 76);
    assert_eq!(
        daemon_config
            .health
            .thresholds()
            .max_memory_pressure_some_avg10_millipercent,
        18_500
    );
}

#[test]
fn daemon_config_from_user_config_applies_system_wide_allowlist() {
    let toml = r#"
            daemon_allow_system_wide_suggestions = true

            [system_wide_allowlist]
            cpu_policies = ["policy0", "policy1"]
            gpu_cards = ["card0"]
            gpu_pci_ids = ["1002:*"]
            irq_devices = ["amdgpu", "xhci_hcd"]
            vm_knobs = ["proc/sys/vm/swappiness"]
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();
    assert!(
        parsed
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.field.as_deref() != Some("system_wide_allowlist"))
    );

    let daemon_config =
        daemon_config_from_user_config(Some(&parsed.file), None, ActionSource::Cli).unwrap();
    let allowlist = &daemon_config.safety.system_wide_allowlist;

    assert!(allowlist.cpu_policies.contains("policy0"));
    assert!(allowlist.allows_gpu("card0", None));
    assert!(allowlist.allows_gpu("card99", Some("1002:abcd")));
    assert!(allowlist.allows_irq_device("pci:amdgpu"));
    assert!(allowlist.allows_vm_knob(PathBuf::from("/proc/sys/vm/swappiness").as_path()));
}

#[test]
fn daemon_config_from_user_config_applies_privileged_worker_timing() {
    let toml = r#"
            [autotune]
            privileged_worker_socket_ready_timeout_ms = 1500
            privileged_worker_socket_ready_retry_ms = 40
            privileged_worker_shutdown_poll_ms = 15
        "#;

    let parsed = parse_user_config_toml_versioned(toml).unwrap();
    let daemon_config =
        daemon_config_from_user_config(Some(&parsed.file), None, ActionSource::Cli).unwrap();

    assert_eq!(
        daemon_config
            .autotune
            .privileged_worker_socket_ready_timeout_ms,
        1500
    );
    assert_eq!(
        daemon_config
            .autotune
            .privileged_worker_socket_ready_retry_ms,
        40
    );
    assert_eq!(
        daemon_config.autotune.privileged_worker_shutdown_poll_ms,
        15
    );
}

#[test]
fn daemon_config_from_user_config_applies_nested_workload_policy_overrides() {
    let toml = r#"
            [autotune]
            allow_medium_risk_apply = true

            [[autotune.workload_policy.rules]]
            situation = "browser_focused"
            allowed_families = ["nice"]
            allowed_objectives = ["browser_interactivity"]
            autonomous_families = []
        "#;
    let parsed = parse_user_config_toml_versioned(toml).unwrap();
    validate_daemon_user_config(&parsed.file).unwrap();

    assert!(parsed.diagnostics.iter().any(|diagnostic| {
        diagnostic.field.as_deref() == Some("autotune.workload_policy.rules[0].autonomous_families")
            && diagnostic.message.contains("disables autonomous apply")
    }));

    let daemon_config =
        daemon_config_from_user_config(Some(&parsed.file), None, ActionSource::Cli).unwrap();
    let matrix = daemon_config
        .autotune
        .workload_policy
        .resolved_matrix()
        .unwrap();
    let rule = matrix.rule_for(crate::autotune::situation::SituationKind::BrowserFocused);

    assert!(daemon_config.autotune.allow_medium_risk_apply);
    assert_eq!(
        rule.allowed_families,
        ["nice"].into_iter().map(str::to_owned).collect()
    );
}

#[test]
fn daemon_config_from_user_config_applies_workload_policy_overrides() {
    let toml = r#"
            [autotune]
            allow_medium_risk_apply = true

            [[autotune.workload_policy_rules]]
            situation = "browser_focused"
            allowed_families = ["nice"]
            allowed_objectives = ["browser_interactivity"]
            autonomous_families = []
        "#;
    let config = parse_user_config_toml(toml).unwrap();
    validate_daemon_user_config(&config).unwrap();

    let daemon_config =
        daemon_config_from_user_config(Some(&config), None, ActionSource::Cli).unwrap();
    let matrix = daemon_config
        .autotune
        .workload_policy
        .resolved_matrix()
        .unwrap();
    let rule = matrix.rule_for(crate::autotune::situation::SituationKind::BrowserFocused);

    assert!(daemon_config.autotune.allow_medium_risk_apply);
    assert_eq!(
        rule.allowed_families,
        ["nice"].into_iter().map(str::to_owned).collect()
    );
}

#[test]
fn daemon_config_from_user_config_rejects_critical_workload_policy_lints() {
    let toml = r#"
            daemon_preset = "gaming-low-risk"

            [autotune]

            [[autotune.workload_policy.rules]]
            situation = "game_focused"
            allowed_families = ["gpu_power"]
            allowed_objectives = ["thermal_recovery"]
            autonomous_families = ["gpu_power"]
        "#;
    let config = parse_user_config_toml(toml).unwrap();

    let err = daemon_config_from_user_config(Some(&config), None, ActionSource::Cli)
        .unwrap_err()
        .to_string();

    assert!(err.contains("critical workload policy lint"));
    assert!(err.contains("system-wide action family"));
}

#[test]
fn test_community_rules_config_from_user_config_uses_parsed_section() {
    let toml = r#"
            [community_rules]
            enabled = false
            sources = []
            paths = ["/tmp/stutter/rules/custom.generated.json"]
        "#;

    let user_config = parse_user_config_toml(toml).unwrap();
    let community_rules = community_rules_config_from_user_config(Some(&user_config));

    assert!(!community_rules.enabled);
    assert_eq!(
        community_rules.explicit_rules_files,
        vec![PathBuf::from("/tmp/stutter/rules/custom.generated.json")]
    );
    assert!(community_rules.user_rules_dir.is_none());
    assert!(!community_rules.load_builtin_fixture);
}

#[test]
fn test_missing_agent_autotune_limits_uses_defaults() {
    let toml = r#"
            summary_ms = 500
        "#;

    let config = parse_user_config_toml(toml).unwrap();
    let limits = agent_autotune_limits_from_user_config(Some(&config)).unwrap();

    assert_eq!(limits, AgentAutotuneLimits::default());
}
