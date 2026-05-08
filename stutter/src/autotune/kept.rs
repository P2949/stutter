#![allow(dead_code)]

use super::{
    candidate::CandidateAction,
    comparison::ExperimentResult,
    experiment::{ExperimentId, WindowScore},
};
use crate::actions::RollbackToken;

#[derive(Clone, Debug)]
pub struct KeptCandidateState {
    pub experiment_id: ExperimentId,
    pub current_profile_name: String,
    pub candidate: CandidateAction,
    pub baseline_score: WindowScore,
    pub candidate_score: WindowScore,
    pub rollback: RollbackToken,
    pub kept_unix_nanos: u128,
    pub reason: String,
}

impl KeptCandidateState {
    pub fn new(
        experiment_id: ExperimentId,
        candidate: CandidateAction,
        baseline_score: WindowScore,
        candidate_score: WindowScore,
        rollback: RollbackToken,
        kept_unix_nanos: u128,
        reason: impl Into<String>,
    ) -> Self {
        let current_profile_name = candidate.profile_name().to_owned();

        Self {
            experiment_id,
            current_profile_name,
            candidate,
            baseline_score,
            candidate_score,
            rollback,
            kept_unix_nanos,
            reason: reason.into(),
        }
    }

    pub fn current_profile_name(&self) -> &str {
        &self.current_profile_name
    }

    pub fn rollback_token(&self) -> &RollbackToken {
        &self.rollback
    }

    pub fn comparison_baseline_for_next_experiment(&self) -> &WindowScore {
        &self.candidate_score
    }
}

#[derive(Clone, Debug)]
pub struct KeptCandidateHistoryEntry {
    pub experiment_id: ExperimentId,
    pub profile_name: String,
    pub kept_unix_nanos: u128,
    pub result: ExperimentResult,
    pub reason: String,
    pub baseline_score_total: u64,
    pub candidate_score_total: u64,
    pub rollback_available: bool,
}

impl KeptCandidateHistoryEntry {
    pub fn from_kept_state(state: &KeptCandidateState, result: ExperimentResult) -> Self {
        Self {
            experiment_id: state.experiment_id.clone(),
            profile_name: state.current_profile_name.clone(),
            kept_unix_nanos: state.kept_unix_nanos,
            result,
            reason: state.reason.clone(),
            baseline_score_total: state.baseline_score.score.total,
            candidate_score_total: state.candidate_score.score.total,
            rollback_available: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ActiveProfileState {
    pub current: Option<KeptCandidateState>,
    pub history: Vec<KeptCandidateHistoryEntry>,
}

impl ActiveProfileState {
    pub fn current_profile_name(&self) -> Option<&str> {
        self.current
            .as_ref()
            .map(KeptCandidateState::current_profile_name)
    }

    pub fn current_rollback(&self) -> Option<&RollbackToken> {
        self.current
            .as_ref()
            .map(KeptCandidateState::rollback_token)
    }

    pub fn comparison_baseline_for_next_experiment(&self) -> Option<&WindowScore> {
        self.current
            .as_ref()
            .map(KeptCandidateState::comparison_baseline_for_next_experiment)
    }

    pub fn record_kept_candidate(&mut self, kept: KeptCandidateState, result: ExperimentResult) {
        let history_entry = KeptCandidateHistoryEntry::from_kept_state(&kept, result);
        self.current = Some(kept);
        self.history.push(history_entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::RollbackToken,
        affinity::CpuMask,
        autotune::candidate::CandidateAction,
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
        scorer::StutterScore,
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

    fn candidate(name: &str) -> CandidateAction {
        CandidateAction::cpu_affinity_profile(profile(name), 1234)
    }

    fn window_score(total: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
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
            path: std::path::PathBuf::from("/tmp/stutter-restore.json"),
            affected_tasks: 31,
        }
    }

    #[test]
    fn kept_candidate_state_sets_current_profile_to_candidate() {
        let kept = KeptCandidateState::new(
            ExperimentId::new("experiment-1"),
            candidate("game-main"),
            window_score(1_000),
            window_score(850),
            rollback_token(),
            123,
            "candidate improved by 15.00%; kept and entered cooldown",
        );

        assert_eq!(kept.current_profile_name(), "game-main");
        assert_eq!(kept.rollback_token().affected_tasks(), 31);
        assert_eq!(
            kept.comparison_baseline_for_next_experiment().score.total,
            850
        );
    }

    #[test]
    fn active_profile_state_records_history_and_keeps_rollback_available() {
        let kept = KeptCandidateState::new(
            ExperimentId::new("experiment-1"),
            candidate("game-main"),
            window_score(1_000),
            window_score(850),
            rollback_token(),
            123,
            "candidate improved by 15.00%; kept and entered cooldown",
        );
        let mut state = ActiveProfileState::default();

        state.record_kept_candidate(
            kept,
            ExperimentResult::Improved {
                improvement_percent: 15.0,
            },
        );

        assert_eq!(state.current_profile_name(), Some("game-main"));
        assert_eq!(
            state
                .comparison_baseline_for_next_experiment()
                .map(|score| score.score.total),
            Some(850)
        );
        assert_eq!(
            state
                .current_rollback()
                .map(|rollback| rollback.affected_tasks()),
            Some(31)
        );
        assert_eq!(state.history.len(), 1);
        assert_eq!(state.history[0].profile_name, "game-main");
        assert_eq!(state.history[0].baseline_score_total, 1_000);
        assert_eq!(state.history[0].candidate_score_total, 850);
        assert!(state.history[0].rollback_available);
        assert!(
            state.history[0]
                .reason
                .contains("kept and entered cooldown")
        );
    }

    #[test]
    fn later_kept_candidate_becomes_new_current_profile() {
        let first = KeptCandidateState::new(
            ExperimentId::new("experiment-1"),
            candidate("game-main"),
            window_score(1_000),
            window_score(850),
            rollback_token(),
            123,
            "first kept",
        );
        let second = KeptCandidateState::new(
            ExperimentId::new("experiment-2"),
            candidate("game-main-v2"),
            window_score(850),
            window_score(700),
            rollback_token(),
            456,
            "second kept",
        );
        let mut state = ActiveProfileState::default();

        state.record_kept_candidate(
            first,
            ExperimentResult::Improved {
                improvement_percent: 15.0,
            },
        );
        state.record_kept_candidate(
            second,
            ExperimentResult::Improved {
                improvement_percent: 17.64,
            },
        );

        assert_eq!(state.current_profile_name(), Some("game-main-v2"));
        assert_eq!(
            state
                .comparison_baseline_for_next_experiment()
                .map(|score| score.score.total),
            Some(700)
        );
        assert_eq!(state.history.len(), 2);
        assert_eq!(state.history[0].profile_name, "game-main");
        assert_eq!(state.history[1].profile_name, "game-main-v2");
    }
}
