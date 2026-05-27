mod lifecycle;
mod executor;
mod journal;
mod manager;
mod model;
mod rollback;
mod scoring;
#[cfg(test)]
mod tests;

pub use manager::LiveExperimentManager;
pub use model::{
    LiveExperiment, LiveExperimentEvent, LiveExperimentHistoryContext, LiveExperimentManagerInput,
};

#[cfg(test)]
pub(crate) use manager::LiveExperimentRuntimeState;
#[cfg(test)]
pub(crate) use executor::{LiveExperimentActionExecutor, RuntimeLiveExperimentActionExecutor};
#[cfg(test)]
pub(crate) use crate::autotune::{
    comparison::ExperimentResult,
    controller_journal::ControllerJournalState,
    state::ControllerPhase,
};
