use super::*;

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
