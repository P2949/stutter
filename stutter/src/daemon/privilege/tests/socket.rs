use super::*;

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
