use std::os::unix::fs::MetadataExt;

use super::*;

#[test]
fn managed_worker_start_preserves_an_existing_non_socket() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.sock");
    fs::write(&path, b"user data").unwrap();
    let result = PrivilegedWorkerHandle::spawn_with_executable_and_timing(
        &path,
        Path::new("/bin/true"),
        Duration::from_millis(100),
        Duration::from_millis(5),
    );
    assert!(result.is_err());
    assert_eq!(fs::read(path).unwrap(), b"user data");
}

#[test]
fn managed_worker_start_preserves_a_live_listener() {
    if !unix_socket_bind_supported() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let before = fs::symlink_metadata(&path).unwrap();
    let result = PrivilegedWorkerHandle::spawn_with_executable_and_timing(
        &path,
        Path::new("/bin/true"),
        Duration::from_millis(100),
        Duration::from_millis(5),
    );
    assert!(result.is_err());
    let after = fs::symlink_metadata(&path).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    let _ = listener.accept().unwrap();
}

#[test]
fn worker_socket_cleanup_preserves_a_replacement_file() {
    if !unix_socket_bind_supported() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.sock");
    let _listener = UnixListener::bind(&path).unwrap();
    let metadata = fs::symlink_metadata(&path).unwrap();
    let identity = (metadata.dev(), metadata.ino());
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"replacement data").unwrap();
    assert!(super::super::worker_lifecycle::remove_owned_socket(&path, identity).is_err());
    assert_eq!(fs::read(path).unwrap(), b"replacement data");
}

#[test]
fn managed_worker_start_reaps_a_child_that_never_opens_its_socket() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.sock");
    let script = dir.path().join("fake-worker");
    let pid_file = dir.path().join("pid");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n",
            pid_file.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let result = PrivilegedWorkerHandle::spawn_with_executable_and_timing(
        &path,
        &script,
        Duration::from_millis(500),
        Duration::from_millis(5),
    );
    assert!(result.is_err());
    let pid: u32 = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
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
