//! Synthetic faults run through the production action runner, manager and runtime.
//! These contracts are not substitutes for the real process crash matrix.
use super::*;
use crate::autotune::live_experiment::{LiveExperimentActionExecutor, LiveExperimentManagerInput};

#[derive(Clone, Copy, Debug)]
enum Failure {
    Preflight,
    Verify { rollback_fails: bool },
    Partial { rollback_fails: bool },
}
struct Executor {
    failure: Failure,
    calls: usize,
}
impl LiveExperimentActionExecutor for Executor {
    fn apply_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        candidate: &CandidateAction,
        experiment_id: &str,
        _: &AutotuneObservation,
    ) -> anyhow::Result<RollbackToken> {
        self.calls += 1;
        let journal = input.controller_journal_path.as_ref().unwrap();
        let audit = journal.with_file_name("audit.jsonl");
        let action = crate::actions::fake_action::FakeAction::new()
            .with_action_id(candidate.action_id().as_str())
            .with_restore_path(journal.with_file_name("synthetic-restore.json"));
        let action = match self.failure {
            Failure::Preflight => action.with_fail_preflight(),
            Failure::Verify { rollback_fails } => {
                let action = action.with_fail_verify();
                if rollback_fails {
                    action.with_fail_rollback()
                } else {
                    action
                }
            }
            Failure::Partial { rollback_fails } => {
                let action = action.with_partial_apply_failure();
                if rollback_fails {
                    action.with_fail_rollback()
                } else {
                    action
                }
            }
        };
        let outcome = crate::autotune::action_transaction::apply_journaled_action(
            &action,
            crate::actions::runner::ActionRunPolicy::apply_low_risk(ActionSource::Test, false),
            crate::autotune::action_transaction::JournaledActionInput {
                journal_path: journal,
                audit_path: &audit,
                experiment_id,
                metadata: Default::default(),
            },
        )?;
        Ok(outcome.rollback.unwrap())
    }
    fn rollback_candidate(
        &mut self,
        _: &LiveExperimentManagerInput<'_>,
        _: &LiveExperiment,
        _: &AutotuneObservation,
    ) -> anyhow::Result<Option<ActiveConfigSnapshot>> {
        unreachable!("automatic failure rollback belongs to the real action runner")
    }
}

