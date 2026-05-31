use std::time::Duration;

use super::{model::LiveExperiment, scoring::experiment_data_quality};
use crate::{
    actions::SafetyClass,
    autotune::{
        comparison::ExperimentResult,
        controller::ControllerRuntimeState,
        decision::AutotuneDecision,
        experiment::{ExperimentId, WindowScore},
        kept::ActiveProfileState,
        objective::{ObjectiveComparisonInput, compare_for_objective},
        observation::AutotuneObservation,
        planning::candidate::CandidateAction,
        washout::WashoutWindowConfig,
    },
    daemon::{
        DaemonPolicy,
        policy::DaemonMode,
        state::{DaemonExperimentState, DaemonRollbackState},
    },
};

#[derive(Debug, Default)]
pub struct LiveExperimentManager {
    pub(crate) current: Option<LiveExperiment>,
}

#[doc(hidden)]
pub(crate) struct LiveExperimentRuntimeState<'a> {
    pub(crate) controller_state: &'a mut ControllerRuntimeState,
    pub(crate) active_profile_state: &'a mut ActiveProfileState,
}

impl LiveExperimentManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_active_experiment(&self) -> bool {
        self.current.is_some()
    }

    pub fn current_experiment(&self) -> Option<&LiveExperiment> {
        self.current.as_ref()
    }

    pub fn abandon_current_for_external_resync(&mut self) -> Option<LiveExperiment> {
        self.current.take()
    }

    pub fn current_experiment_id(&self) -> Option<ExperimentId> {
        self.current
            .as_ref()
            .map(|experiment| experiment.experiment_id.clone())
    }

    #[cfg(test)]
    pub fn set_current_for_tests(&mut self, experiment: LiveExperiment) {
        self.current = Some(experiment);
    }

    pub fn daemon_experiment_state(&self) -> Option<DaemonExperimentState> {
        self.current
            .as_ref()
            .map(|experiment| DaemonExperimentState {
                experiment_id: experiment.experiment_id.clone(),
                action_id: experiment.action_id(),
                candidate_name: Some(experiment.candidate_name().to_owned()),
                mode: experiment.mode,
                safety_class: experiment.safety_class.clone(),
                started_unix_nanos: Some(experiment.applied_unix_nanos),
            })
    }

    pub fn daemon_rollback_state(
        &self,
        manual_restore_command: &'static str,
    ) -> Option<DaemonRollbackState> {
        self.current.as_ref().map(|experiment| DaemonRollbackState {
            action_id: experiment.action_id(),
            mode: experiment.mode,
            safety_class: experiment.safety_class.clone(),
            rollback_available: true,
            token: Some(experiment.rollback.clone()),
            manual_restore_command: Some(manual_restore_command.to_owned()),
        })
    }

    pub fn active_window_decision(
        &self,
        observation: &AutotuneObservation,
    ) -> Option<AutotuneDecision> {
        let experiment = self.current.as_ref()?;

        if observation.now_unix_nanos < experiment.washout_until_unix_nanos {
            return Some(AutotuneDecision::Noop {
                reason: format!(
                    "candidate '{}' washout window is still stabilizing",
                    experiment.candidate_name()
                ),
            });
        }

        if observation.now_unix_nanos < experiment.measure_until_unix_nanos {
            return Some(AutotuneDecision::Noop {
                reason: format!(
                    "candidate '{}' measurement window is still collecting",
                    experiment.candidate_name()
                ),
            });
        }

        None
    }

    pub fn validate_start_candidate(
        mode: DaemonMode,
        policy: &DaemonPolicy,
        candidate: &CandidateAction,
    ) -> anyhow::Result<bool> {
        match mode {
            DaemonMode::ApplyLowRisk => {
                if candidate.safety_class() != SafetyClass::ReversibleLowRisk {
                    anyhow::bail!(
                        "live apply-low-risk rejected non-low-risk candidate {} safety={:?}",
                        candidate.profile_name(),
                        candidate.safety_class()
                    );
                }
                Ok(true)
            }
            DaemonMode::ApplyMediumRisk => {
                if !policy.allow_medium_risk_apply {
                    anyhow::bail!("live apply-medium-risk requires explicit medium-risk unlock");
                }
                if candidate.safety_class() > SafetyClass::ReversibleMediumRisk {
                    anyhow::bail!(
                        "live apply-medium-risk rejected non-medium-risk candidate {} safety={:?}",
                        candidate.profile_name(),
                        candidate.safety_class()
                    );
                }
                if candidate.is_high_risk_system_adjacent() {
                    anyhow::bail!(
                        "live apply-medium-risk rejected manual-only high-risk candidate {} action_kind={}",
                        candidate.profile_name(),
                        candidate.action_kind()
                    );
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub fn deadlines_from_now(
        simulate_action_effects: bool,
        washout: &WashoutWindowConfig,
        candidate_window_seconds: u64,
        applied_unix_nanos: u128,
    ) -> (u128, u128) {
        if simulate_action_effects {
            return (applied_unix_nanos, applied_unix_nanos);
        }

        let washout_until_unix_nanos =
            applied_unix_nanos.saturating_add(washout.washout_duration().as_nanos());
        let measure_until_unix_nanos = washout_until_unix_nanos
            .saturating_add(Duration::from_secs(candidate_window_seconds).as_nanos());

        (washout_until_unix_nanos, measure_until_unix_nanos)
    }

    pub fn compare_keep_result(
        experiment: &LiveExperiment,
        candidate_score: &WindowScore,
        observation: &AutotuneObservation,
    ) -> ExperimentResult {
        compare_for_objective(ObjectiveComparisonInput {
            objective: experiment.candidate.objective(),
            baseline: &experiment.baseline_score,
            candidate: candidate_score,
            baseline_signals: &experiment.baseline_signals,
            candidate_signals: &observation.objective_signals,
            data_quality: experiment_data_quality(&observation.data_quality),
            target_disappeared: !observation.target_present,
        })
    }
}
