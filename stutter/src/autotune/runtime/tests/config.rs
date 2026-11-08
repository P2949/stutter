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

#[test]
fn worker_only_service_refuses_in_process_managed_missing_socket_and_wrong_mode() {
    let mut config = AutotuneRuntimeConfig::apply_low_risk(None, None, None);
    config.daemon_config.autotune.worker_only_mutation = true;
    assert!(validate_runtime_config(&config).is_err());
    config.daemon_config.mode = DaemonMode::ApplyMediumRisk;
    config.daemon_config.autotune.manage_privileged_worker = false;
    config.daemon_config.autotune.privileged_worker_socket =
        Some("/run/private-worker.sock".into());
    assert!(validate_runtime_config(&config).is_ok());
    config
        .daemon_config
        .autotune
        .unsafe_in_process_privileged_worker = true;
    assert!(validate_runtime_config(&config).is_err());
    config
        .daemon_config
        .autotune
        .unsafe_in_process_privileged_worker = false;
    config.daemon_config.autotune.manage_privileged_worker = true;
    assert!(validate_runtime_config(&config).is_err());
    config.daemon_config.autotune.manage_privileged_worker = false;
    config.daemon_config.autotune.privileged_worker_socket = None;
    assert!(validate_runtime_config(&config).is_err());
}

#[test]
fn the_runtime_samples_system_psi_into_every_observation_without_acting_on_it() {
    let mut config = AutotuneRuntimeConfig::suggest(None, Some(std::process::id()), None);
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let first = runtime.sensor_hub.sample();
    let second = runtime.sensor_hub.sample();
    // The real /proc/pressure exists on this kernel; a host without PSI reports why instead.
    let unavailable: Vec<_> = runtime
        .sensor_hub
        .unavailable()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let find = |readings: &[crate::autotune::sensors::SensorReading], id: &str| {
        readings.iter().position(|reading| reading.sensor == id)
    };
    // Each sensor is sampled when it could be admitted; one that could not says why instead.
    if !unavailable.contains(&"psi_system") {
        let (a, b) = (
            find(&first, "psi_system").unwrap(),
            find(&second, "psi_system").unwrap(),
        );
        assert!(second[b].values.contains_key("cpu_some_avg10"));
        assert!(first[a].monotonic_ns <= second[b].monotonic_ns);
    } else {
        assert!(find(&first, "psi_system").is_none());
    }
    assert_eq!(
        find(&first, "cpu_power").is_some(),
        !unavailable.contains(&"cpu_power")
    );
}

#[test]
fn the_context_signature_joins_inventory_topology_and_boot_context_and_is_stable_between_ticks() {
    use super::super::context_signature::compose;
    assert_eq!(compose("inv", "topo", "ctx"), "inv+topo+ctx");
    let mut config = AutotuneRuntimeConfig::suggest(None, Some(std::process::id()), None);
    config.history_log = None;
    let runtime = AutotuneRuntime::new(config);
    let first = runtime.context_signature("inventory-hash");
    let second = runtime.context_signature("inventory-hash");
    assert_eq!(first, second, "cached, so stable between ticks");
    assert!(first.starts_with("inventory-hash+"), "{first}");
    assert_eq!(first.matches('+').count(), 2, "{first}");
    // A different inventory is a different key and is not served from the cache.
    assert!(runtime.context_signature("other").starts_with("other+"));
}

#[test]
fn sampling_every_always_on_sensor_stays_cheap_enough_for_every_tick() {
    let mut config = AutotuneRuntimeConfig::suggest(None, Some(std::process::id()), None);
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let rounds = 200;
    let started = std::time::Instant::now();
    for _ in 0..rounds {
        let _ = runtime.sensor_hub.sample();
    }
    let per_sample = started.elapsed() / rounds;
    log::info!("always-on sensor sample: {per_sample:?} per evaluation");
    // About one evaluation a second runs this; at 5 ms it would still be 0.5 percent of a core.
    assert!(
        per_sample < std::time::Duration::from_millis(5),
        "{per_sample:?}"
    );
}
