#[cfg(test)]
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::planning::candidate::CandidateAction;
use crate::{actions::RollbackToken, scorer::StutterScore};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExperimentId(pub String);

impl ExperimentId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WindowScore {
    pub started_unix_nanos: u128,
    pub finished_unix_nanos: u128,
    pub interval_count: usize,
    pub scored_samples: u64,
    pub scored_task_count: usize,
    pub score: StutterScore,
}

impl WindowScore {
    pub fn diagnostic_score_total(&self) -> u64 {
        self.score.total
    }

    pub fn score_per_sample(&self) -> Option<f64> {
        per_sample(self.score.total, self.scored_samples)
    }

    pub fn over_1ms_per_sample(&self) -> Option<f64> {
        per_sample(self.score.over_1ms, self.scored_samples)
    }

    pub fn over_2ms_per_sample(&self) -> Option<f64> {
        per_sample(self.score.over_2ms, self.scored_samples)
    }

    pub fn over_5ms_per_sample(&self) -> Option<f64> {
        per_sample(self.score.over_5ms, self.scored_samples)
    }

    pub fn duration_unix_nanos(&self) -> u128 {
        self.finished_unix_nanos
            .saturating_sub(self.started_unix_nanos)
    }

    pub fn duration_seconds(&self) -> Option<f64> {
        let nanos = self.duration_unix_nanos();
        if nanos == 0 {
            None
        } else {
            Some(nanos as f64 / 1_000_000_000.0)
        }
    }

    pub fn score_per_second(&self) -> Option<f64> {
        self.duration_seconds()
            .map(|duration_seconds| self.score.total as f64 / duration_seconds)
    }
}