#[test]
fn failed_action_reports_agree_and_block_future_mutation() {
    for (failure, rollback_completed, outstanding) in [
        (Failure::Preflight, false, false),
        (
            Failure::Verify {
                rollback_fails: false,
            },
            true,
            false,
        ),
        (
            Failure::Verify {
                rollback_fails: true,
            },
            false,
            true,
        ),
        (
            Failure::Partial {
                rollback_fails: false,
            },
            true,
            false,
        ),
        (
            Failure::Partial {
                rollback_fails: true,
            },
            false,
            true,
        ),
    ] {
        let root = crate::test_support::TestRoot::new("runtime-fault-consistency");
        let journal_path = root.join("controller.json");
        let mut config = AutotuneRuntimeConfig::from_daemon_config(
            daemon_config_for_runtime_mode(
                DaemonMode::ApplyLowRisk,
                ActionSource::Test,
                Some(1234),
                None,
            ),
            Some(root.join("decisions.jsonl")),
        );
        config.controller_journal_path = Some(journal_path.clone());
        config.history_log = Some(root.join("history.jsonl"));
        let mut runtime = AutotuneRuntime::new(config);
        let decision = AutotuneDecision::StartExperiment {
            candidate: CandidateAction::fake(
                crate::actions::ActionId::new("fault-action"),
                SafetyClass::ReversibleLowRisk,
            ),
            reason: "synthetic fault contract".into(),
        };
        let observation = high_quality_game_observation_with_focus_confidence(0.95);
        let mut executor = Executor { failure, calls: 0 };
        let error = runtime
            .apply_decision_side_effects_with_executor(
                &observation,
                &decision,
                "fault contract",
                &mut executor,
            )
            .unwrap_err();
        let typed = error
            .downcast_ref::<crate::autotune::action_transaction::JournaledActionFailure>()
            .expect("preserve typed runner outcome");
        assert_eq!(typed.rollback_completed, rollback_completed, "{failure:?}");
        assert_eq!(runtime.runtime_phase(), AutotuneRuntimePhase::Faulted);
        let journal =
            crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
        let state =
            crate::daemon::state::load_daemon_state(&root.join("daemon_state.json")).unwrap();
        assert_eq!(state.phase, DaemonPhase::Faulted);
        assert_eq!(state.active_rollback.is_some(), outstanding, "{failure:?}");
        if outstanding {
            assert_eq!(
                state.active_rollback.as_ref().unwrap().token.as_ref(),
                journal.rollback_token()
            );
            assert!(
                state
                    .active_rollback
                    .as_ref()
                    .unwrap()
                    .manual_restore_command
                    .is_some()
            );
            assert!(!journal.is_clean());
        } else if rollback_completed {
            assert!(journal.is_clean());
        }
        let history =
            crate::autotune::history::read_autotune_history_events(&root.join("history.jsonl"))
                .unwrap();
        assert!(!history.is_empty());
        for entry in history {
            assert_eq!(
                entry.phase,
                crate::autotune::history::ControllerPhase::Faulted
            );
            assert_eq!(entry.rollback_performed, rollback_completed, "{failure:?}");
            assert!(entry.action_id.is_some());
            assert!(!entry.decision.eligible);
        }
        let audit = crate::audit::read_audit_tail(&root.join("audit.jsonl"), 20).unwrap();
        assert!(!audit.last().unwrap().success);
        assert!(
            state
                .last_decision
                .unwrap()
                .reason
                .contains("runtime decision side effect failed")
        );
        assert!(
            runtime
                .apply_decision_side_effects_with_executor(
                    &observation,
                    &decision,
                    "must stay blocked",
                    &mut executor
                )
                .is_err()
        );
        assert_eq!(executor.calls, 1);
    }
}

#[test]
fn reporting_io_failure_preserves_original_error_and_still_saves_fault_snapshot() {
    let root = crate::test_support::TestRoot::new("fault-report-io");
    let history = root.join("history-is-directory");
    fs::create_dir(&history).unwrap();
    let mut config = AutotuneRuntimeConfig::from_daemon_config(
        daemon_config_for_runtime_mode(
            DaemonMode::ApplyLowRisk,
            ActionSource::Test,
            Some(1234),
            None,
        ),
        Some(root.join("decisions.jsonl")),
    );
    config.controller_journal_path = Some(root.join("controller.json"));
    config.history_log = Some(history);
    let mut runtime = AutotuneRuntime::new(config);
    let decision = AutotuneDecision::StartExperiment {
        candidate: CandidateAction::fake(
            crate::actions::ActionId::new("fault-io-action"),
            SafetyClass::ReversibleLowRisk,
        ),
        reason: "reporting IO regression".into(),
    };
    let mut executor = Executor {
        failure: Failure::Verify {
            rollback_fails: true,
        },
        calls: 0,
    };
    let error = runtime
        .apply_decision_side_effects_with_executor(
            &high_quality_game_observation_with_focus_confidence(0.95),
            &decision,
            "fault contract",
            &mut executor,
        )
        .unwrap_err();
    let original = error
        .downcast_ref::<crate::autotune::action_transaction::JournaledActionFailure>()
        .unwrap();
    assert!(original.rollback.is_some());
    assert!(!original.rollback_completed);
    assert!(
        error
            .to_string()
            .contains("failed to persist side-effect fault report")
    );
    let state = crate::daemon::state::load_daemon_state(&root.join("daemon_state.json")).unwrap();
    assert_eq!(state.phase, DaemonPhase::Faulted);
    assert!(state.active_rollback.unwrap().token.is_some());
    assert!(root.join("decisions.jsonl").is_file());
}
