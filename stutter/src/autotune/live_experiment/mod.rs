mod acceptance;
mod evidence;
mod executor;
mod journal;
pub(crate) use journal::reserve_experiment_restore_path;
mod kept_rollback;
mod lifecycle;
mod manager;
mod observation_study;
pub use observation_study::{KpiObservationStudyConfig, KpiObservationStudyEvidence};
mod model;
mod rollback;
mod scoring;
mod stale;
#[cfg(test)]
mod tests;

pub(crate) use acceptance::FaultInjectingExecutor;
pub(crate) use executor::{LiveExperimentActionExecutor, RuntimeLiveExperimentActionExecutor};
pub use manager::LiveExperimentManager;
pub(crate) use manager::LiveExperimentRuntimeState;
pub use model::{
    LiveExperiment, LiveExperimentEvent, LiveExperimentHistoryContext, LiveExperimentManagerInput,
};

#[cfg(test)]
pub(crate) use crate::autotune::{
    comparison::ExperimentResult, controller_journal::ControllerJournalState,
    state::ControllerPhase,
};