fn per_sample(value: u64, scored_samples: u64) -> Option<f64> {
    if scored_samples == 0 {
        None
    } else {
        Some(value as f64 / scored_samples as f64)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExperimentPhase {
    BaselineCollecting,
    CandidateApplying,
    CandidateWashout,
    CandidateMeasuring,
    CandidateKeeping,
    CandidateReverting,
    Cooldown,
}

#[derive(Clone, Debug)]
pub struct ActiveExperiment {
    pub id: ExperimentId,
    pub candidate: CandidateAction,
    pub baseline_score: WindowScore,
    pub candidate_score: Option<WindowScore>,
    pub started_unix_nanos: u128,
    pub applied_unix_nanos: Option<u128>,
    pub measure_started_unix_nanos: Option<u128>,
    pub rollback: Option<RollbackToken>,
    pub phase: ExperimentPhase,
}

impl ActiveExperiment {
    pub fn new(
        id: ExperimentId,
        candidate: CandidateAction,
        baseline_score: WindowScore,
        started_unix_nanos: u128,
    ) -> Self {
        Self {
            id,
            candidate,
            baseline_score,
            candidate_score: None,
            started_unix_nanos,
            applied_unix_nanos: None,
            measure_started_unix_nanos: None,
            rollback: None,
            phase: ExperimentPhase::BaselineCollecting,
        }
    }

    pub fn mark_candidate_applying(&mut self) {
        self.phase = ExperimentPhase::CandidateApplying;
    }

    pub fn mark_candidate_applied(&mut self, applied_unix_nanos: u128, rollback: RollbackToken) {
        self.applied_unix_nanos = Some(applied_unix_nanos);
        self.rollback = Some(rollback);
        self.phase = ExperimentPhase::CandidateWashout;
    }

    pub fn mark_candidate_measuring(&mut self, measure_started_unix_nanos: u128) {
        self.measure_started_unix_nanos = Some(measure_started_unix_nanos);
        self.phase = ExperimentPhase::CandidateMeasuring;
    }

    pub fn set_candidate_score(&mut self, candidate_score: WindowScore) {
        self.candidate_score = Some(candidate_score);
    }

    pub fn mark_candidate_kept(&mut self) {
        self.phase = ExperimentPhase::CandidateKeeping;
    }

    pub fn mark_candidate_kept_and_enter_cooldown(&mut self) {
        self.phase = ExperimentPhase::Cooldown;
    }

    pub fn mark_candidate_keeping(&mut self) {
        self.phase = ExperimentPhase::CandidateKeeping;
    }

    pub fn mark_candidate_reverting(&mut self) {
        self.phase = ExperimentPhase::CandidateReverting;
    }

    pub fn mark_cooldown(&mut self) {
        self.phase = ExperimentPhase::Cooldown;
    }

    pub fn take_rollback(&mut self) -> Option<RollbackToken> {
        self.rollback.take()
    }

    pub fn has_rollback(&self) -> bool {
        self.rollback.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        affinity::CpuMask,
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
    };

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    fn candidate() -> CandidateAction {
        CandidateAction::cpu_affinity_profile(profile("game-main"), 1234)
    }

    fn window_score(total: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 5,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total,
                ..StutterScore::default()
            },
        }
    }

    fn rollback_token() -> RollbackToken {
        RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/stutter-restore.json"),
            affected_tasks: 31,
        }
    }

    #[test]
    fn new_active_experiment_starts_in_baseline_collecting() {
        let experiment = ActiveExperiment::new(
            ExperimentId::new("experiment-1"),
            candidate(),
            window_score(143),
            1_000,
        );

        assert_eq!(experiment.id.as_str(), "experiment-1");
        assert_eq!(experiment.phase, ExperimentPhase::BaselineCollecting);
        assert_eq!(experiment.baseline_score.diagnostic_score_total(), 143);
        assert!(experiment.candidate_score.is_none());
        assert!(experiment.applied_unix_nanos.is_none());
        assert!(experiment.measure_started_unix_nanos.is_none());
        assert!(!experiment.has_rollback());
    }

    #[test]
    fn active_experiment_tracks_apply_measure_keep_lifecycle() {
        let mut experiment = ActiveExperiment::new(
            ExperimentId::new("experiment-1"),
            candidate(),
            window_score(143),
            1_000,
        );

        experiment.mark_candidate_applying();
        assert_eq!(experiment.phase, ExperimentPhase::CandidateApplying);

        experiment.mark_candidate_applied(1_100, rollback_token());
        assert_eq!(experiment.phase, ExperimentPhase::CandidateWashout);
        assert_eq!(experiment.applied_unix_nanos, Some(1_100));
        assert!(experiment.has_rollback());

        experiment.mark_candidate_measuring(1_200);
        assert_eq!(experiment.phase, ExperimentPhase::CandidateMeasuring);
        assert_eq!(experiment.measure_started_unix_nanos, Some(1_200));

        experiment.set_candidate_score(window_score(100));
        assert_eq!(
            experiment
                .candidate_score
                .as_ref()
                .map(WindowScore::diagnostic_score_total),
            Some(100)
        );

        experiment.mark_candidate_keeping();
        assert_eq!(experiment.phase, ExperimentPhase::CandidateKeeping);

        experiment.mark_cooldown();
        assert_eq!(experiment.phase, ExperimentPhase::Cooldown);
    }

    #[test]
    fn active_experiment_tracks_revert_lifecycle_and_takes_rollback_once() {
        let mut experiment = ActiveExperiment::new(
            ExperimentId::new("experiment-1"),
            candidate(),
            window_score(143),
            1_000,
        );

        experiment.mark_candidate_applied(1_100, rollback_token());
        experiment.mark_candidate_reverting();

        assert_eq!(experiment.phase, ExperimentPhase::CandidateReverting);
        assert!(experiment.has_rollback());

        let token = experiment.take_rollback();

        assert!(token.is_some());
        assert!(!experiment.has_rollback());
        assert!(experiment.take_rollback().is_none());
    }

    #[test]
    fn window_score_reports_duration_and_total() {
        let score = WindowScore {
            started_unix_nanos: 500,
            finished_unix_nanos: 750,
            interval_count: 2,
            scored_samples: 40,
            scored_task_count: 1,
            score: StutterScore {
                total: 99,
                ..StutterScore::default()
            },
        };

        assert_eq!(score.duration_unix_nanos(), 250);
        assert_eq!(score.diagnostic_score_total(), 99);
    }

    #[test]
    fn window_score_reports_per_sample_rates() {
        let mut score = window_score(200);
        score.scored_samples = 100;
        score.score.over_1ms = 25;
        score.score.over_2ms = 10;
        score.score.over_5ms = 4;

        assert_eq!(score.score_per_sample(), Some(2.0));
        assert_eq!(score.over_1ms_per_sample(), Some(0.25));
        assert_eq!(score.over_2ms_per_sample(), Some(0.1));
        assert_eq!(score.over_5ms_per_sample(), Some(0.04));
    }

    #[test]
    fn window_score_rates_are_missing_when_denominator_is_zero() {
        let mut score = window_score(200);
        score.scored_samples = 0;

        assert_eq!(score.score_per_sample(), None);
        assert_eq!(score.over_1ms_per_sample(), None);
        assert_eq!(score.over_2ms_per_sample(), None);
        assert_eq!(score.over_5ms_per_sample(), None);
    }

    #[test]
    fn window_score_reports_duration_seconds_and_score_rate() {
        let score = WindowScore {
            started_unix_nanos: 1_000_000_000,
            finished_unix_nanos: 3_500_000_000,
            interval_count: 2,
            scored_samples: 40,
            scored_task_count: 1,
            score: StutterScore {
                total: 250,
                ..StutterScore::default()
            },
        };

        assert_eq!(score.duration_seconds(), Some(2.5));
        assert_eq!(score.score_per_second(), Some(100.0));
    }

    #[test]
    fn window_score_duration_rate_is_missing_for_zero_duration() {
        let score = WindowScore {
            started_unix_nanos: 500,
            finished_unix_nanos: 500,
            interval_count: 2,
            scored_samples: 40,
            scored_task_count: 1,
            score: StutterScore {
                total: 250,
                ..StutterScore::default()
            },
        };

        assert_eq!(score.duration_seconds(), None);
        assert_eq!(score.score_per_second(), None);
    }
}
