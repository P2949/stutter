//! Runtime configuration and observation event tests split from the parent runtime test module.

use super::*;
use crate::autotune::quality::OnlineDataQualityPolicy;

#[test]
fn candidate_selection_blocks_focus_confidence_below_policy_threshold() {
    let mut runtime = AutotuneRuntime::new(
        AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
            .with_profiles(vec![low_risk_profile()]),
    );
    let observation = high_quality_game_observation_with_focus_confidence(
        runtime.controller.policy.min_focus_confidence - 0.01,
    );

    let candidate = runtime
        .select_candidate_for_observation(&observation)
        .unwrap();

    assert!(candidate.is_none());
}

#[test]
fn runtime_observation_uses_configured_online_data_quality_policy() {
    let mut runtime = AutotuneRuntime::new(
        AutotuneRuntimeConfig::observe(None, Some(1234), None).with_online_data_quality_policy(
            OnlineDataQualityPolicy {
                min_scored_samples: 200,
                ..OnlineDataQualityPolicy::default()
            },
        ),
    );

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        runtime
            .controller
            .window
            .push_interval(crate::recorder::IntervalRecord {
                elapsed_ms,
                task: 42,
                samples: 20,
                over_1ms: 1,
                max_ns: 2_000_000,
                ..Default::default()
            });
    }

    let observation = runtime.build_observation();

    assert_eq!(observation.scored_samples, 100);
    assert!(observation.data_quality.is_low());
    assert!(
        observation
            .data_quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("fewer than min_scored_samples"))
    );
}

#[test]
fn runtime_config_defaults_to_default_online_data_quality_policy() {
    let config = AutotuneRuntimeConfig::observe(None, Some(1234), None);
    let default_policy = OnlineDataQualityPolicy::default();

    assert_eq!(
        config.online_data_quality_policy.min_scored_intervals,
        default_policy.min_scored_intervals
    );
    assert_eq!(
        config.online_data_quality_policy.min_scored_samples,
        default_policy.min_scored_samples
    );
    assert_eq!(
        config.online_data_quality_policy.max_drop_counter_total,
        default_policy.max_drop_counter_total
    );
    assert_eq!(
        config.online_data_quality_policy.frame_data_policy,
        default_policy.frame_data_policy
    );
}

#[test]
fn runtime_config_defaults_and_overrides_washout_policy() {
    let default_config = AutotuneRuntimeConfig::observe(None, Some(1234), None);

    assert_eq!(
        default_config.washout.washout_seconds,
        crate::autotune::washout::DEFAULT_WASHOUT_SECONDS
    );
    assert_eq!(
        default_config.washout.verify_interval_ms,
        crate::autotune::washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS
    );

    let custom_config =
        AutotuneRuntimeConfig::observe(None, Some(1234), None).with_washout(30, 2_000);

    assert_eq!(custom_config.washout.washout_seconds, 30);
    assert_eq!(custom_config.washout.verify_interval_ms, 2_000);

    let clamped_config = AutotuneRuntimeConfig::observe(None, Some(1234), None).with_washout(0, 50);

    assert_eq!(
        clamped_config.washout.washout_seconds,
        crate::autotune::washout::MIN_WASHOUT_SECONDS
    );
    assert_eq!(
        clamped_config.washout.verify_interval_ms,
        crate::autotune::washout::MIN_WASHOUT_VERIFY_INTERVAL_MS
    );
}

#[test]
fn runtime_config_default_min_focus_confidence_matches_controller_default() {
    let config = AutotuneRuntimeConfig::suggest(None, None, None);
    let runtime = AutotuneRuntime::new(config.clone());

    assert_eq!(
        config.daemon_config.safety.min_confidence,
        crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
    );
    assert_eq!(
        runtime.controller.policy.min_focus_confidence,
        crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
    );
}

#[test]
fn runtime_config_custom_min_focus_confidence_updates_controller_policy() {
    let config = AutotuneRuntimeConfig::suggest(None, None, None).with_min_focus_confidence(0.42);
    let runtime = AutotuneRuntime::new(config.clone());

    assert_eq!(config.daemon_config.safety.min_confidence, 0.42);
    assert_eq!(runtime.controller.policy.min_focus_confidence, 0.42);
}

