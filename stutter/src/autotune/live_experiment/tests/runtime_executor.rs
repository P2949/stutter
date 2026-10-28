use super::{super::*, support::*};

#[test]
fn failed_verification_retains_outstanding_rollback_in_daemon_state() {
    let root = crate::test_support::TestRoot::new("failed-apply-rollback-visibility");
    let journal_path = root.join("controller.json");
    let mut manager = LiveExperimentManager::new();
    let mut controller = ControllerRuntimeState::default();
    let mut profile = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor {
        verify_failure: Some(true),
        ..Default::default()
    };
    let result = manager.apply_decision_side_effects_with_executor(
        input(journal_path.clone()),
        LiveExperimentRuntimeState {
            controller_state: &mut controller,
            active_profile_state: &mut profile,
        },
        &observation(1_000, 1_000_000_000),
        &AutotuneDecision::StartExperiment {
            candidate: low_risk_candidate(),
            reason: "fault regression".into(),
        },
        "fault regression",
        &mut executor,
    );
    assert!(result.is_err());
    assert_eq!(controller.phase, ControllerPhase::Faulted);
    let journal =
        crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
    let rollback = manager
        .daemon_rollback_state("stutter daemon emergency-restore")
        .expect("failed rollback must remain visible");
    assert_eq!(rollback.token.as_ref(), journal.rollback_token());
    assert!(rollback.rollback_available);
}

#[test]
fn manager_preserves_executor_owned_applied_transition() {
    let root = crate::test_support::TestRoot::new("executor-journal-ownership");
    let journal_path = root.join("controller.json");
    let input = input(journal_path.clone());
    let candidate = low_risk_candidate();
    let manager = LiveExperimentManager::new();
    manager
        .apply_candidate_for_runtime_with_executor(
            &input,
            &mut FakeLiveExecutor::default(),
            &candidate,
            "journal-owner",
            candidate.action_id().as_str(),
            &observation(1_000, 1_000_000_000),
        )
        .unwrap();
    let journal =
        crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
    assert_eq!(journal.verify_result.as_deref(), Some("fake_verified"));
}

