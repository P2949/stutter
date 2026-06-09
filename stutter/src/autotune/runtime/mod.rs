use std::collections::VecDeque;
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use std::{collections::BTreeMap, path::PathBuf};

pub(crate) mod config;
pub(crate) mod daemon_state;
pub(crate) mod decision_view;
pub(crate) mod emission;
pub(crate) mod history;
mod phase;
mod plan_output;
pub(crate) mod planning;
mod restore;
pub(crate) mod session;
mod start;
mod state_snapshot;
mod stop;
pub(crate) mod stream;
pub(crate) mod target_state;
mod tick;
pub(crate) mod worker;

pub(crate) use config::validate_runtime_config;
pub use config::{AutotuneRuntimeConfig, daemon_config_for_runtime_mode};
pub use daemon_state::daemon_phase_from_controller_phase;
pub(crate) use decision_view::data_quality_label;
pub(crate) use phase::AutotuneRuntimePhase;
#[cfg(test)]
pub(crate) use session::finish_autotune_controller_session;
pub use session::{AutotuneControllerExit, run_autotune_controller_session};
pub(crate) use stream::emit_decision_stream_entry;
pub use stream::{AutotuneDecisionStreamEntry, AutotuneDryRunPlanFileSummary};
pub use target_state::RuntimeTargetState;

#[cfg(test)]
use self::planning::top_denied_reason_for_plan;
use self::{history::RuntimeHistoryContext, phase::AutotuneRuntimePhaseMachine};
#[cfg(test)]
use crate::autotune::{
    controller::decide_autotune_transition,
    decision::{AutotuneDecision, ExperimentId},
    planning::candidate::CandidateAction,
    state::{ControllerPhase, SituationKind},
};
use crate::{
    autotune::{
        activity::ActivityClassifier,
        controller::{ControllerPolicy, ControllerRuntimeState},
        kept::ActiveProfileState,
        live_experiment::LiveExperimentManager,
        observation::AutotuneObservation,
        observation_builder::AutotuneObservationFocus,
        planner::PlanResult,
        rolling_window::RollingWindow,
    },
    daemon::DaemonPolicy,
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
};
#[cfg(test)]
use crate::{daemon::policy::DaemonMode, session_events::MonitorEvent};

pub const DEFAULT_RUNTIME_WINDOW_SECONDS: u64 = 30;
pub const DEFAULT_RECENT_DIAGNOSIS_LIMIT: usize = 16;

const DAEMON_EMERGENCY_RESTORE_COMMAND: &str = "stutter daemon emergency-restore";

#[derive(Clone, Debug)]
pub struct OnlineAutotuneController {
    pub policy: ControllerPolicy,
    pub state: ControllerRuntimeState,
    pub window: RollingWindow,
    pub active_profile_state: ActiveProfileState,
}

#[derive(Debug)]
pub struct AutotuneRuntime {
    config: AutotuneRuntimeConfig,
    controller: OnlineAutotuneController,
    latest_focus: Option<AutotuneObservationFocus>,
    target_state: RuntimeTargetState,
    latest_drop_counters: DropCountersSnapshot,
    recent_diagnoses: VecDeque<LiveDiagnosisEntry>,
    activity_classifier: ActivityClassifier,
    live_experiments: LiveExperimentManager,
    phase_machine: AutotuneRuntimePhaseMachine,
    last_observation: AutotuneObservation,
    last_decision: Option<AutotuneDecisionStreamEntry>,
    last_plan_result: Option<PlanResult>,
    last_dry_run_plan_files: Vec<AutotuneDryRunPlanFileSummary>,
    pending_history_context: Option<RuntimeHistoryContext>,
}

#[cfg(test)]
mod tests;
