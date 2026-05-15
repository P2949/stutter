use std::time::Duration;

use crate::{
    actions::{RollbackToken, SafetyClass},
    autotune::{
        candidate::CandidateAction,
        comparison::{ExperimentDataQuality, ExperimentResult},
        experiment::{ExperimentId, WindowScore},
        objective::{ObjectiveComparisonInput, ObjectiveSignals, compare_for_objective},
        observation::AutotuneObservation,
        washout::WashoutWindowConfig,
    },
    daemon::{DaemonMode, DaemonPolicy},
};

#[derive(Clone, Debug)]
pub struct LiveLowRiskExperiment {
    pub experiment_id: ExperimentId,
    pub candidate: CandidateAction,
    pub baseline_score: WindowScore,
    pub applied_unix_nanos: u128,
    pub washout_until_unix_nanos: u128,
    pub measure_until_unix_nanos: u128,
    pub rollback: RollbackToken,
}

impl LiveLowRiskExperiment {
    pub fn candidate_name(&self) -> &str {
        self.candidate.profile_name()
    }

    pub fn action_id(&self) -> String {
        self.candidate.action_id().0
    }
}

pub struct LiveExperimentManager;

impl LiveExperimentManager {
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
        experiment: &LiveLowRiskExperiment,
        candidate_score: &WindowScore,
        observation: &AutotuneObservation,
    ) -> ExperimentResult {
        let baseline_signals = ObjectiveSignals::from_window_score(&experiment.baseline_score);
        let candidate_signals = ObjectiveSignals::from_window_score(candidate_score);

        compare_for_objective(ObjectiveComparisonInput {
            objective: experiment.candidate.objective(),
            baseline: &experiment.baseline_score,
            candidate: candidate_score,
            baseline_signals: &baseline_signals,
            candidate_signals: &candidate_signals,
            data_quality: experiment_data_quality(&observation.data_quality),
            target_disappeared: !observation.target_present,
        })
    }
}

fn experiment_data_quality(
    quality: &crate::autotune::quality::OnlineDataQuality,
) -> ExperimentDataQuality {
    match quality {
        crate::autotune::quality::OnlineDataQuality::High => ExperimentDataQuality::High,
        crate::autotune::quality::OnlineDataQuality::Medium { .. } => ExperimentDataQuality::Medium,
        crate::autotune::quality::OnlineDataQuality::Low { .. } => ExperimentDataQuality::Low,
    }
}