#[test]
#[ignore = "real owned-child affinity mutation; requires two allowed CPUs"]
fn real_runtime_manager_applies_with_isolated_journal_and_startup_restores() {
    struct Owned(std::process::Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let child = Owned(
        std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let tid = stutter_core::ids::Tid::new(child.0.id());
    let original = crate::affinity::read_allowed_mask(tid).unwrap();
    let desired = crate::affinity::CpuMask::parse(
        original.to_range_string().split([',', '-']).next().unwrap(),
    )
    .unwrap();
    assert_ne!(
        original, desired,
        "this explicit live test requires two allowed CPUs"
    );
    let root = crate::test_support::TestRoot::new("real-runtime-journal");
    let journal_path = root.join("controller.json");
    let input = input(journal_path.clone());
    let candidate = CandidateAction::cpu_affinity_profile(
        crate::profiles::Profile {
            name: "owned-child-runtime".into(),
            rules: vec![crate::profiles::ProfileRule {
                affinity: Some(desired.clone()),
                nice: None,
                ionice: None,
                match_class: vec![],
                match_comm: vec![crate::process_tree::CompiledPattern::new("/.*/".into()).unwrap()],
            }],
        },
        child.0.id(),
    );
    let mut observation = observation(1_000, 1_000_000_000);
    observation.target_root_pid = Some(child.0.id());
    let manager = LiveExperimentManager::new();
    let token = manager
        .apply_candidate_for_runtime_with_executor(
            &input,
            &mut RuntimeLiveExperimentActionExecutor,
            &candidate,
            "owned-child-runtime",
            candidate.action_id().as_str(),
            &observation,
        )
        .unwrap();
    assert_eq!(crate::affinity::read_allowed_mask(tid).unwrap(), desired);
    assert!(
        token
            .restore_path()
            .unwrap()
            .starts_with(root.join("experiments"))
    );
    let journal =
        crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
    assert_eq!(journal.rollback_token(), Some(&token));
    assert_eq!(
        journal.verify_result.as_deref(),
        Some("applied_pending_verify")
    );
    let recovered = crate::autotune::startup_recovery::recover_controller_journal_on_startup(
        crate::autotune::startup_recovery::StartupRecoveryConfig {
            rollback_on_crash_recovery: true,
            journal_path: journal_path.clone(),
            audit_path: input.audit_path.unwrap(),
            history_path: root.join("history.jsonl"),
            state_snapshot_path: root.join("daemon-state.json"),
            privileged_worker_socket: None,
        },
    )
    .unwrap();
    assert!(matches!(
        recovered,
        crate::autotune::startup_recovery::StartupRecoveryOutcome::Recovered {
            affected_tasks: 1,
            ..
        }
    ));
    assert_eq!(crate::affinity::read_allowed_mask(tid).unwrap(), original);
    assert!(
        crate::autotune::controller_journal::read_controller_journal(&journal_path)
            .unwrap()
            .is_clean()
    );
}

#[test]
fn runtime_experiments_reserve_distinct_restore_paths() {
    let root = crate::test_support::TestRoot::new("experiment-isolation");
    let journal = root.join("controller.json");
    let first = super::super::journal::reserve_experiment_restore_path(&journal).unwrap();
    let second = super::super::journal::reserve_experiment_restore_path(&journal).unwrap();
    assert_ne!(first, second);
    assert!(first.parent().unwrap().is_dir());
    assert!(second.parent().unwrap().is_dir());
    assert!(!first.exists());
    assert!(!second.exists());
}

#[test]
fn runtime_executor_uses_injected_privileged_service_for_medium_risk_apply() {
    let journal_path = temp_journal_path("medium-injected");
    let service = FakePrivilegedService::default();
    let input = medium_input(journal_path, Some(&service));
    let observation = observation(1_000, 1_000_000_000);
    let mut executor = RuntimeLiveExperimentActionExecutor;

    let rollback = executor
        .apply_candidate(
            &input,
            &medium_risk_candidate(),
            "medium-experiment",
            &observation,
        )
        .unwrap();

    assert!(matches!(rollback, RollbackToken::NiceRestore { .. }));
    assert_eq!(service.apply_calls(), 1);
}

#[test]
fn runtime_executor_requires_privileged_service_for_medium_risk_apply() {
    let journal_path = temp_journal_path("medium-missing-service");
    let input = medium_input(journal_path, None);
    let observation = observation(1_000, 1_000_000_000);
    let mut executor = RuntimeLiveExperimentActionExecutor;

    let err = executor
        .apply_candidate(
            &input,
            &medium_risk_candidate(),
            "medium-experiment",
            &observation,
        )
        .unwrap_err()
        .to_string();

    assert!(err.contains("privileged_worker_required"));
}

/// A privileged service whose prepared apply fails where a test says: before the journal hook
/// (the worker refused), or after it (the commit failed or its answer was lost).
#[derive(Debug, Default)]
struct FailingCommitService {
    refuse_before_hook: bool,
    fail_rollback: bool,
    rollbacks: std::sync::Mutex<usize>,
}

impl PrivilegedActionService for FailingCommitService {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<crate::autotune::planning::dry_run::CandidateDryRunRecord> {
        FakePrivilegedService::default().dry_run_candidate(request)
    }

    fn apply_candidate(
        &self,
        _request: CandidateApplyRequest,
    ) -> anyhow::Result<crate::daemon::privilege::ApplyResult> {
        anyhow::bail!("unused")
    }

    fn apply_candidate_prepared(
        &self,
        _request: CandidateApplyRequest,
        before_commit: &mut crate::daemon::privilege::PreparedHook<'_>,
    ) -> anyhow::Result<crate::daemon::privilege::ApplyResult> {
        if self.refuse_before_hook {
            anyhow::bail!("privileged_worker_request_failed: the worker refused the prepare");
        }
        before_commit(&RollbackToken::NiceRestore {
            records: Vec::new(),
        })?;
        anyhow::bail!("privileged_worker_request_failed: apply failed: failed to preflight target")
    }

    fn rollback(
        &self,
        request: RollbackRequest,
    ) -> anyhow::Result<crate::daemon::privilege::RollbackResult> {
        *self.rollbacks.lock().unwrap() += 1;
        if self.fail_rollback {
            anyhow::bail!("the worker is gone");
        }
        Ok(crate::daemon::privilege::RollbackResult {
            affected_tasks: request.token.affected_tasks(),
        })
    }
}

fn failed_prepared_apply(
    service: &FailingCommitService,
) -> (
    anyhow::Error,
    crate::autotune::controller_journal::ControllerJournalRecord,
) {
    let root = crate::test_support::TestRoot::new("failed-prepared-apply");
    let journal_path = root.join("controller.json");
    let input = medium_input(journal_path.clone(), Some(service));
    let candidate = medium_risk_candidate();
    // The lifecycle writes this pending entry before the executor runs.
    crate::autotune::controller_journal::write_controller_journal_applying_with_metadata(
        &journal_path,
        ExperimentId::try_new("failed-prepared").unwrap(),
        candidate.action_id(),
        super::super::journal::controller_journal_metadata_for_candidate(
            &input,
            &candidate,
            &observation(1_000, 1_000_000_000),
            None,
            "pending_apply",
        ),
    )
    .unwrap();
    let error = RuntimeLiveExperimentActionExecutor
        .apply_candidate(
            &input,
            &candidate,
            "failed-prepared",
            &observation(1_000, 1_000_000_000),
        )
        .unwrap_err();
    let journal =
        crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
    (error, journal)
}

#[test]
fn a_commit_that_fails_after_the_journal_is_rolled_back_and_the_journal_cleared() {
    let service = FailingCommitService::default();
    let (error, journal) = failed_prepared_apply(&service);
    assert!(
        format!("{error:#}").contains("rolled back and the journal cleared"),
        "{error:#}"
    );
    assert_eq!(*service.rollbacks.lock().unwrap(), 1);
    assert!(
        journal.is_clean(),
        "the journal must not stay `applying`: {journal:?}"
    );
}

#[test]
fn a_prepare_the_worker_refused_leaves_no_applying_journal_and_no_rollback() {
    let service = FailingCommitService {
        refuse_before_hook: true,
        ..Default::default()
    };
    let (_, journal) = failed_prepared_apply(&service);
    assert_eq!(*service.rollbacks.lock().unwrap(), 0);
    assert!(journal.is_clean(), "{journal:?}");
}

#[test]
fn a_failed_rollback_keeps_the_token_journalled_for_startup_recovery() {
    let service = FailingCommitService {
        fail_rollback: true,
        ..Default::default()
    };
    let (error, journal) = failed_prepared_apply(&service);
    assert!(
        format!("{error:#}").contains("journal keeps the token"),
        "{error:#}"
    );
    assert_eq!(
        journal.state(),
        crate::autotune::controller_journal::ControllerJournalState::Applying
    );
    assert!(journal.rollback_token().is_some());
}
