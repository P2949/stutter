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
    pub baseline_signals: ObjectiveSignals,
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

fn experiment_data_quality(
    quality: &crate::autotune::quality::OnlineDataQuality,
) -> ExperimentDataQuality {
    match quality {
        crate::autotune::quality::OnlineDataQuality::High => ExperimentDataQuality::High,
        crate::autotune::quality::OnlineDataQuality::Medium { .. } => ExperimentDataQuality::Medium,
        crate::autotune::quality::OnlineDataQuality::Low { .. } => ExperimentDataQuality::Low,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::{
            RollbackToken, TaskIdentity,
            nice::{NiceAction, NicePolicy},
        },
        autotune::{
            candidate::NiceActionPlan, comparison::ExperimentResult, objective::ObjectiveKind,
        },
        scorer::StutterScore,
    };

    fn score(total: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 1,
            finished_unix_nanos: 2,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total,
                frame_p99_ms: 12.0,
                frame_max_ms: 12.0,
                over_5ms: 1,
                ..StutterScore::default()
            },
        }
    }

    fn io_candidate() -> CandidateAction {
        CandidateAction::Nice {
            plan: NiceActionPlan {
                name: "io-objective-test".to_owned(),
                action: NiceAction {
                    targets: vec![TaskIdentity {
                        tid: 42,
                        process_pid: Some(42),
                        comm: Some("worker".to_owned()),
                        starttime_ticks: Some(1),
                    }],
                    nice: 5,
                    policy: NicePolicy::default(),
                },
                target_root_pid: Some(42),
                evidence: Vec::new(),
                objective: ObjectiveKind::IoLatency,
            },
        }
    }

    fn rollback() -> RollbackToken {
        RollbackToken::CpuAffinityRestoreFile {
            path: "/tmp/stutter-test-rollback.json".into(),
            affected_tasks: 1,
        }
    }

    #[test]
    fn compare_keep_result_rejects_io_candidate_when_live_io_signal_regresses() {
        let experiment = LiveLowRiskExperiment {
            experiment_id: ExperimentId::new("io-test"),
            candidate: io_candidate(),
            baseline_score: score(1_000),
            baseline_signals: ObjectiveSignals {
                block_io_overlap_count: Some(1),
                block_io_worst_latency_ns: Some(2_000_000),
                ..ObjectiveSignals::from_window_score(&score(1_000))
            },
            applied_unix_nanos: 10,
            washout_until_unix_nanos: 20,
            measure_until_unix_nanos: 30,
            rollback: rollback(),
        };
        let candidate_score = score(800);
        let observation = AutotuneObservation {
            target_present: true,
            data_quality: crate::autotune::quality::OnlineDataQuality::High,
            objective_signals: ObjectiveSignals {
                block_io_overlap_count: Some(2),
                block_io_worst_latency_ns: Some(3_000_000),
                ..ObjectiveSignals::from_window_score(&candidate_score)
            },
            ..AutotuneObservation::default()
        };

        let result =
            LiveExperimentManager::compare_keep_result(&experiment, &candidate_score, &observation);

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }
}
