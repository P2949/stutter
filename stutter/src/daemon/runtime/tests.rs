    use std::{collections::BTreeMap, path::PathBuf};

    use super::*;
    use crate::{
        actions::SafetyClass,
        autotune::runtime::AutotuneDecisionStreamEntry,
        config::model::MonitorConfig,
        daemon::{
            DaemonConfig,
            health::SystemHealthState,
            lifecycle::{DaemonLifecycleAction, DaemonLifecycleEvent},
            policy::{ActionSource, DaemonMode},
            state::{DaemonExperimentState, DaemonRollbackState, DaemonPhase, DAEMON_STATE_SCHEMA_VERSION},
        },
        process_tree::{TaskClass, TaskInfo},
        session_events::MonitorEvent,
    };

    fn runtime_config() -> DaemonRuntimeConfig {
        DaemonRuntimeConfig {
            daemon_config: DaemonConfig {
                mode: DaemonMode::ApplyLowRisk,
                source: ActionSource::Cli,
                ..DaemonConfig::default()
            },
            monitor_config: MonitorConfig::default(),
            autotune_config: None,
            state_snapshot_path: PathBuf::from("/tmp/stutter-daemon-runtime-state.json"),
            rollback_on_stop: true,
        }
    }

    #[test]
    fn daemon_runtime_new_starts_in_init_phase_without_active_state() {
        let runtime = DaemonRuntime::new(runtime_config());

        assert_eq!(runtime.phase(), DaemonPhase::Init);
        assert_eq!(runtime.state().phase, DaemonPhase::Init);
        assert_eq!(runtime.state().mode, DaemonMode::ApplyLowRisk);
        assert_eq!(runtime.state().schema_version, DAEMON_STATE_SCHEMA_VERSION);
        assert!(runtime.state().last_decision.is_none());
        assert!(runtime.state().faulted.is_none());
        assert!(runtime.state().active_target.is_none());
        assert!(runtime.state().active_experiment.is_none());
        assert!(runtime.state().active_rollback.is_none());
        assert!(runtime.config.rollback_on_stop);
        assert_eq!(runtime.config.monitor_config, MonitorConfig::default());
        assert!(runtime.config.autotune_config.is_none());
    }

    #[test]
    fn daemon_runtime_transition_updates_phase_state_and_last_decision() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        let transition = runtime.transition_to(DaemonPhase::Recover, "startup recovery begins");

        assert_eq!(transition.from, DaemonPhase::Init);
        assert_eq!(transition.to, DaemonPhase::Recover);
        assert_eq!(transition.reason, "startup recovery begins");
        assert_eq!(runtime.phase(), DaemonPhase::Recover);
        assert_eq!(runtime.state().phase, DaemonPhase::Recover);

        let last_decision = runtime.state().last_decision.as_ref().unwrap();
        assert_eq!(last_decision.decision, "daemon_transition");
        assert_eq!(last_decision.reason, "startup recovery begins");
        assert_eq!(last_decision.unix_nanos, Some(transition.unix_nanos));
        assert_eq!(last_decision.diagnostic_current_raw_score_total, None);
        assert!(runtime.state().faulted.is_none());
    }

    #[test]
    fn daemon_runtime_transitions_return_explicit_order() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        let first = runtime.transition_to(DaemonPhase::Recover, "recover controller journal");
        let second = runtime.transition_to(DaemonPhase::Observe, "recovery complete");

        assert_eq!(first.from, DaemonPhase::Init);
        assert_eq!(first.to, DaemonPhase::Recover);
        assert_eq!(second.from, DaemonPhase::Recover);
        assert_eq!(second.to, DaemonPhase::Observe);
        assert_eq!(runtime.phase(), DaemonPhase::Observe);
        assert_eq!(runtime.state().phase, DaemonPhase::Observe);
        assert_eq!(runtime.health.transition_count, 2);
        assert_eq!(
            runtime.health.last_transition_unix_nanos,
            Some(second.unix_nanos)
        );
    }

    #[test]
    fn daemon_runtime_fault_state_is_set_only_when_transitioning_to_faulted() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        runtime.transition_to(DaemonPhase::Recover, "recover controller journal");
        assert!(runtime.state().faulted.is_none());

        runtime.transition_to(DaemonPhase::Faulted, "startup recovery failed");

        let faulted = runtime.state().faulted.as_ref().unwrap();
        assert_eq!(runtime.phase(), DaemonPhase::Faulted);
        assert_eq!(runtime.state().phase, DaemonPhase::Faulted);
        assert_eq!(faulted.reason, "startup recovery failed");
        assert_eq!(
            faulted.manual_restore_command.as_deref(),
            Some("stutter autotune restore")
        );

        runtime.transition_to(DaemonPhase::Shutdown, "operator requested shutdown");

        assert_eq!(runtime.phase(), DaemonPhase::Shutdown);
        assert_eq!(runtime.state().phase, DaemonPhase::Shutdown);
        assert!(runtime.state().faulted.is_none());
    }

    #[test]
    fn daemon_runtime_lifecycle_suspend_schedules_rollback_for_active_experiment() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.transition_to(DaemonPhase::Measure, "candidate measurement running");
        runtime.state.active_experiment = Some(active_experiment());
        runtime.state.active_rollback = Some(active_rollback(true));

        let transition = runtime.handle_lifecycle_event(DaemonLifecycleEvent::SuspendDetected);

        assert_eq!(runtime.phase(), DaemonPhase::Rollback);
        assert_eq!(runtime.state().phase, DaemonPhase::Rollback);
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::RollbackActiveExperiment)
        );
        assert_eq!(
            runtime
                .state()
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("daemon_lifecycle")
        );
        assert_eq!(
            runtime.state().health.reason_code.as_deref(),
            Some("suspend_resume_stabilizing")
        );
    }

    #[test]
    fn daemon_runtime_lifecycle_resume_pauses_and_marks_health_stabilizing() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.transition_to(DaemonPhase::Measure, "candidate measurement running");

        let transition = runtime
            .handle_lifecycle_event(DaemonLifecycleEvent::ResumeDetected { slept_ms: 8_000 });

        assert_eq!(runtime.phase(), DaemonPhase::Paused);
        assert_eq!(runtime.state().phase, DaemonPhase::Paused);
        assert_eq!(
            runtime.state().health.state,
            SystemHealthState::SuspendedResumed
        );
        assert!(!runtime.state().health.ok_for_apply);
        assert_eq!(transition.stabilization_ms, Some(10_000));
        assert!(
            runtime
                .state()
                .degraded
                .iter()
                .any(|status| status.category == "lifecycle"
                    && status.message.contains("wait_stabilization"))
        );
    }

    #[test]
    fn daemon_runtime_lifecycle_target_exit_without_rollback_enters_observe_only() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.transition_to(DaemonPhase::Measure, "candidate measurement running");
        runtime.state.active_experiment = Some(active_experiment());
        runtime.state.active_rollback = None;

        let transition =
            runtime.handle_lifecycle_event(DaemonLifecycleEvent::TargetExited { pid: 1234 });

        assert_eq!(runtime.phase(), DaemonPhase::Observe);
        assert_eq!(runtime.state().phase, DaemonPhase::Observe);
        assert_eq!(runtime.state().mode, DaemonMode::Observe);
        assert!(runtime.state().active_experiment.is_none());
        assert!(runtime.state().active_rollback.is_none());
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::EnterObserveOnly)
        );
    }

    #[test]
    fn daemon_runtime_handle_event_drives_startup_transitions() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        let init = runtime
            .handle_event(DaemonRuntimeEvent::Initialized)
            .unwrap();
        assert_eq!(init.from, DaemonPhase::Init);
        assert_eq!(init.to, DaemonPhase::Recover);

        let recovered = runtime
            .handle_event(DaemonRuntimeEvent::RecoveryCompleted)
            .unwrap();
        assert_eq!(recovered.from, DaemonPhase::Recover);
        assert_eq!(recovered.to, DaemonPhase::Observe);
        assert_eq!(runtime.state().phase, DaemonPhase::Observe);
    }

    #[test]
    fn daemon_runtime_monitor_target_snapshot_updates_status_cache() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.handle_event(DaemonRuntimeEvent::RecoveryCompleted);
        let mut targets = BTreeMap::new();
        targets.insert(11.into(), task_info(11, 10, "game-main"));
        targets.insert(12.into(), task_info(12, 10, "game-render"));

        let transition = runtime.handle_event(DaemonRuntimeEvent::MonitorEvent(
            MonitorEvent::TargetSnapshot {
                elapsed_ms: 100,
                active_targets: targets,
                removed_targets: Vec::new(),
            },
        ));

        assert!(transition.is_none());
        let target = runtime.state().active_target.as_ref().unwrap();
        assert_eq!(target.root_pid, Some(10));
        assert_eq!(target.active_targets, 2);
        assert_eq!(target.comm.as_deref(), Some("game-main"));
    }

    #[test]
    fn daemon_runtime_empty_target_snapshot_rolls_back_active_experiment() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.transition_to(DaemonPhase::Measure, "candidate measurement running");
        runtime.state.active_experiment = Some(active_experiment());
        runtime.state.active_rollback = Some(active_rollback(true));

        let transition = runtime
            .handle_event(DaemonRuntimeEvent::MonitorEvent(
                MonitorEvent::TargetSnapshot {
                    elapsed_ms: 200,
                    active_targets: BTreeMap::new(),
                    removed_targets: vec![1234],
                },
            ))
            .unwrap();

        assert_eq!(transition.from, DaemonPhase::Measure);
        assert_eq!(transition.to, DaemonPhase::Rollback);
        assert_eq!(runtime.state().phase, DaemonPhase::Rollback);
        assert!(runtime.state().active_target.is_none());
    }

    #[test]
    fn daemon_runtime_decision_event_updates_phase_target_and_last_decision() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        let transition = runtime
            .handle_event(DaemonRuntimeEvent::Decision(Box::new(
                AutotuneDecisionStreamEntry {
                    unix_nanos: 42,
                    phase: "Measuring".to_owned(),
                    mode: "ApplyLowRisk".to_owned(),
                    focus_kind: Some("Game".to_owned()),
                    focus_confidence: 0.9,
                    target_root_pid: Some(1234),
                    active_target_count: 3,
                    situation: "GameCpuSchedulerPressure".to_owned(),
                    situation_confidence: 0.9,
                    situation_evidence: Vec::new(),
                    situation_blockers: Vec::new(),
                    protected_tasks_count: 0,
                    candidate_count: 0,
                    top_denied_reason: None,
                    planner: None,
                    dry_run_plan_files: Vec::new(),
                    diagnostic_raw_score_total: 900,
                    data_quality: "High".to_owned(),
                    data_quality_reason_codes: Vec::new(),
                    decision: "start_experiment".to_owned(),
                    reason: "candidate evidence is strong".to_owned(),
                },
            )))
            .unwrap();

        assert_eq!(transition.from, DaemonPhase::Init);
        assert_eq!(transition.to, DaemonPhase::Measure);
        assert_eq!(runtime.state().mode, DaemonMode::ApplyLowRisk);
        assert_eq!(
            runtime
                .state()
                .active_target
                .as_ref()
                .map(|target| (target.root_pid, target.active_targets)),
            Some((Some(1234), 3))
        );
        let last_decision = runtime.state().last_decision.as_ref().unwrap();
        assert_eq!(last_decision.decision, "start_experiment");
        assert_eq!(last_decision.diagnostic_current_raw_score_total, Some(900));
        assert_eq!(last_decision.unix_nanos, Some(42));
    }

    fn active_experiment() -> DaemonExperimentState {
        DaemonExperimentState {
            experiment_id: "experiment-1".to_owned(),
            action_id: "action-1".to_owned(),
            candidate_name: Some("game-candidate".to_owned()),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            started_unix_nanos: Some(100),
        }
    }

    fn active_rollback(rollback_available: bool) -> DaemonRollbackState {
        DaemonRollbackState {
            action_id: "action-1".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            rollback_available,
            token: None,
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
        }
    }

    fn task_info(tid: u32, process_pid: u32, comm: &str) -> TaskInfo {
        TaskInfo {
            tid: tid.into(),
            process_pid: process_pid.into(),
            process_ppid: 1.into(),
            comm: comm.to_owned(),
            process_comm: comm.to_owned(),
            process_starttime_ticks: Some(100),
            task_starttime_ticks: Some(100 + u64::from(tid)),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Game,
            sched_policy: None,
            from_cgroup: false,
        }
    }
