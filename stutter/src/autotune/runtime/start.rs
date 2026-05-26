use std::{collections::VecDeque, time::Duration};

use super::*;

impl OnlineAutotuneController {
    pub fn new(daemon_policy: DaemonPolicy, window_seconds: u64) -> Self {
        Self {
            policy: ControllerPolicy::from_daemon_policy(&daemon_policy),
            state: ControllerRuntimeState::default(),
            window: RollingWindow::new(Duration::from_secs(window_seconds)),
            active_profile_state: ActiveProfileState::default(),
        }
    }
}

impl AutotuneRuntime {
    pub fn new(config: AutotuneRuntimeConfig) -> Self {
        let daemon_policy = config.daemon_policy.clone();
        let window_seconds = config.window_seconds;
        let controller = OnlineAutotuneController::new(daemon_policy, window_seconds);

        Self {
            target_state: RuntimeTargetState::new(config.tree_pid()),
            controller,
            latest_focus: None,
            latest_drop_counters: DropCountersSnapshot::default(),
            recent_diagnoses: VecDeque::new(),
            activity_classifier: ActivityClassifier::new(5),
            live_experiments: LiveExperimentManager::new(),
            phase_machine: AutotuneRuntimePhaseMachine::default(),
            last_observation: AutotuneObservation::default(),
            last_decision: None,
            last_plan_result: None,
            last_dry_run_plan_files: Vec::new(),
            pending_history_context: None,
            config,
        }
    }
}
