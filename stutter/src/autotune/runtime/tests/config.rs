use super::*;

#[test]
fn runtime_config_stores_intent_and_permissions_in_daemon_fields() {
    let config =
        AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), Some("Game.exe".to_owned()))
            .with_min_focus_confidence(0.81)
            .with_candidate_window_seconds(45);

    assert_eq!(config.daemon_config.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(config.daemon_config.source, ActionSource::AutotuneRuntime);
    assert_eq!(config.daemon_config.target.tree_pids, vec![1234]);
    assert_eq!(
        config.daemon_config.target.watch_process.as_deref(),
        Some("Game.exe")
    );
    assert!(config.daemon_config.target.require_explicit_target);
    assert_eq!(config.daemon_config.safety.min_confidence, 0.81);
    assert_eq!(config.daemon_config.autotune.candidate_window_seconds, 45);
    assert_eq!(config.daemon_policy.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(
        config.daemon_policy.max_safety_class,
        SafetyClass::ReversibleLowRisk
    );
    assert_eq!(config.daemon_policy.min_confidence, 0.81);
}

#[test]
fn runtime_config_resolves_workload_policy_once_from_daemon_config() {
    let mut daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::Suggest,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    daemon_config.autotune.workload_policy = DaemonWorkloadPolicyConfig {
        rules: vec![crate::autotune::workload_policy::WorkloadPolicyRule {
            situation: SituationKind::BrowserFocused,
            allowed_families: std::collections::BTreeSet::from(["nice".to_owned()]),
            allowed_objectives: std::collections::BTreeSet::from([
                crate::autotune::objective::ObjectiveKind::BrowserInteractivity,
            ]),
            autonomous_families: std::collections::BTreeSet::new(),
        }],
    };

    let runtime_config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None);
    let rule = runtime_config
        .workload_policy
        .rule_for(SituationKind::BrowserFocused);

    assert_eq!(
        rule.allowed_families,
        std::collections::BTreeSet::from(["nice".to_owned()])
    );
    assert!(runtime_config.workload_policy_error.is_none());
}

#[test]
fn runtime_config_records_invalid_workload_policy_error_once() {
    let mut daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::Suggest,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    daemon_config.autotune.workload_policy = DaemonWorkloadPolicyConfig {
        rules: vec![crate::autotune::workload_policy::WorkloadPolicyRule {
            situation: SituationKind::BrowserFocused,
            allowed_families: std::collections::BTreeSet::from(["not_real".to_owned()]),
            allowed_objectives: std::collections::BTreeSet::new(),
            autonomous_families: std::collections::BTreeSet::new(),
        }],
    };

    let runtime_config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None);

    assert!(
        runtime_config
            .workload_policy_error
            .as_deref()
            .unwrap_or_default()
            .contains("unknown workload policy action family")
    );
}

#[test]
fn dry_run_all_safe_runtime_config_requires_suggest_mode() {
    let config =
        AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None).with_dry_run_all_safe(true);

    let err = validate_runtime_config(&config).unwrap_err().to_string();

    assert!(err.contains("--dry-run-all-safe requires suggest mode"));
}
