use super::*;

#[test]
fn runtime_preserves_selected_focus_membership_and_configured_root_precedence() {
    for (configured_root, expected_samples) in [(None, 2_000), (Some(42), 1_000)] {
        let mut config = AutotuneRuntimeConfig::observe(None, configured_root, None);
        config.history_log = None;
        let mut runtime = AutotuneRuntime::new(config);
        let focus_event = runtime
            .on_event(MonitorEvent::FocusChanged {
                elapsed_ms: 0,
                old_kind: None,
                new_kind: FocusGroupKind::Game,
                root_pids: vec![99],
                member_pids: vec![42, 99],
                confidence: 0.95,
                score: 1.0,
                situation: crate::autotune::state::SituationKind::GameCpuSchedulerPressure,
                reasons: Vec::new(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            focus_event.target_root_pid,
            Some(configured_root.unwrap_or(99))
        );
        for pid in [42, 99] {
            runtime.target_state.active_tasks.insert(
                pid.into(),
                crate::process_tree::TaskInfo {
                    tid: pid.into(),
                    process_pid: pid.into(),
                    process_ppid: 1.into(),
                    comm: "game".to_owned(),
                    process_comm: "game".to_owned(),
                    process_starttime_ticks: Some(100),
                    task_starttime_ticks: Some(100),
                    exe_dev: None,
                    exe_ino: None,
                    class: TaskClass::Game,
                    sched_policy: None,
                    from_cgroup: false,
                },
            );
            let mut histogram = crate::metrics::LatencyHistogram::new();
            for _ in 0..1_000 {
                histogram.record(1_000_000);
            }
            runtime.controller.window.push_interval(IntervalRecord {
                elapsed_ms: 1_000,
                task: pid,
                process_pid: Some(pid),
                class: TaskClass::Game,
                samples: 1_000,
                max_ns: 1_000_000,
                histogram: histogram.snapshot(),
                ..IntervalRecord::default()
            });
        }
        let observation = runtime.build_observation();
        assert_eq!(
            observation.objective_signals.runnable_samples,
            Some(expected_samples)
        );
    }
}

#[test]
fn runtime_starts_with_default_observation() {
    let runtime = runtime();
    let observation = runtime.observation();

    assert!(!observation.target_present);
    assert_eq!(observation.target_root_pid, None);
    assert_eq!(observation.score.total, 0);
    assert!(observation.data_quality.blocks_action());
}

#[test]
fn apply_medium_runtime_can_start_reversible_medium_experiment_in_simulation() {
    let daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::ApplyMediumRisk,
        ActionSource::AutotuneRuntime,
        Some(1234),
        None,
    );
    let mut daemon_config = daemon_config;
    daemon_config.autotune.allow_medium_risk_apply = true;
    let mut config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None)
        .with_simulated_action_effects();
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-medium".to_owned()),
        SafetyClass::ReversibleMediumRisk,
    );

    runtime
        .apply_decision_side_effects(
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate,
                reason: "test medium start".to_owned(),
            },
            "test medium start",
        )
        .unwrap();

    assert!(runtime.has_active_experiment());
    assert_eq!(runtime.controller.state.phase, ControllerPhase::Measuring);
    assert_eq!(
        runtime
            .pending_history_context
            .as_ref()
            .map(|context| context.action_kind.as_str()),
        Some("fake")
    );
}

#[test]
fn controller_session_finish_rolls_back_active_experiment_on_clean_stop() {
    let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
        .with_simulated_action_effects();
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let observation = high_quality_game_observation_with_focus_confidence(0.95);
    runtime.last_observation = observation.clone();

    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-low-risk-stop".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );

    runtime
        .apply_decision_side_effects(
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate,
                reason: "test start".to_owned(),
            },
            "test start",
        )
        .unwrap();

    assert!(runtime.has_active_experiment());

    let exit =
        finish_autotune_controller_session(&mut runtime, Ok("stop requested".to_owned())).unwrap();

    assert_eq!(exit.reason, "stop requested");
    assert!(!runtime.has_active_experiment());
    assert_eq!(runtime.controller.state.phase, ControllerPhase::Cooldown);
    assert!(runtime.controller.state.active_experiment.is_none());
}

fn simulated_runtime_with_active_experiment() -> AutotuneRuntime {
    let mut config =
        AutotuneRuntimeConfig::observe(None, Some(1234), None).with_simulated_action_effects();
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    runtime.prepare_runtime_phase_for_apply_candidate().unwrap();
    runtime
        .transition_runtime_phase(AutotuneRuntimePhase::ApplyingCandidate, "test")
        .unwrap();
    runtime
        .live_experiments
        .set_current_for_tests(fake_live_experiment(CandidateAction::fake(
            crate::actions::ActionId::new("suspend-candidate".to_owned()),
            SafetyClass::ReversibleLowRisk,
        )));
    runtime
        .controller
        .window
        .push_intervals([crate::recorder::IntervalRecord {
            samples: 10,
            ..Default::default()
        }]);
    runtime
}

#[test]
fn suspend_gap_invalidates_window_reverts_candidate_and_stabilizes() {
    use crate::daemon::lifecycle::{DaemonLifecycleAction, LifecycleClockSample};
    let mut runtime = simulated_runtime_with_active_experiment();
    assert_eq!(runtime.controller.window.interval_count(), 1);
    let sample = |realtime_ms, monotonic_ms| LifecycleClockSample {
        realtime_ms,
        monotonic_ms,
    };
    assert!(
        runtime
            .observe_lifecycle_clock(sample(1_000_000, 1_000))
            .unwrap()
            .is_none()
    );
    // Wall clock advanced 60 s while the monotonic clock advanced 1 s: suspended.
    let event = runtime
        .observe_lifecycle_clock(sample(1_060_000, 2_000))
        .unwrap()
        .expect("resume must be detected");
    assert!(matches!(
        event,
        crate::daemon::lifecycle::DaemonLifecycleEvent::ResumeDetected { slept_ms: 59_000 }
    ));
    assert_eq!(runtime.controller.window.interval_count(), 0);
    assert!(!runtime.live_experiments.has_active_experiment());
    let transition = runtime.lifecycle.last_transition.as_ref().unwrap();
    assert!(matches!(
        transition.event,
        crate::daemon::lifecycle::DaemonLifecycleEvent::ResumeDetected { .. }
    ));
    assert!(
        transition
            .actions
            .contains(&DaemonLifecycleAction::ClearMeasurementWindows)
    );
    assert!(transition.stabilization_ms.is_some());
    let cooldown = runtime.controller.state.cooldown_until_unix_nanos.unwrap();
    assert!(cooldown > crate::audit::unix_nanos_now());
}

#[test]
fn clock_jitter_below_threshold_does_not_touch_measurements_or_candidate() {
    use crate::daemon::lifecycle::LifecycleClockSample;
    let mut runtime = simulated_runtime_with_active_experiment();
    for (realtime_ms, monotonic_ms) in [(1_000_000, 1_000), (1_004_000, 2_000), (1_005_500, 3_000)]
    {
        assert!(
            runtime
                .observe_lifecycle_clock(LifecycleClockSample {
                    realtime_ms,
                    monotonic_ms,
                })
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(runtime.controller.window.interval_count(), 1);
    assert!(runtime.live_experiments.has_active_experiment());
}
