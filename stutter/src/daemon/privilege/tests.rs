use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use super::*;
use crate::{
    actions::{ActionState, SafetyClass},
    audit::AuditEvent,
    autotune::objective::ObjectiveKind,
};

fn check(request: PrivilegeCommandRequest) -> PrivilegeDecision {
    PrivilegeCommandAllowlist.check(&request)
}

#[test]
fn control_plane_can_request_privileged_worker_operation_over_unix_socket() {
    let decision = check(PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::ControlPlane,
        operation: PrivilegedOperation::ApplyAction,
        transport: PrivilegeTransport::UnixSocket,
        authenticated: true,
        apply_authorized: true,
    });

    assert!(decision.allowed);
    assert!(decision.privileged_worker_required);
    assert!(decision.audit_required);
    assert_eq!(
        decision.reason_code,
        "allowlisted_control_plane_worker_request"
    );
}

#[test]
fn ui_client_cannot_request_privileged_worker_operation() {
    let decision = check(PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::UiClient,
        operation: PrivilegedOperation::RollbackAction,
        transport: PrivilegeTransport::UnixSocket,
        authenticated: true,
        apply_authorized: true,
    });

    assert!(!decision.allowed);
    assert_eq!(
        decision.reason_code,
        "caller_role_cannot_request_privileged_operation"
    );
}

#[test]
fn remote_tcp_cannot_request_privileged_operation_even_with_apply_auth() {
    let decision = check(PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::ControlPlane,
        operation: PrivilegedOperation::StartRecording,
        transport: PrivilegeTransport::RemoteTcp,
        authenticated: true,
        apply_authorized: true,
    });

    assert!(!decision.allowed);
    assert_eq!(decision.reason_code, "non_local_privileged_operation");
}

#[test]
fn loopback_tcp_privileged_operation_requires_apply_authorization() {
    let decision = check(PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::ControlPlane,
        operation: PrivilegedOperation::ControlDaemon,
        transport: PrivilegeTransport::LoopbackTcp,
        authenticated: true,
        apply_authorized: false,
    });

    assert!(!decision.allowed);
    assert_eq!(decision.reason_code, "missing_apply_authorization");
}

#[test]
fn status_read_is_unprivileged_but_still_requires_authenticated_boundary_context() {
    let denied = check(PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::UiClient,
        operation: PrivilegedOperation::ReadStatus,
        transport: PrivilegeTransport::RemoteTcp,
        authenticated: false,
        apply_authorized: false,
    });
    let allowed = check(PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::UiClient,
        operation: PrivilegedOperation::ReadStatus,
        transport: PrivilegeTransport::RemoteTcp,
        authenticated: true,
        apply_authorized: false,
    });

    assert!(!denied.allowed);
    assert_eq!(denied.reason_code, "missing_authentication");
    assert!(allowed.allowed);
    assert!(!allowed.privileged_worker_required);
    assert!(!allowed.audit_required);
}

#[test]
fn privileged_worker_can_execute_allowlisted_worker_operations() {
    let decision = check(PrivilegeCommandRequest {
        caller_role: PrivilegeProcessRole::PrivilegedWorker,
        operation: PrivilegedOperation::LoadEbpf,
        transport: PrivilegeTransport::LocalCli,
        authenticated: false,
        apply_authorized: false,
    });

    assert!(decision.allowed);
    assert_eq!(decision.reason_code, "privileged_worker_execution_allowed");
    assert!(decision.privileged_worker_required);
}

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

fn fake_apply_request() -> CandidateApplyRequest {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake:privilege".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    CandidateApplyRequest {
        plan: CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
        policy: DaemonPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test),
        context: DaemonPolicyContext::default(),
        max_plan_age_nanos: 1_000_000_000,
    }
}

fn nice_candidate(target: TaskIdentity) -> CandidateAction {
    CandidateAction::Nice {
        plan: crate::autotune::planning::executable_plan::NiceActionPlan {
            name: format!("nice-{}", target.tid),
            action: crate::actions::nice::NiceAction {
                targets: vec![target],
                nice: 5,
                policy: crate::actions::nice::NicePolicy::default(),
            },
            target_root_pid: Some(100),
            evidence: vec![
                crate::autotune::planning::candidate::CandidateEvidence::new(
                    "test",
                    "target revalidation test",
                    1.0,
                ),
            ],
            objective: ObjectiveKind::DesktopInteractivity,
        },
    }
}

