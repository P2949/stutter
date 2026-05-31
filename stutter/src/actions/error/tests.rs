use super::*;
use crate::actions::ActionPhase;

#[test]
fn constructors_build_structured_failures() {
    let err = ActionError::verify_rollback_completed("verify intentional failure");
    assert!(matches!(
        err.failure(),
        ActionFailure::Rollback(RollbackOutcome::VerifyRollbackCompleted {
            verify_error
        }) if verify_error == "verify intentional failure"
    ));
    assert_eq!(err.phase(), ActionPhase::Verify);
    assert_eq!(err.category(), "verify_failure_rollback_completed");
    assert_eq!(
        err.to_string(),
        "verify failed; rollback completed: verify intentional failure"
    );

    let err = ActionError::scope_limit_exceeded(8, 3);
    assert!(matches!(
        err.failure(),
        ActionFailure::ScopeLimitExceeded(ScopeLimitExceeded {
            affected_tasks: 8,
            max_affected_tasks: 3,
        })
    ));
    assert_eq!(err.phase(), ActionPhase::DryRun);
    assert_eq!(err.category(), "scope_limit_exceeded");
    assert_eq!(
        err.to_string(),
        "dry run affected 8 task(s), exceeding scope limit 3"
    );
}

#[test]
fn action_error_serialization_preserves_legacy_scope_limit_shape() {
    let err = ActionError::scope_limit_exceeded(8, 3);

    let json_str = serde_json::to_string(&err).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "kind": "scope_limit_exceeded",
            "affected_tasks": 8,
            "max_affected_tasks": 3
        })
    );

    let decoded: ActionError = serde_json::from_str(&json_str).unwrap();
    assert_eq!(decoded, err);
}

#[test]
fn action_error_serialization_preserves_legacy_timeout_rollback_shape() {
    let err = ActionError::timeout_rollback_failure(
        ActionPhase::Apply,
        20,
        10,
        "rollback intentional failure",
    );

    let json_str = serde_json::to_string(&err).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "kind": "timeout_rollback_failure",
            "phase": "apply",
            "elapsed_ms": 20,
            "timeout_ms": 10,
            "rollback_error": "rollback intentional failure"
        })
    );

    let decoded: ActionError = serde_json::from_str(&json_str).unwrap();
    assert_eq!(decoded, err);
    assert!(matches!(
        decoded.failure(),
        ActionFailure::Rollback(RollbackOutcome::TimeoutRollbackFailure { .. })
    ));
}

#[test]
fn action_error_serialization_preserves_invalid_rollback_token_shape() {
    let err = ActionError::invalid_rollback_token("nice-restore", "ioprio-restore");

    assert_eq!(err.phase(), ActionPhase::Rollback);
    assert_eq!(err.category(), "invalid_rollback_token");
    assert_eq!(
        err.to_string(),
        "invalid rollback token: expected nice-restore, actual ioprio-restore"
    );

    let json_str = serde_json::to_string(&err).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "kind": "invalid_rollback_token",
            "expected": "nice-restore",
            "actual": "ioprio-restore"
        })
    );

    let decoded: ActionError = serde_json::from_str(&json_str).unwrap();
    assert_eq!(decoded, err);
}

#[test]
fn action_boundary_error_round_trips_through_action_error_serde() {
    const EXPECTED_REASON: &str = "action_missing_explicit_targets";
    const EXPECTED_MESSAGE: &str =
        "action_missing_explicit_targets: uclamp requires at least one explicit target";

    let source = anyhow::Error::new(ActionBoundaryError::MissingExplicitTargets {
        action_kind: "uclamp",
    });

    let err = ActionError::from_phase_error(ActionPhase::DryRun, source);

    assert!(matches!(
        err.failure(),
        ActionFailure::Boundary(ActionBoundaryFailure {
            phase: ActionPhase::DryRun,
            action_kind,
            reason_code,
            message,
        }) if action_kind == "uclamp"
            && reason_code == EXPECTED_REASON
            && message == EXPECTED_MESSAGE
    ));

    assert_eq!(err.phase(), ActionPhase::DryRun);
    assert_eq!(err.category(), EXPECTED_REASON);
    assert_eq!(
        err.to_string(),
        format!("dry_run failed: {EXPECTED_MESSAGE}")
    );

    let json_str = serde_json::to_string(&err).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "kind": "action_boundary_failure",
            "phase": "dry_run",
            "action_kind": "uclamp",
            "reason_code": EXPECTED_REASON,
            "message": EXPECTED_MESSAGE
        })
    );

    let decoded: ActionError = serde_json::from_str(&json_str).unwrap();
    assert_eq!(decoded, err);

    assert!(matches!(
        decoded.failure(),
        ActionFailure::Boundary(ActionBoundaryFailure {
            phase: ActionPhase::DryRun,
            action_kind,
            reason_code,
            message,
        }) if action_kind == "uclamp"
            && reason_code == EXPECTED_REASON
            && message == EXPECTED_MESSAGE
    ));
}

#[test]
fn action_boundary_invalid_value_round_trips_through_action_error_serde() {
    const EXPECTED_REASON: &str = "action_invalid_value";
    const EXPECTED_MESSAGE: &str =
        "action_invalid_value: gpu-power field power_profile: unsupported profile turbo";

    let source = anyhow::Error::new(ActionBoundaryError::InvalidValue {
        action_kind: "gpu-power",
        field: "power_profile".to_owned(),
        reason: "unsupported profile turbo".to_owned(),
    });

    let err = ActionError::from_phase_error(ActionPhase::Preflight, source);

    assert!(matches!(
        err.failure(),
        ActionFailure::Boundary(ActionBoundaryFailure {
            phase: ActionPhase::Preflight,
            action_kind,
            reason_code,
            message,
        }) if action_kind == "gpu-power"
            && reason_code == EXPECTED_REASON
            && message == EXPECTED_MESSAGE
    ));

    assert_eq!(err.phase(), ActionPhase::Preflight);
    assert_eq!(err.category(), EXPECTED_REASON);
    assert_eq!(
        err.to_string(),
        format!("preflight failed: {EXPECTED_MESSAGE}")
    );

    let json_str = serde_json::to_string(&err).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "kind": "action_boundary_failure",
            "phase": "preflight",
            "action_kind": "gpu-power",
            "reason_code": EXPECTED_REASON,
            "message": EXPECTED_MESSAGE
        })
    );

    let decoded: ActionError = serde_json::from_str(&json_str).unwrap();
    assert_eq!(decoded, err);

    assert!(matches!(
        decoded.failure(),
        ActionFailure::Boundary(ActionBoundaryFailure {
            phase: ActionPhase::Preflight,
            action_kind,
            reason_code,
            message,
        }) if action_kind == "gpu-power"
            && reason_code == EXPECTED_REASON
            && message == EXPECTED_MESSAGE
    ));
}

#[test]
fn action_failure_serialization_preserves_legacy_policy_shape() {
    let failure = ActionFailure::PolicyRejected {
        message: "policy denied action".to_owned(),
    };

    let json_str = serde_json::to_string(&failure).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    assert_eq!(
        value,
        serde_json::json!({
            "kind": "policy_rejected",
            "message": "policy denied action"
        })
    );

    let decoded: ActionFailure = serde_json::from_str(&json_str).unwrap();
    assert_eq!(decoded, failure);
}
