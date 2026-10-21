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
