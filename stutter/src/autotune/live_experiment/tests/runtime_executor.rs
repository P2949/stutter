#![allow(unused_imports)]
use super::{super::*, support::*};

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
