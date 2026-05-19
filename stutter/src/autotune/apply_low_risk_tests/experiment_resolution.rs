//! Experiment-resolution tests extracted from `autotune::apply_low_risk`.
//!
//! Owns low-risk experiment resolution, comparison thresholds, measurement readiness, baseline readiness, and washout defaults tests.
//! Does not own policy gates, target selection, audit/journal behavior, rollback orchestration, or production behavior.

#[cfg(test)]
mod tests {
    use super::super::super::*;

    #[test]
    fn low_risk_resolution_keeps_improved_candidate_as_current_profile() {
        use crate::{
            actions::RollbackToken,
            affinity::CpuMask,
            autotune::{
                candidate::CandidateAction,
                comparison::ExperimentResult,
                experiment::{ActiveExperiment, ExperimentId, ExperimentPhase, WindowScore},
                kept::ActiveProfileState,
                resolution::{ExperimentResolution, ExperimentRollbackExecutor},
            },
            process_tree::TaskClass,
            profiles::{Profile, ProfileRule},
            scorer::StutterScore,
        };

        struct FakeRollback {
            calls: usize,
        }

        impl ExperimentRollbackExecutor for FakeRollback {
            fn rollback(&mut self, _token: &RollbackToken) -> anyhow::Result<()> {
                self.calls += 1;
                Ok(())
            }
        }

        let profile = Profile {
            name: "game-main".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };
        let candidate = CandidateAction::cpu_affinity_profile(profile, 1234);
        let baseline = WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total: 1_000,
                ..StutterScore::default()
            },
        };
        let candidate_score = WindowScore {
            started_unix_nanos: 300,
            finished_unix_nanos: 400,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total: 875,
                ..StutterScore::default()
            },
        };
        let mut experiment = ActiveExperiment::new(
            ExperimentId::new("low-risk-test"),
            candidate,
            baseline,
            1_000,
        );
        experiment.mark_candidate_applied(
            1_100,
            RollbackToken::CpuAffinityRestoreFile {
                path: std::path::PathBuf::from("/tmp/stutter-restore.json"),
                affected_tasks: 31,
            },
        );
        experiment.set_candidate_score(candidate_score);

        let mut rollback = FakeRollback { calls: 0 };
        let mut active_profile_state = ActiveProfileState::default();
        let resolution = resolve_low_risk_experiment_with_active_profile_state(
            &mut experiment,
            &ExperimentResult::Improved {
                improvement_percent: 12.5,
            },
            &mut rollback,
            &mut active_profile_state,
            9_999,
        )
        .unwrap();

        assert!(matches!(resolution, ExperimentResolution::Kept { .. }));
        assert_eq!(rollback.calls, 0);
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
    }

    #[test]
    fn low_risk_resolution_reverts_inconclusive_result() {
        use crate::{
            actions::RollbackToken,
            affinity::CpuMask,
            autotune::{
                candidate::CandidateAction,
                comparison::ExperimentResult,
                experiment::{ActiveExperiment, ExperimentId, ExperimentPhase, WindowScore},
                resolution::{ExperimentResolution, ExperimentRollbackExecutor},
            },
            process_tree::TaskClass,
            profiles::{Profile, ProfileRule},
            scorer::StutterScore,
        };

        struct FakeRollback {
            calls: usize,
        }

        impl ExperimentRollbackExecutor for FakeRollback {
            fn rollback(&mut self, _token: &RollbackToken) -> anyhow::Result<()> {
                self.calls += 1;
                Ok(())
            }
        }

        let profile = Profile {
            name: "game-main".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };
        let candidate = CandidateAction::cpu_affinity_profile(profile, 1234);
        let score = WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total: 1_000,
                ..StutterScore::default()
            },
        };
        let mut experiment =
            ActiveExperiment::new(ExperimentId::new("low-risk-test"), candidate, score, 1_000);
        experiment.mark_candidate_applied(
            1_100,
            RollbackToken::CpuAffinityRestoreFile {
                path: std::path::PathBuf::from("/tmp/stutter-restore.json"),
                affected_tasks: 31,
            },
        );

        let mut rollback = FakeRollback { calls: 0 };
        let resolution = resolve_low_risk_experiment(
            &mut experiment,
            &ExperimentResult::Inconclusive {
                reason: "not enough improvement".to_owned(),
            },
            &mut rollback,
        )
        .unwrap();

        assert!(matches!(resolution, ExperimentResolution::Reverted { .. }));
        assert_eq!(rollback.calls, 1);
        assert_eq!(experiment.phase, ExperimentPhase::Cooldown);
        assert!(!experiment.has_rollback());
    }

    #[test]
    fn low_risk_experiment_comparison_uses_conservative_thresholds() {
        let baseline = crate::autotune::experiment::WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: crate::scorer::StutterScore {
                total: 1_000,
                over_5ms: 10,
                frame_p99_ms: 12.0,
                frame_max_ms: 12.0,
                ..crate::scorer::StutterScore::default()
            },
        };
        let candidate = crate::autotune::experiment::WindowScore {
            started_unix_nanos: 300,
            finished_unix_nanos: 400,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: crate::scorer::StutterScore {
                total: 875,
                over_5ms: 10,
                frame_p99_ms: 13.0,
                frame_max_ms: 13.0,
                ..crate::scorer::StutterScore::default()
            },
        };

        let result = compare_low_risk_experiment(
            &baseline,
            &candidate,
            crate::autotune::comparison::ExperimentDataQuality::High,
            false,
        );

        assert!(matches!(
            result,
            crate::autotune::comparison::ExperimentResult::Improved { .. }
        ));
    }

    #[test]
    fn candidate_measurement_not_ready_blocks_decision_gate() {
        let status = crate::autotune::measurement::CandidateMeasurementWindowStatus::Collecting {
            elapsed_ms: 10_000,
            scored_intervals: 5,
            scored_samples: 50,
            scored_task_count: 1,
            drop_counter_total: 0,
            reasons: vec!["candidate measurement window not complete".to_owned()],
        };

        let err = ensure_candidate_measurement_ready_for_decision(&status)
            .unwrap_err()
            .to_string();

        assert!(err.contains("candidate measurement window is not ready"));
        assert!(err.contains("candidate measurement window not complete"));
    }

    #[test]
    fn baseline_not_ready_blocks_apply_gate() {
        let status = crate::autotune::baseline::BaselineWindowStatus::Collecting {
            elapsed_ms: 10_000,
            scored_intervals: 5,
            scored_samples: 50,
            scored_task_count: 1,
            drop_counter_total: 0,
            reasons: vec!["baseline window not complete".to_owned()],
        };

        let err = ensure_baseline_ready_for_apply(&status)
            .unwrap_err()
            .to_string();

        assert!(err.contains("baseline window is not ready"));
        assert!(err.contains("baseline window not complete"));
    }

    #[test]
    fn washout_config_defaults_are_safe_for_apply_low_risk() {
        let config = WashoutWindowConfig::default();

        assert_eq!(config.washout_seconds, 10);
        assert_eq!(config.verify_interval_ms, 1_000);
        assert_eq!(config.washout_ms(), 10_000);
    }
}