fn nice_apply_request(candidate: CandidateAction) -> CandidateApplyRequest {
    CandidateApplyRequest {
        plan: CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
        policy: DaemonPolicy::apply_medium_risk(crate::daemon_policy::ActionSource::Test),
        context: DaemonPolicyContext::default(),
        max_plan_age_nanos: 1_000_000_000,
    }
}

fn target(tid: u32, process_pid: u32, comm: &str, starttime: u64) -> TaskIdentity {
    TaskIdentity {
        tid: tid.into(),
        process_pid: Some((process_pid).into()),
        comm: Some(comm.to_owned()),
        starttime_ticks: Some(starttime),
    }
}

fn temp_proc_root(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "stutter-target-revalidation-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn temp_audit_path(name: &str) -> PathBuf {
    let mut root = std::env::temp_dir();
    root.push(format!(
        "stutter-privilege-audit-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&root).unwrap();
    root.join("audit.jsonl")
}

fn read_audit_events(path: &std::path::Path) -> Vec<AuditEvent> {
    crate::audit::read_audit_tail(path, 100).unwrap()
}

fn proc_stat(tid: u32, comm: &str, starttime: u64) -> String {
    let mut fields = vec!["S".to_owned()];
    fields.extend((0..18).map(|_| "0".to_owned()));
    fields.push(starttime.to_string());
    fields.extend((0..24).map(|_| "0".to_owned()));
    format!("{tid} ({comm}) {}\n", fields.join(" "))
}

fn write_expected_task(
    proc_root: &std::path::Path,
    process_pid: u32,
    tid: u32,
    comm: &str,
    starttime: u64,
) {
    let task_dir = proc_root
        .join(process_pid.to_string())
        .join("task")
        .join(tid.to_string());
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(task_dir.join("stat"), proc_stat(tid, comm, starttime)).unwrap();
}

fn write_top_level_task(
    proc_root: &std::path::Path,
    tgid: u32,
    tid: u32,
    comm: &str,
    starttime: u64,
) {
    let task_dir = proc_root.join(tid.to_string());
    fs::create_dir_all(&task_dir).unwrap();
    fs::write(task_dir.join("stat"), proc_stat(tid, comm, starttime)).unwrap();
    fs::write(
        task_dir.join("status"),
        format!("Name:\t{comm}\nTgid:\t{tgid}\n"),
    )
    .unwrap();
}

#[derive(Debug, Default)]
struct FakeWorkerService {
    dry_run_calls: Mutex<usize>,
    apply_calls: Mutex<usize>,
    rollback_calls: Mutex<usize>,
}

impl FakeWorkerService {
    fn calls(&self, field: &Mutex<usize>) -> usize {
        *field.lock().unwrap()
    }
}

impl PrivilegedActionService for FakeWorkerService {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<CandidateDryRunRecord> {
        *self.dry_run_calls.lock().unwrap() += 1;
        Ok(CandidateDryRunRecord {
            candidate_name: request.plan.candidate.candidate_name().to_owned(),
            affected_tasks: 2,
            warnings: Vec::new(),
            safety_class: request.plan.candidate.safety_class(),
            eligible: true,
            reason: None,
        })
    }

    fn apply_candidate(&self, _request: CandidateApplyRequest) -> anyhow::Result<ApplyResult> {
        *self.apply_calls.lock().unwrap() += 1;
        Ok(ApplyResult {
            state: ActionState {
                applied: true,
                affected_tasks: 2,
                checked_tasks: 2,
                pending_changes: 2,
                warnings: Vec::new(),
            },
            rollback: RollbackToken::NiceRestore {
                records: Vec::new(),
            },
        })
    }

    fn rollback(&self, request: RollbackRequest) -> anyhow::Result<RollbackResult> {
        *self.rollback_calls.lock().unwrap() += 1;
        Ok(RollbackResult {
            affected_tasks: request.token.affected_tasks(),
        })
    }
}

fn temp_socket_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "stutter-privileged-worker-{name}-{}-{}.sock",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    path
}

fn wait_for_socket(path: &std::path::Path) {
    for _ in 0..100 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for socket {}", path.display());
}

fn unix_socket_bind_supported() -> bool {
    let socket = temp_socket_path("support-probe");
    match UnixListener::bind(&socket) {
        Ok(listener) => {
            drop(listener);
            fs::remove_file(socket).ok();
            true
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => false,
        Err(err) => panic!("unexpected privileged-worker unix socket probe error: {err}"),
    }
}

#[test]
fn privileged_worker_socket_wait_succeeds_when_listener_is_connectable() {
    if !unix_socket_bind_supported() {
        return;
    }

    let socket = temp_socket_path("ready-listener");
    let listener = UnixListener::bind(&socket).unwrap();
    let accept = thread::spawn(move || listener.accept().map(|_| ()));

    wait_for_privileged_worker_socket_with_timing(
        &socket,
        Duration::from_millis(200),
        Duration::from_millis(5),
    )
    .unwrap();

    accept.join().unwrap().unwrap();
    fs::remove_file(socket).ok();
}

#[test]
fn privileged_worker_socket_wait_rejects_stale_socket_path() {
    if !unix_socket_bind_supported() {
        return;
    }

    let socket = temp_socket_path("stale-path");
    let listener = UnixListener::bind(&socket).unwrap();
    drop(listener);
    assert!(socket.exists());

    let err = wait_for_privileged_worker_socket_with_timing(
        &socket,
        Duration::from_millis(60),
        Duration::from_millis(5),
    )
    .unwrap_err()
    .to_string();

    assert!(err.contains("privileged_worker_socket_not_ready"));
    assert!(err.contains("was not connectable within 60ms"));
    assert!(err.contains("last_error="));
    fs::remove_file(socket).ok();
}

#[test]
fn privileged_worker_socket_wait_timeout_reports_clear_error() {
    if !unix_socket_bind_supported() {
        return;
    }

    let socket = temp_socket_path("missing-path");

    let err = wait_for_privileged_worker_socket_with_timing(
        &socket,
        Duration::from_millis(40),
        Duration::from_millis(5),
    )
    .unwrap_err()
    .to_string();

    assert!(err.contains("privileged_worker_socket_not_ready"));
    assert!(err.contains("was not connectable within 40ms"));
    assert!(err.contains(socket.to_string_lossy().as_ref()));
}

#[test]
fn privileged_worker_socket_wait_uses_supplied_retry_interval() {
    if !unix_socket_bind_supported() {
        return;
    }

    let socket = temp_socket_path("retry-interval");
    let started = Instant::now();

    let err = wait_for_privileged_worker_socket_with_timing(
        &socket,
        Duration::from_millis(5),
        Duration::from_millis(25),
    )
    .unwrap_err()
    .to_string();

    assert!(started.elapsed() >= Duration::from_millis(20));
    assert!(err.contains("privileged_worker_socket_not_ready"));
}

#[test]
fn unix_socket_privileged_worker_round_trips_apply_and_rollback() {
    if !unix_socket_bind_supported() {
        return;
    }

    let socket = temp_socket_path("round-trip");
    let service = Arc::new(FakeWorkerService::default());
    let worker_service = Arc::clone(&service);
    let worker_socket = socket.clone();
    let handle = thread::spawn(move || {
        run_privileged_worker_with_service(&worker_socket, worker_service.as_ref())
    });
    wait_for_socket(&socket);

    let metadata = fs::metadata(&socket).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

    let client = UnixSocketPrivilegedActionService::new(&socket);
    let candidate = nice_candidate(target(200, 100, "worker", 12345));
    let apply = client
        .apply_candidate(nice_apply_request(candidate.clone()))
        .unwrap();
    assert_eq!(apply.state.affected_tasks, 2);

    let rollback = client
        .rollback(RollbackRequest {
            candidate,
            token: apply.rollback,
            policy: DaemonPolicy::apply_medium_risk(crate::daemon_policy::ActionSource::Test),
            context: DaemonPolicyContext::default(),
        })
        .unwrap();
    assert_eq!(rollback.affected_tasks, 0);

    client.request_shutdown_for_tests().unwrap();
    handle.join().unwrap().unwrap();

    assert_eq!(service.calls(&service.apply_calls), 1);
    assert_eq!(service.calls(&service.rollback_calls), 1);
    assert!(!socket.exists());
}

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
