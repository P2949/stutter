//! The privileged autotune worker is a typed runner action: the program, its arguments and its
//! socket are derived from the authorization, and only the pinned stutter binary can be started.
use std::{os::unix::fs::PermissionsExt, path::Path};

use super::{Scratch, authorization, envelope, grant, me, peer};
use crate::{
    model::{ErrorCode, Request, RunnerAction},
    process::{ActionExecutor, SystemExecutor, plan_privileged_worker, worker_socket_path},
};

fn sha256(bytes: &[u8]) -> String {
    use std::io::Write;
    let mut child = std::process::Command::new("sha256sum")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(bytes).unwrap();
    let output = child.wait_with_output().unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

/// A repository whose `target/debug/stutter` is a script that creates the socket it is given.
fn repository_with_fake_worker(root: &Path) -> String {
    let bin = root.join("target/debug");
    std::fs::create_dir_all(&bin).unwrap();
    let script = b"#!/bin/sh\n: > \"$3\"\nexec sleep 60\n";
    std::fs::write(bin.join("stutter"), script).unwrap();
    std::fs::set_permissions(bin.join("stutter"), std::fs::Permissions::from_mode(0o755)).unwrap();
    sha256(script)
}

#[test]
fn the_launch_is_the_pinned_stutter_with_a_derived_socket_and_nothing_else() {
    let auth = authorization("/work/stutter");
    let plan = plan_privileged_worker(&auth, Path::new("/run/stutter-validation"));
    assert_eq!(
        plan.program,
        Path::new("/work/stutter/target/debug/stutter")
    );
    assert_eq!(
        plan.args,
        [
            "privileged-worker",
            "--socket",
            "/run/stutter-validation/worker-core-abc1234-1.sock"
        ]
    );
    assert_eq!(
        plan.raw,
        worker_socket_path(Path::new("/run/stutter-validation"), &auth)
    );
}

#[test]
fn only_a_campaign_granted_the_action_may_ask_for_a_worker() {
    let mut auth = authorization("/work/stutter");
    let ask = |auth: &crate::model::Authorization, request| {
        auth.authorize(&peer(me()), &envelope("campaign", request), 8)
    };
    assert_eq!(
        ask(&auth, Request::StartAutotunePrivilegedWorker)
            .unwrap_err()
            .0,
        ErrorCode::ActionNotPermitted
    );
    auth.grants.insert(grant(
        "campaign",
        RunnerAction::StartAutotunePrivilegedWorker,
        None,
    ));
    assert!(ask(&auth, Request::StartAutotunePrivilegedWorker).is_ok());
    // Stop is a separate grant.
    assert!(ask(&auth, Request::StopAutotunePrivilegedWorker).is_err());
}

#[test]
fn the_worker_starts_once_on_its_socket_and_stops_on_request() {
    let scratch = Scratch::new("privileged-worker");
    let repository = scratch.path().join("repo");
    let digest = repository_with_fake_worker(&repository);
    let mut auth = authorization(&repository.to_string_lossy());
    auth.stutter_sha256 = digest;
    // The runner clears the environment; the fake worker needs `sleep` on the derived PATH.
    auth.path_env = std::env::var("PATH").unwrap_or_default();
    let executor = SystemExecutor::new(false, scratch.path().join("run"));
    let first = executor.start_privileged_worker(&auth).unwrap();
    let socket = worker_socket_path(&scratch.path().join("run"), &auth);
    assert!(socket.exists());
    assert_eq!(first["already_running"], false);
    let second = executor.start_privileged_worker(&auth).unwrap();
    assert_eq!(second["already_running"], true);
    assert_eq!(second["pid"], first["pid"]);
    let stopped = executor.stop_privileged_worker(&auth).unwrap();
    assert_eq!(stopped["stopped"], true);
    assert!(!socket.exists());
    assert_eq!(
        executor.stop_privileged_worker(&auth).unwrap()["stopped"],
        false
    );
}

#[test]
fn a_changed_stutter_binary_is_never_started_as_root() {
    let scratch = Scratch::new("privileged-worker-tamper");
    let repository = scratch.path().join("repo");
    let digest = repository_with_fake_worker(&repository);
    let mut auth = authorization(&repository.to_string_lossy());
    auth.stutter_sha256 = digest;
    std::fs::write(
        repository.join("target/debug/stutter"),
        b"#!/bin/sh\nexit 0\n",
    )
    .unwrap();
    let executor = SystemExecutor::new(false, scratch.path().join("run"));
    let error = executor.start_privileged_worker(&auth).unwrap_err();
    assert_eq!(error.0, ErrorCode::ExecutionFailed);
    assert!(error.1.contains("pinned digest"), "{}", error.1);
}
