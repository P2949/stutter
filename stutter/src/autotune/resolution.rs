use super::{
    comparison::ExperimentResult,
    experiment::{ActiveExperiment, ExperimentId, ExperimentPhase},
    kept::{ActiveProfileState, KeptCandidateState},
};
use crate::actions::{RollbackToken, TuningAction};

#[derive(Clone, Debug, PartialEq)]
pub enum ExperimentResolution {
    Kept {
        experiment_id: ExperimentId,
        reason: String,
    },
    Reverted {
        experiment_id: ExperimentId,
        reason: String,
    },
}

impl ExperimentResolution {
    pub fn entered_cooldown(&self) -> bool {
        true
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Kept { reason, .. } | Self::Reverted { reason, .. } => reason,
        }
    }

    pub fn experiment_id(&self) -> &ExperimentId {
        match self {
            Self::Kept { experiment_id, .. } | Self::Reverted { experiment_id, .. } => {
                experiment_id
            }
        }
    }
}

pub trait ExperimentRollbackExecutor {
    fn rollback(&mut self, token: &RollbackToken) -> anyhow::Result<()>;
}

pub struct ActionRollbackExecutor<'a, A: TuningAction + ?Sized> {
    action: &'a A,
}

impl<'a, A: TuningAction + ?Sized> ActionRollbackExecutor<'a, A> {
    pub fn new(action: &'a A) -> Self {
        Self { action }
    }
}

impl<A: TuningAction + ?Sized> ExperimentRollbackExecutor for ActionRollbackExecutor<'_, A> {
    fn rollback(&mut self, token: &RollbackToken) -> anyhow::Result<()> {
        self.action.rollback(token)
    }
}

pub fn resolve_experiment_with_action<A: TuningAction + ?Sized>(
    experiment: &mut ActiveExperiment,
    result: &ExperimentResult,
    action: &A,
) -> anyhow::Result<ExperimentResolution> {
    let mut executor = ActionRollbackExecutor::new(action);
    resolve_experiment(experiment, result, &mut executor)
}

pub fn resolve_experiment<E: ExperimentRollbackExecutor + ?Sized>(
    experiment: &mut ActiveExperiment,
    result: &ExperimentResult,
    rollback_executor: &mut E,
) -> anyhow::Result<ExperimentResolution> {
    let mut active_profile_state = ActiveProfileState::default();
    resolve_experiment_with_active_profile_state(
        experiment,
        result,
        rollback_executor,
        &mut active_profile_state,
        crate::audit::unix_nanos_now(),
    )
}

pub fn resolve_experiment_with_active_profile_state<E: ExperimentRollbackExecutor + ?Sized>(
    experiment: &mut ActiveExperiment,
    result: &ExperimentResult,
    rollback_executor: &mut E,
    active_profile_state: &mut ActiveProfileState,
    now_unix_nanos: u128,
) -> anyhow::Result<ExperimentResolution> {
    match result {
        ExperimentResult::Improved {
            improvement_percent,
        } => keep_improved_experiment(
            experiment,
            active_profile_state,
            result.clone(),
            *improvement_percent,
            now_unix_nanos,
        ),
        ExperimentResult::Regressed { regression_percent } => rollback_and_enter_cooldown(
            experiment,
            rollback_executor,
            format!(
                "candidate regressed by {:.2}%; reverted and entered cooldown",
                regression_percent
            ),
        ),
        ExperimentResult::Inconclusive { reason } => rollback_and_enter_cooldown(
            experiment,
            rollback_executor,
            format!("candidate result inconclusive: {reason}; reverted and entered cooldown"),
        ),
        ExperimentResult::Invalid { reason } => rollback_and_enter_cooldown(
            experiment,
            rollback_executor,
            format!("candidate data invalid: {reason}; reverted and entered cooldown"),
        ),
    }
}

fn keep_improved_experiment(
    experiment: &mut ActiveExperiment,
    active_profile_state: &mut ActiveProfileState,
    result: ExperimentResult,
    improvement_percent: f64,
    now_unix_nanos: u128,
) -> anyhow::Result<ExperimentResolution> {
    experiment.phase = ExperimentPhase::CandidateKeeping;

    let candidate_score = experiment
        .candidate_score
        .clone()
        .ok_or_else(|| anyhow::anyhow!("cannot keep experiment without candidate score"))?;

    let rollback = experiment
        .rollback
        .clone()
        .ok_or_else(|| anyhow::anyhow!("cannot keep experiment without rollback token"))?;

    let reason = format!(
        "candidate improved by {:.2}%; kept as current active profile and entered cooldown",
        improvement_percent
    );

    let kept = KeptCandidateState::new(
        experiment.id.clone(),
        experiment.candidate.clone(),
        experiment.baseline_score.clone(),
        candidate_score,
        rollback,
        now_unix_nanos,
        reason.clone(),
    );

    active_profile_state.record_kept_candidate(kept, result)?;
    experiment.phase = ExperimentPhase::Cooldown;

    Ok(ExperimentResolution::Kept {
        experiment_id: experiment.id.clone(),
        reason,
    })
}

