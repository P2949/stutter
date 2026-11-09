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

#[test]
fn an_offline_receipt_needs_its_own_grant_a_known_scope_and_a_bounded_lifetime() {
    use crate::model::ReceiptScope;
    let mut auth = authorization("/work/stutter");
    let ask = |auth: &crate::model::Authorization, lifetime| {
        auth.authorize(
            &peer(me()),
            &envelope(
                "campaign",
                Request::IssueOfflineReceipt {
                    scope: ReceiptScope::PowerProfileHold,
                    lifetime_seconds: lifetime,
                },
            ),
            8,
        )
    };
    // Having other grants is not enough.
    auth.grants.insert(grant(
        "campaign",
        RunnerAction::StartAutotunePrivilegedWorker,
        None,
    ));
    assert_eq!(
        ask(&auth, 600).unwrap_err().0,
        ErrorCode::ActionNotPermitted
    );
    let mut scoped = grant("campaign", RunnerAction::IssueOfflineReceipt, None);
    scoped.receipt_scope = Some(ReceiptScope::PowerProfileHold);
    auth.grants.insert(scoped);
    assert!(ask(&auth, 600).is_ok());
    for bad in [0, crate::receipt::MAX_LIFETIME_SECONDS + 1] {
        assert_eq!(
            ask(&auth, bad).unwrap_err().0,
            ErrorCode::InvalidArgument,
            "{bad}"
        );
    }
}

#[test]
fn the_default_executor_refuses_to_issue_a_receipt() {
    use crate::{model::ReceiptScope, process::ActionExecutor};
    struct Bare;
    impl ActionExecutor for Bare {
        fn live_suite(
            &self,
            _: &crate::model::Authorization,
            _: &crate::model::Envelope,
        ) -> Result<serde_json::Value, crate::authorization::Denial> {
            unreachable!()
        }
        fn scheduler_burst(
            &self,
            _: &crate::model::Authorization,
            _: &crate::model::Envelope,
        ) -> Result<serde_json::Value, crate::authorization::Denial> {
            unreachable!()
        }
    }
    let auth = authorization("/work/stutter");
    let error = Bare
        .issue_offline_receipt(&auth, ReceiptScope::SchedulerSwitch, 60)
        .unwrap_err();
    assert_eq!(error.0, ErrorCode::Unsupported);
}

#[test]
fn a_receipt_grant_is_scope_specific_so_a_granted_scope_never_opens_another() {
    use crate::model::ReceiptScope;
    let mut auth = authorization("/work/stutter");
    let mut power = grant("campaign", RunnerAction::IssueOfflineReceipt, None);
    power.receipt_scope = Some(ReceiptScope::PowerProfileHold);
    auth.grants.insert(power);
    let ask = |scope| {
        auth.authorize(
            &peer(me()),
            &envelope(
                "campaign",
                Request::IssueOfflineReceipt {
                    scope,
                    lifetime_seconds: 60,
                },
            ),
            8,
        )
    };
    assert!(ask(ReceiptScope::PowerProfileHold).is_ok());
    for other in [ReceiptScope::SchedulerSwitch, ReceiptScope::UnitProperties] {
        assert_eq!(
            ask(other).unwrap_err().0,
            ErrorCode::TaskNotPermitted,
            "{other:?}"
        );
    }
}

#[test]
fn an_authorization_with_a_malformed_receipt_grant_does_not_validate() {
    use crate::model::ReceiptScope;
    let mut auth = authorization("/work/stutter");
    // The action without a scope, and a scope on another action.
    auth.grants
        .insert(grant("campaign", RunnerAction::IssueOfflineReceipt, None));
    assert!(auth.validate_shape().is_err());
    auth.grants.clear();
    let mut stray = grant(
        "campaign",
        RunnerAction::StartAutotunePrivilegedWorker,
        None,
    );
    stray.receipt_scope = Some(ReceiptScope::SchedulerSwitch);
    auth.grants.insert(stray);
    assert!(auth.validate_shape().is_err());
    auth.grants.clear();
    let mut fine = grant("campaign", RunnerAction::IssueOfflineReceipt, None);
    fine.receipt_scope = Some(ReceiptScope::UnitProperties);
    auth.grants.insert(fine);
    assert!(auth.validate_shape().is_ok(), "{:?}", auth.validate_shape());
}
