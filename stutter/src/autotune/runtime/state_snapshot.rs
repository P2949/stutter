use super::*;

impl AutotuneRuntime {
    pub fn observation(&self) -> AutotuneObservation {
        self.last_observation.clone()
    }

    pub fn last_decision(&self) -> Option<&AutotuneDecisionStreamEntry> {
        self.last_decision.as_ref()
    }

    pub(crate) fn config(&self) -> &AutotuneRuntimeConfig {
        &self.config
    }

    pub fn controller_state(&self) -> &ControllerRuntimeState {
        &self.controller.state
    }

    pub fn active_profile_state(&self) -> &ActiveProfileState {
        &self.controller.active_profile_state
    }

    pub fn has_active_experiment(&self) -> bool {
        self.live_experiments.has_active_experiment()
    }
}
