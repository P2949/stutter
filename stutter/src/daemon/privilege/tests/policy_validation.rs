use super::*;

#[test]
fn privileged_worker_rejects_non_executable_candidate_plan() {
    let request = PrivilegedWorkerRequest::Apply {
        plan: PrivilegedWorkerCandidatePlan::from_plan_request(&fake_apply_request().plan),
        policy: DaemonPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test),
        context: DaemonPolicyContext::default(),
        max_plan_age_nanos: 1_000_000_000,
    };

    let response = execute_privileged_worker_request(request, &FakeWorkerService::default());

    let PrivilegedWorkerResponse::Error { reason_code, .. } = response else {
        panic!("expected worker error");
    };
    assert_eq!(
        reason_code,
        "privileged_worker_candidate_plan_not_executable"
    );
}

#[test]
fn privileged_action_service_rejects_stale_candidate_plan_before_execution() {
    let service = InProcessPrivilegedActionService::default();
    let mut request = fake_apply_request();
    request.plan.created_unix_nanos = 1;
    request.max_plan_age_nanos = 1;

    let err = service.apply_candidate(request).unwrap_err().to_string();

    assert!(err.contains("stale_candidate_plan"));
}

#[test]
fn privileged_action_service_rechecks_descriptor_integrity() {
    let service = InProcessPrivilegedActionService::default();
    let mut request = fake_apply_request();
    request.plan.descriptor.action_kind = "nice".to_owned();

    let err = service.apply_candidate(request).unwrap_err().to_string();

    assert!(err.contains("candidate_plan_descriptor_mismatch"));
}

#[test]
fn privileged_action_service_rejects_payload_without_evidence() {
    let service = InProcessPrivilegedActionService::default();
    let mut request = fake_apply_request();
    request.plan.evidence_count = 0;

    let err = service.apply_candidate(request).unwrap_err().to_string();

    assert!(err.contains("candidate_plan_missing_evidence"));
}

#[test]
fn target_revalidation_accepts_valid_task_identity() {
    let proc_root = temp_proc_root("valid");
    write_expected_task(&proc_root, 100, 200, "worker", 12345);
    let candidate = nice_candidate(target(200, 100, "worker", 12345));

    revalidate_candidate_targets(&candidate, &proc_root).unwrap();

    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn target_revalidation_rejects_missing_tid() {
    let proc_root = temp_proc_root("missing");
    let candidate = nice_candidate(target(200, 100, "worker", 12345));

    let err = revalidate_candidate_targets(&candidate, &proc_root)
        .unwrap_err()
        .to_string();

    assert!(err.contains("target_revalidation_missing_tid"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn target_revalidation_rejects_reused_tid_starttime() {
    let proc_root = temp_proc_root("reused");
    write_expected_task(&proc_root, 100, 200, "worker", 99999);
    let candidate = nice_candidate(target(200, 100, "worker", 12345));

    let err = revalidate_candidate_targets(&candidate, &proc_root)
        .unwrap_err()
        .to_string();

    assert!(err.contains("target_revalidation_starttime_mismatch"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn target_revalidation_rejects_process_pid_mismatch() {
    let proc_root = temp_proc_root("pid-mismatch");
    write_top_level_task(&proc_root, 201, 200, "worker", 12345);
    let candidate = nice_candidate(target(200, 100, "worker", 12345));

    let err = revalidate_candidate_targets(&candidate, &proc_root)
        .unwrap_err()
        .to_string();

    assert!(err.contains("target_revalidation_process_pid_mismatch"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn target_revalidation_rejects_comm_mismatch() {
    let proc_root = temp_proc_root("comm-mismatch");
    write_expected_task(&proc_root, 100, 200, "other", 12345);
    let candidate = nice_candidate(target(200, 100, "worker", 12345));

    let err = revalidate_candidate_targets(&candidate, &proc_root)
        .unwrap_err()
        .to_string();

    assert!(err.contains("target_revalidation_comm_mismatch"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn privileged_apply_revalidates_targets_before_execution() {
    let proc_root = temp_proc_root("service-comm-mismatch");
    write_expected_task(&proc_root, 100, 200, "other", 12345);
    let service = InProcessPrivilegedActionService::with_proc_root(&proc_root);
    let request = nice_apply_request(nice_candidate(target(200, 100, "worker", 12345)));

    let err = service.apply_candidate(request).unwrap_err().to_string();

    assert!(err.contains("target_revalidation_comm_mismatch"));
    fs::remove_dir_all(proc_root).ok();
}

#[test]
fn privileged_worker_plan_rejects_empty_action_id() {
    let request = fake_apply_request();
    let mut wire = PrivilegedWorkerCandidatePlan::from_plan_request(&request.plan);

    wire.descriptor.action_id = crate::actions::ActionId::new("");
    wire.plan_file.descriptor.action_id = crate::actions::ActionId::new("");

    let err = wire
        .into_plan_request()
        .expect_err("empty action id should be rejected");

    let typed = err
        .downcast_ref::<crate::daemon::privilege::PrivilegedWorkerError>()
        .expect("expected typed privileged worker error");

    assert_eq!(
        typed.reason_code(),
        "privileged_worker_invalid_action_descriptor"
    );
    assert!(err.to_string().contains("ActionId cannot be empty"));
}