#[test]
fn runtime_config_min_focus_confidence_is_clamped() {
    let low = AutotuneRuntimeConfig::suggest(None, None, None).with_min_focus_confidence(-1.0);
    let high = AutotuneRuntimeConfig::suggest(None, None, None).with_min_focus_confidence(2.0);

    assert_eq!(low.daemon_config.safety.min_confidence, 0.0);
    assert_eq!(high.daemon_config.safety.min_confidence, 1.0);
}

#[test]
fn runtime_washout_policy_delays_live_measurement_deadline() {
    let config = AutotuneRuntimeConfig::observe(None, Some(1234), None)
        .with_candidate_window_seconds(30)
        .with_washout(20, 2_000);
    let applied_unix_nanos = 1_000_000_000_u128;

    let (washout_until_unix_nanos, measure_until_unix_nanos) =
        LiveExperimentManager::deadlines_from_now(
            config.simulate_action_effects,
            &config.washout,
            config.candidate_window_seconds,
            applied_unix_nanos,
        );

    let expected_washout_until =
        applied_unix_nanos.saturating_add(Duration::from_secs(20).as_nanos());
    let expected_measure_until =
        expected_washout_until.saturating_add(Duration::from_secs(30).as_nanos());

    assert_eq!(washout_until_unix_nanos, expected_washout_until);
    assert_eq!(measure_until_unix_nanos, expected_measure_until);
}

#[test]
fn interval_event_updates_window_and_emits_noop_decision() {
    let mut runtime = runtime();
    let record = IntervalRecord {
        elapsed_ms: 1_000,
        task: 42,
        samples: 100,
        over_1ms: 3,
        over_2ms: 2,
        over_5ms: 1,
        max_ns: 7_000_000,
        ..IntervalRecord::default()
    };

    let emitted = runtime
        .on_event(MonitorEvent::Interval {
            elapsed_ms: 1_000,
            records: vec![record],
            drop_counters: DropCountersSnapshot::default(),
        })
        .unwrap()
        .unwrap();

    assert_eq!(emitted.decision, "observed");
    assert_eq!(emitted.diagnostic_raw_score_total, 143);
    assert_eq!(runtime.observation().score.total, 143);
    assert_eq!(runtime.observation().scored_task_count, 1);
}

#[test]
fn focus_change_resets_window_and_sets_focus_context() {
    let mut runtime = runtime();

    let emitted = runtime
        .on_event(MonitorEvent::FocusChanged {
            elapsed_ms: 1_000,
            old_kind: None,
            new_kind: FocusGroupKind::Game,
            root_pids: vec![2222],
            member_pids: vec![2222, 2223],
            confidence: 0.90,
            score: 0.95,
            situation: SituationKind::GameFocused,
            reasons: vec!["test focus".to_owned()],
        })
        .unwrap()
        .unwrap();

    assert_eq!(emitted.focus_kind.as_deref(), Some("Game"));
    assert_eq!(emitted.target_root_pid, Some(2222));
    assert_eq!(runtime.observation().focus_kind, Some(FocusGroupKind::Game));
    assert_eq!(
        runtime.observation().primary_situation,
        SituationKind::GameFocused
    );
}

#[test]
fn low_quality_is_reported_in_decision_stream() {
    let mut runtime = runtime();

    let emitted = runtime
        .on_event(MonitorEvent::DataQualityWarning {
            message: "synthetic warning".to_owned(),
        })
        .unwrap()
        .unwrap();

    assert_eq!(emitted.decision, "observed");
    assert!(emitted.data_quality.starts_with("Low"));
    assert_eq!(
        emitted.data_quality_reason_codes,
        vec![
            "insufficient_samples".to_owned(),
            "target_missing".to_owned()
        ]
    );
}

#[test]
fn data_quality_label_names_high_medium_low() {
    assert_eq!(data_quality_label(&OnlineDataQuality::High), "High");
    assert!(
        data_quality_label(&OnlineDataQuality::Medium {
            reasons: vec!["reason".to_owned()]
        })
        .starts_with("Medium")
    );
    assert!(
        data_quality_label(&OnlineDataQuality::Low {
            reasons: vec!["reason".to_owned()]
        })
        .starts_with("Low")
    );
}
