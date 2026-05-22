use std::path::PathBuf;

use crate::{
    actions::{RollbackToken, SafetyClass},
    autotune::{
        controller::ControllerPolicy,
        experiment::{ExperimentId, WindowScore},
        objective::ObjectiveSignals,
        observation::ActiveConfigSnapshot,
        planning::candidate::CandidateAction,
        washout::WashoutWindowConfig,
    },
    daemon::{DaemonPolicy, policy::DaemonMode, privilege::PrivilegedActionService},
};

#[derive(Clone, Debug)]
pub struct LiveExperiment {
    pub experiment_id: ExperimentId,
    pub candidate: CandidateAction,
    pub safety_class: SafetyClass,
    pub mode: DaemonMode,
    pub baseline_score: WindowScore,
    pub baseline_signals: ObjectiveSignals,
    pub baseline_active_config: Option<ActiveConfigSnapshot>,
    pub applied_unix_nanos: u128,
    pub washout_until_unix_nanos: u128,
    pub measure_until_unix_nanos: u128,
    pub rollback: RollbackToken,
}
impl LiveExperiment {
    pub fn candidate_name(&self) -> &str {
        self.candidate.profile_name()
    }

    pub fn action_id(&self) -> String {
        self.candidate.action_id().into_string()
    }
}
#[derive(Clone, Debug)]
pub struct LiveExperimentManagerInput<'a> {
    pub mode: DaemonMode,
    pub daemon_policy: DaemonPolicy,
    pub controller_policy: ControllerPolicy,
    pub simulate_action_effects: bool,
    pub washout: WashoutWindowConfig,
    pub candidate_window_seconds: u64,
    pub manual_restore_command: &'static str,
    pub controller_journal_path: Option<PathBuf>,
    #[cfg(test)]
    pub exit_rollback_registry: Option<&'a crate::autotune::shutdown::ActiveAutotuneActionRegistry>,
    pub privileged_action_service: Option<&'a dyn PrivilegedActionService>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveExperimentEvent {
    Noop,
    Started,
    Kept,
    Reverted,
    CooldownEntered,
    Faulted,
}
#[derive(Clone, Debug)]
pub struct LiveExperimentHistoryContext {
    pub experiment_id: String,
    pub action_id: String,
    pub candidate_name: String,
    pub action_kind: String,
    pub mode: DaemonMode,
    pub safety_class: SafetyClass,
    pub score_before: Option<WindowScore>,
    pub score_after: Option<WindowScore>,
    pub rollback_performed: bool,
    pub rollback_policy: String,
    pub cooldown_until_unix_nanos: Option<u128>,
    pub manual_restore_command: Option<String>,
}
#[derive(Clone, Debug)]
pub struct LiveExperimentOutcome {
    pub event: LiveExperimentEvent,
    pub history_context: Option<LiveExperimentHistoryContext>,
    pub clear_measurement_window: bool,
}
impl LiveExperimentEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Noop => "noop",
            Self::Started => "started",
            Self::Kept => "kept",
            Self::Reverted => "reverted",
            Self::CooldownEntered => "cooldown_entered",
            Self::Faulted => "faulted",
        }
    }
}
impl LiveExperimentOutcome {
    pub(super) fn noop() -> Self {
        Self {
            event: LiveExperimentEvent::Noop,
            history_context: None,
            clear_measurement_window: false,
        }
    }

    pub(super) fn event(event: LiveExperimentEvent) -> Self {
        Self {
            event,
            history_context: None,
            clear_measurement_window: false,
        }
    }

    pub(super) fn with_history(
        event: LiveExperimentEvent,
        history_context: LiveExperimentHistoryContext,
    ) -> Self {
        Self {
            event,
            history_context: Some(history_context),
            clear_measurement_window: false,
        }
    }

    pub(super) fn with_clear_measurement_window(mut self) -> Self {
        self.clear_measurement_window = true;
        self
    }
}
