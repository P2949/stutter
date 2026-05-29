use super::*;

#[test]
fn privileged_operation_audit_event_uses_stable_action_id() {
    let event = privileged_operation_audit_event(
        PrivilegedOperation::RollbackAction,
        false,
        "rollback denied",
    );

    assert_eq!(event.command, "daemon_privilege");
    assert_eq!(
        event.action_id.as_deref(),
        Some("privilege-rollback-action")
    );
    assert_eq!(event.safety_class, Some(SafetyClass::ReversibleLowRisk));
    assert!(!event.success);
    assert_eq!(event.message, "rollback denied");
}

#[test]
fn privilege_audit_records_stale_plan_denial() {
    let audit_path = temp_audit_path("stale");
    let service = InProcessPrivilegedActionService::with_audit_path(&audit_path);
    let mut request = fake_apply_request();
    request.plan.created_unix_nanos = 1;
    request.max_plan_age_nanos = 1;

    let _ = service.apply_candidate(request).unwrap_err();

    let events = read_audit_events(&audit_path);
    assert!(events.iter().any(|event| {
        event.message.contains("stage=request_received")
            && event.message.contains("policy_intent=Apply")
    }));
    assert!(events.iter().any(|event| {
        !event.success
            && event.error_category.as_deref() == Some("stale_candidate_plan")
            && event.message.contains("stage=policy_validation")
    }));
    fs::remove_dir_all(audit_path.parent().unwrap()).ok();
}

#[test]
fn privilege_audit_records_descriptor_mismatch_denial() {
    let audit_path = temp_audit_path("descriptor");
    let service = InProcessPrivilegedActionService::with_audit_path(&audit_path);
    let mut request = fake_apply_request();
    request.plan.descriptor.action_kind = "nice".to_owned();

    let _ = service.apply_candidate(request).unwrap_err();

    let events = read_audit_events(&audit_path);
    assert!(events.iter().any(|event| {
        !event.success
            && event.error_category.as_deref() == Some("candidate_plan_descriptor_mismatch")
    }));
    fs::remove_dir_all(audit_path.parent().unwrap()).ok();
}

#[test]
fn privilege_audit_records_missing_evidence_denial() {
    let audit_path = temp_audit_path("missing-evidence");
    let service = InProcessPrivilegedActionService::with_audit_path(&audit_path);
    let mut request = fake_apply_request();
    request.plan.evidence_count = 0;

    let _ = service.apply_candidate(request).unwrap_err();

    let events = read_audit_events(&audit_path);
    assert!(events.iter().any(|event| {
        !event.success && event.error_category.as_deref() == Some("candidate_plan_missing_evidence")
    }));
    fs::remove_dir_all(audit_path.parent().unwrap()).ok();
}

#[test]
fn privilege_audit_records_successful_worker_apply_and_rollback() {
    let audit_path = temp_audit_path("worker-success");
    let audit_sink = PrivilegeAuditSink::to_path(&audit_path);
    let service = FakeWorkerService::default();
    let candidate = nice_candidate(target(200, 100, "worker", 12345));
    let apply_request = PrivilegedWorkerRequest::Apply {
        plan: PrivilegedWorkerCandidatePlan::from_plan_request(
            &nice_apply_request(candidate.clone()).plan,
        ),
        policy: DaemonPolicy::apply_medium_risk(crate::daemon_policy::ActionSource::Test),
        context: DaemonPolicyContext::default(),
        max_plan_age_nanos: 1_000_000_000,
    };
    let apply_response =
        execute_privileged_worker_request_with_audit_sink(apply_request, &service, &audit_sink);
    let rollback_token = match apply_response {
        PrivilegedWorkerResponse::Apply { result } => result.rollback,
        other => panic!("expected apply response, got {other:?}"),
    };

    let rollback_request = PrivilegedWorkerRequest::Rollback {
        plan: PrivilegedWorkerCandidatePlan::from_plan_request(
            &CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
        ),
        token: rollback_token,
        policy: DaemonPolicy::apply_medium_risk(crate::daemon_policy::ActionSource::Test),
        context: DaemonPolicyContext::default(),
    };
    let rollback_response =
        execute_privileged_worker_request_with_audit_sink(rollback_request, &service, &audit_sink);
    assert!(matches!(
        rollback_response,
        PrivilegedWorkerResponse::Rollback { .. }
    ));

    let events = read_audit_events(&audit_path);
    assert!(events.iter().any(|event| {
        event.success
            && event.error_category.as_deref() == Some("apply_completed")
            && event.message.contains("stage=apply_completed")
    }));
    assert!(events.iter().any(|event| {
        event.success
            && event.error_category.as_deref() == Some("rollback_completed")
            && event.message.contains("stage=rollback_completed")
    }));
    fs::remove_dir_all(audit_path.parent().unwrap()).ok();
}
