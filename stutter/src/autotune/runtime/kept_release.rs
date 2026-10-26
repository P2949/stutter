//! Releasing kept actions: when the workload they were kept for is gone, and when the controller
//! stops. A restore that fails leaves the action kept and in the ledger rather than forgetting it.
use super::{
    AutotuneRuntime, DAEMON_EMERGENCY_RESTORE_COMMAND, tick::resolve_privileged_action_service,
};
use crate::{
    autotune::live_experiment::{
        LiveExperimentManager, LiveExperimentManagerInput, RuntimeLiveExperimentActionExecutor,
    },
    daemon::privilege::PrivilegedActionService,
};

impl AutotuneRuntime {
    pub(super) fn live_experiment_input<'a>(
        &self,
        privileged_action_service: Option<&'a dyn PrivilegedActionService>,
    ) -> LiveExperimentManagerInput<'a> {
        LiveExperimentManagerInput {
            mode: self.config.mode(),
            daemon_policy: self.config.daemon_policy.clone(),
            controller_policy: self.controller.policy.clone(),
            simulate_action_effects: self.config.simulate_action_effects,
            washout: self.config.washout.clone(),
            candidate_window_seconds: self.config.candidate_window_seconds,
            manual_restore_command: DAEMON_EMERGENCY_RESTORE_COMMAND,
            controller_journal_path: self.config.controller_journal_path.clone(),
            audit_path: None,
            #[cfg(test)]
            exit_rollback_registry: None,
            privileged_action_service,
        }
    }

    pub fn has_kept_actions(&self) -> bool {
        self.controller.active_profile_state.kept_action_count() > 0
    }

    /// Roll back every kept action. Returns how many could not be restored (they stay kept).
    pub(super) fn release_kept_actions(&mut self, reason: &str) -> anyhow::Result<usize> {
        if !self.has_kept_actions() {
            return Ok(0);
        }
        let service = resolve_privileged_action_service(&self.config)?;
        let input = self.live_experiment_input(service.as_service());
        let mut observation = self.last_observation.clone();
        observation.now_unix_nanos = crate::audit::unix_nanos_now();
        let release = LiveExperimentManager::release_kept_actions_with_executor(
            &input,
            &mut self.controller.active_profile_state,
            &observation,
            reason,
            &mut RuntimeLiveExperimentActionExecutor,
        );
        log::info!(
            "autotune_kept_actions_released reason={reason} restored={} failed={}",
            release.restored,
            release.failed
        );
        Ok(release.failed)
    }

    /// Remember what was tried, beside the controller journal, so a restart does not retry a
    /// losing candidate immediately. Best effort: a write failure only costs one repeated trial.
    pub(super) fn persist_candidate_memory(&self) {
        let Some(path) = candidate_memory_path_for(&self.config) else {
            return;
        };
        if let Err(error) = self.controller.state.candidate_memory.save(&path) {
            log::warn!(
                "candidate memory not persisted to {}: {error:#}",
                path.display()
            );
        }
    }
}

/// Beside the controller journal the session was configured with; a session with no journal (a
/// unit-test runtime) persists nothing. A live session always has one.
pub(super) fn candidate_memory_path_for(
    config: &super::AutotuneRuntimeConfig,
) -> Option<std::path::PathBuf> {
    let journal = config.controller_journal_path.as_deref()?;
    Some(crate::autotune::candidate_memory::persistence::candidate_memory_path(journal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::runtime::AutotuneRuntimeConfig;

    #[test]
    fn candidate_memory_follows_the_journal_the_session_uses() {
        let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1), None);
        config.controller_journal_path = Some("/run/x/controller.json".into());
        assert_eq!(
            candidate_memory_path_for(&config),
            Some(std::path::PathBuf::from("/run/x/candidate-memory.json"))
        );
        config.controller_journal_path = None;
        assert_eq!(candidate_memory_path_for(&config), None);
    }
}