fn rollback_and_enter_cooldown<E: ExperimentRollbackExecutor + ?Sized>(
    experiment: &mut ActiveExperiment,
    rollback_executor: &mut E,
    reason: String,
) -> anyhow::Result<ExperimentResolution> {
    experiment.phase = ExperimentPhase::CandidateReverting;

    let token = experiment
        .take_rollback()
        .ok_or_else(|| anyhow::anyhow!("cannot revert experiment without rollback token"))?;

    rollback_executor.rollback(&token)?;
    experiment.phase = ExperimentPhase::Cooldown;

    Ok(ExperimentResolution::Reverted {
        experiment_id: experiment.id.clone(),
        reason,
    })
}

pub fn should_keep_candidate(result: &ExperimentResult) -> bool {
    matches!(result, ExperimentResult::Improved { .. })
}

pub fn should_rollback_candidate(result: &ExperimentResult) -> bool {
    matches!(
        result,
        ExperimentResult::Regressed { .. }
            | ExperimentResult::Inconclusive { .. }
            | ExperimentResult::Invalid { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::RollbackToken,
        affinity::CpuMask,
        autotune::{candidate::CandidateAction, experiment::WindowScore, kept::ActiveProfileState},
        process_tree::TaskClass,
        profiles::{Profile, ProfileRule},
        scorer::StutterScore,
    };

    #[derive(Default)]
    struct FakeRollbackExecutor {
        calls: usize,
        fail: bool,
        tokens: Vec<RollbackToken>,
    }

    impl ExperimentRollbackExecutor for FakeRollbackExecutor {
        fn rollback(&mut self, token: &RollbackToken) -> anyhow::Result<()> {
            self.calls += 1;
            self.tokens.push(token.clone());

            if self.fail {
                anyhow::bail!("intentional rollback failure");
            }

            Ok(())
        }
    }

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

    fn active_experiment() -> ActiveExperiment {
        let mut experiment = ActiveExperiment::new(
            ExperimentId::new("experiment-1"),
            candidate(),
            window_score(1_000),
            1_000,
        );
        experiment.mark_candidate_applied(1_100, rollback_token());
        experiment.mark_candidate_measuring(1_200);
        experiment.set_candidate_score(window_score(875));
        experiment
    }

    #[test]
    fn improved_keeps_candidate_enters_cooldown_and_preserves_rollback_in_active_profile_state() {
        let mut experiment = active_experiment();
        let mut rollback_executor = FakeRollbackExecutor::default();
        let mut active_profile_state = ActiveProfileState::default();

        let resolution = resolve_experiment_with_active_profile_state(
            &mut experiment,
            &ExperimentResult::Improved {
                improvement_percent: 12.5,
            },
            &mut rollback_executor,
            &mut active_profile_state,
            9_999,
        )
        .unwrap();

        match resolution {
            ExperimentResolution::Kept {
                experiment_id,
                reason,
            } => {
                assert_eq!(experiment_id.as_str(), "experiment-1");
                assert!(reason.contains("improved by 12.50%"));
                assert!(reason.contains("kept as current active profile"));
                assert!(reason.contains("entered cooldown"));
            }
            other => panic!("expected kept resolution, got {other:?}"),
        }

        assert_eq!(rollback_executor.calls, 0);
        assert_eq!(experiment.phase, ExperimentPhase::Cooldown);
        assert!(experiment.has_rollback());
        assert_eq!(
            active_profile_state.current_profile_name(),
            Some("game-main")
        );
        assert_eq!(
            active_profile_state
                .current_rollback()
                .map(|rollback| rollback.affected_tasks()),
            Some(31)
        );
        assert_eq!(
            active_profile_state
                .comparison_baseline_for_next_experiment()
                .map(|score| score.score.total),
            Some(875)
        );
        assert_eq!(active_profile_state.history.len(), 1);
        assert_eq!(active_profile_state.history[0].profile_name, "game-main");
        assert!(active_profile_state.history[0].rollback_available);
    }

    #[test]
    fn regressed_rolls_back_and_enters_cooldown() {
        let mut experiment = active_experiment();
        let mut rollback_executor = FakeRollbackExecutor::default();

        let resolution = resolve_experiment(
            &mut experiment,
            &ExperimentResult::Regressed {
                regression_percent: 7.5,
            },
            &mut rollback_executor,
        )
        .unwrap();

        match resolution {
            ExperimentResolution::Reverted {
                experiment_id,
                reason,
            } => {
                assert_eq!(experiment_id.as_str(), "experiment-1");
                assert!(reason.contains("regressed by 7.50%"));
                assert!(reason.contains("reverted and entered cooldown"));
            }
            other => panic!("expected reverted resolution, got {other:?}"),
        }

        assert_eq!(rollback_executor.calls, 1);
        match &rollback_executor.tokens[0] {
            RollbackToken::CpuAffinityRestoreFile { affected_tasks, .. } => {
                assert_eq!(*affected_tasks, 31);
            }
            _ => panic!("expected CpuAffinityRestoreFile token"),
        }
        assert_eq!(experiment.phase, ExperimentPhase::Cooldown);
        assert!(!experiment.has_rollback());
    }

    #[test]
    fn inconclusive_rolls_back_and_enters_cooldown() {
        let mut experiment = active_experiment();
        let mut rollback_executor = FakeRollbackExecutor::default();

        let resolution = resolve_experiment(
            &mut experiment,
            &ExperimentResult::Inconclusive {
                reason: "candidate did not meet conservative thresholds".to_owned(),
            },
            &mut rollback_executor,
        )
        .unwrap();

        assert!(matches!(resolution, ExperimentResolution::Reverted { .. }));
        assert!(resolution.reason().contains("inconclusive"));
        assert_eq!(rollback_executor.calls, 1);
        assert_eq!(experiment.phase, ExperimentPhase::Cooldown);
        assert!(!experiment.has_rollback());
    }

    #[test]
    fn invalid_data_rolls_back_and_enters_cooldown() {
        let mut experiment = active_experiment();
        let mut rollback_executor = FakeRollbackExecutor::default();

        let resolution = resolve_experiment(
            &mut experiment,
            &ExperimentResult::Invalid {
                reason: "candidate window has zero scored samples".to_owned(),
            },
            &mut rollback_executor,
        )
        .unwrap();

        assert!(matches!(resolution, ExperimentResolution::Reverted { .. }));
        assert!(resolution.reason().contains("data invalid"));
        assert_eq!(rollback_executor.calls, 1);
        assert_eq!(experiment.phase, ExperimentPhase::Cooldown);
        assert!(!experiment.has_rollback());
    }

    #[test]
    fn rollback_failure_leaves_experiment_in_reverting_phase_and_returns_error() {
        let mut experiment = active_experiment();
        let mut rollback_executor = FakeRollbackExecutor {
            fail: true,
            ..FakeRollbackExecutor::default()
        };

        let err = resolve_experiment(
            &mut experiment,
            &ExperimentResult::Regressed {
                regression_percent: 7.5,
            },
            &mut rollback_executor,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("intentional rollback failure"));
        assert_eq!(rollback_executor.calls, 1);
        assert_eq!(experiment.phase, ExperimentPhase::CandidateReverting);
        assert!(!experiment.has_rollback());
    }

    #[test]
    fn missing_rollback_token_blocks_revert() {
        let mut experiment = active_experiment();
        let _ = experiment.take_rollback();
        let mut rollback_executor = FakeRollbackExecutor::default();

        let err = resolve_experiment(
            &mut experiment,
            &ExperimentResult::Inconclusive {
                reason: "not enough separation".to_owned(),
            },
            &mut rollback_executor,
        )
        .unwrap_err()
        .to_string();

        assert_eq!(err, "cannot revert experiment without rollback token");
        assert_eq!(rollback_executor.calls, 0);
        assert_eq!(experiment.phase, ExperimentPhase::CandidateReverting);
    }

    #[test]
    fn improved_without_candidate_score_is_invalid_to_keep() {
        let mut experiment = active_experiment();
        experiment.candidate_score = None;
        let mut rollback_executor = FakeRollbackExecutor::default();
        let mut active_profile_state = ActiveProfileState::default();

        let err = resolve_experiment_with_active_profile_state(
            &mut experiment,
            &ExperimentResult::Improved {
                improvement_percent: 12.5,
            },
            &mut rollback_executor,
            &mut active_profile_state,
            9_999,
        )
        .unwrap_err()
        .to_string();

        assert_eq!(err, "cannot keep experiment without candidate score");
        assert_eq!(rollback_executor.calls, 0);
        assert!(active_profile_state.kept_actions.is_empty());
        assert!(experiment.has_rollback());
    }

    #[test]
    fn improved_without_rollback_token_is_invalid_to_keep() {
        let mut experiment = active_experiment();
        let _ = experiment.take_rollback();
        let mut rollback_executor = FakeRollbackExecutor::default();
        let mut active_profile_state = ActiveProfileState::default();

        let err = resolve_experiment_with_active_profile_state(
            &mut experiment,
            &ExperimentResult::Improved {
                improvement_percent: 12.5,
            },
            &mut rollback_executor,
            &mut active_profile_state,
            9_999,
        )
        .unwrap_err()
        .to_string();

        assert_eq!(err, "cannot keep experiment without rollback token");
        assert_eq!(rollback_executor.calls, 0);
        assert!(active_profile_state.kept_actions.is_empty());
    }

    #[test]
    fn helper_predicates_match_v1_policy() {
        assert!(should_keep_candidate(&ExperimentResult::Improved {
            improvement_percent: 12.5,
        }));
        assert!(!should_rollback_candidate(&ExperimentResult::Improved {
            improvement_percent: 12.5,
        }));

        assert!(should_rollback_candidate(&ExperimentResult::Regressed {
            regression_percent: 7.5,
        }));
        assert!(should_rollback_candidate(&ExperimentResult::Inconclusive {
            reason: "not clear".to_owned(),
        }));
        assert!(should_rollback_candidate(&ExperimentResult::Invalid {
            reason: "bad data".to_owned(),
        }));
    }
}
