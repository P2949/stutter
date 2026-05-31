//! Runtime history context models and history-log facade exports.

use crate::{
    actions::{ActionId, SafetyClass},
    autotune::{
        experiment::{ExperimentId, WindowScore},
        live_experiment::LiveExperimentHistoryContext,
        observation::AutotuneObservation,
        state::ControllerPhase,
    },
    daemon::policy::DaemonMode,
};

#[derive(Clone, Debug)]
pub(crate) struct RuntimeHistoryContext {
    pub(crate) experiment_id: ExperimentId,
    pub(crate) action_id: ActionId,
    pub(crate) candidate_name: String,
    pub(crate) action_kind: String,
    pub(crate) mode: DaemonMode,
    pub(crate) safety_class: SafetyClass,
    pub(crate) score_before: Option<WindowScore>,
    pub(crate) score_after: Option<WindowScore>,
    pub(crate) rollback_performed: bool,
    pub(crate) rollback_policy: String,
    pub(crate) cooldown_until_unix_nanos: Option<u128>,
    pub(crate) manual_restore_command: Option<String>,
}

impl RuntimeHistoryContext {
    pub(crate) fn rollback_policy_with_metadata(&self) -> String {
        let mut parts = vec![self.rollback_policy.clone()];

        if let Some(cooldown_until_unix_nanos) = self.cooldown_until_unix_nanos {
            parts.push(format!(
                "cooldown_until_unix_nanos={cooldown_until_unix_nanos}"
            ));
        }

        if let Some(manual_restore_command) = self.manual_restore_command.as_deref() {
            parts.push(format!(
                "manual_restore_command={}",
                manual_restore_command.replace(' ', "_")
            ));
        }

        parts.join(";")
    }
}

impl From<LiveExperimentHistoryContext> for RuntimeHistoryContext {
    fn from(context: LiveExperimentHistoryContext) -> Self {
        Self {
            experiment_id: context.experiment_id,
            action_id: context.action_id,
            candidate_name: context.candidate_name,
            action_kind: context.action_kind,
            mode: context.mode,
            safety_class: context.safety_class,
            score_before: context.score_before,
            score_after: context.score_after,
            rollback_performed: context.rollback_performed,
            rollback_policy: context.rollback_policy,
            cooldown_until_unix_nanos: context.cooldown_until_unix_nanos,
            manual_restore_command: context.manual_restore_command,
        }
    }
}

pub(crate) struct LifecycleHistoryEventInput<'a> {
    pub(crate) observation: &'a AutotuneObservation,
    pub(crate) context: &'a RuntimeHistoryContext,
    pub(crate) phase: ControllerPhase,
    pub(crate) decision: &'a str,
    pub(crate) eligible: bool,
    pub(crate) rollback_performed: bool,
    pub(crate) reason: &'a str,
}
