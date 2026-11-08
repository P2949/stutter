use std::collections::VecDeque;
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use std::{collections::BTreeMap, path::PathBuf};

pub(crate) mod acceptance;
pub(crate) mod config;
mod context_signature;
pub(crate) mod daemon_state;
pub(crate) mod decision_view;
pub(crate) mod emission;
mod frame_source;
mod guards;
pub(crate) mod history;
mod kept_release;
mod kpi_source;
mod lifecycle;
mod phase;
mod plan_output;
pub(crate) mod planning;
mod restore;
mod sensor_hub;
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
    /// The last cumulative BPF snapshot, to turn the next one into a per-interval loss.
    previous_drop_counters: DropCountersSnapshot,
    recent_diagnoses: VecDeque<LiveDiagnosisEntry>,
    activity_classifier: ActivityClassifier,
    live_experiments: LiveExperimentManager,
    phase_machine: AutotuneRuntimePhaseMachine,
    last_observation: AutotuneObservation,
    last_decision: Option<AutotuneDecisionStreamEntry>,
    last_plan_result: Option<PlanResult>,
    last_dry_run_plan_files: Vec<AutotuneDryRunPlanFileSummary>,
    pending_history_context: Option<RuntimeHistoryContext>,
    lifecycle: lifecycle::RuntimeLifecycle,
    /// Reads sustained background CPU consumers while a game is the focused workload.
    offender_sampler: std::sync::Mutex<crate::autotune::offenders::OffenderSampler>,
    /// The bytes of the candidate memory last written, so an unchanged memory is not rewritten.
    candidate_memory_saved: std::sync::Mutex<Option<Vec<u8>>>,
    /// The independently collected frame source, when one was attached.
    frame_source: Option<crate::telemetry::live_source::SharedFrameSource>,
    kpi_collector: Option<crate::autotune::objective::source::KpiCollector>,
    kpi_observation: Option<crate::autotune::objective::source::KpiObservation>,
    /// The last owner probe and when it ran; `/proc` is not rescanned on every tick.
    owner_cache: std::sync::Mutex<Option<(std::time::Instant, crate::integrations::OwnerReport)>>,
    /// The frame source found for the focused launch, when `frame_capture_root` is configured.
    managed_source: Option<crate::telemetry::managed_source::ManagedFrameSource>,
    frame_epoch: u64,
    /// Maps the game's frame clock onto the monitor clock the rolling window is keyed on.
    frame_clock: crate::telemetry::frame_clock::FrameClock,
    /// The newest interval's monitor-clock time and when it arrived: the controller's "now".
    interval_anchor: Option<(u64, std::time::Instant)>,
    last_frame_discovery: Option<std::time::Instant>,
    /// Always-on cheap sensors, sampled once per evaluation.
    sensor_hub: sensor_hub::SensorHub,
    /// The cached context signature: when computed, for which inventory hash, and its value.
    context_cache: std::sync::Mutex<Option<(std::time::Instant, String, String)>>,
    /// The domains another owner claimed at the last owner probe.
    claimed_domains: std::collections::BTreeSet<crate::integrations::ResourceDomain>,
}

#[cfg(test)]
mod tests;
