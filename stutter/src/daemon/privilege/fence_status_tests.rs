//! The read-only fence-status request: what the worker reports about the fence it enforces.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use super::*;

const NOW: u64 = 1_000_000;
const BOOT: &str = "boot-1";
const UID: u32 = 1000;

fn me() -> u32 {
    crate::syscall::geteuid()
}

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "stutter-fence-status-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn write(dir: &std::path::Path, body: &str) -> PathBuf {
    let path = dir.join(validation_fence::FENCE_FILE);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    path
}

fn body(pid: u32, ticks: u64, expires: u64) -> String {
    format!(
        r#"{{"version":1,"uid":{UID},"root_pid":{pid},"start_ticks":{ticks},"boot_id":"{BOOT}","expires_unix":{expires}}}"#
    )
}

fn report_at(path: &std::path::Path, now: u64) -> FenceReport {
    validation_fence::report(path, now, BOOT, UID, me())
}

#[test]
fn the_report_names_the_state_the_worker_enforces_and_the_digest_of_the_bytes_it_read() {
    let d = dir("states");
    let missing = d.join(validation_fence::FENCE_FILE);
    let absent = report_at(&missing, NOW);
    assert_eq!(
        (absent.state.as_str(), absent.fence_sha256.is_none()),
        ("absent", true)
    );
    assert_eq!((absent.uid, absent.boot_id.as_str()), (UID, BOOT));

    let text = body(500, 777, NOW + 60);
    let path = write(&d, &text);
    let active = report_at(&path, NOW);
    assert_eq!(active.state, "active");
    assert_eq!(
        (active.root_pid, active.start_ticks),
        (Some(500), Some(777))
    );
    assert_eq!(
        active.fence_sha256.as_deref(),
        Some(stutter_core::hash::sha256_hex(text.as_bytes()).as_str())
    );

    let path = write(&d, &body(0, 0, NOW + 60));
    let deny = report_at(&path, NOW);
    assert_eq!((deny.state.as_str(), deny.root_pid), ("deny_all", None));
}

#[test]
fn an_untrustworthy_or_expired_fence_is_reported_as_refusing_with_its_reason() {
    let d = dir("refusing");
    let path = write(&d, &body(500, 777, NOW + 60));
    let expired = report_at(&path, NOW + 61);
    assert_eq!(expired.state, "refusing");
    assert!(
        expired
            .reason
            .unwrap()
            .contains("validation_fence_expired_remove_it")
    );
    // Written by an account the worker does not trust: refusing, and no digest is claimed for it
    // beyond what was actually read (nothing).
    let untrusted = validation_fence::report(&path, NOW, BOOT, UID, me().wrapping_add(1));
    assert_eq!(untrusted.state, "refusing");
    assert!(
        untrusted
            .reason
            .unwrap()
            .contains("validation_fence_untrusted")
    );
    assert!(untrusted.fence_sha256.is_none());
    fs::write(&path, "not json").unwrap();
    let malformed = report_at(&path, NOW);
    assert_eq!(malformed.state, "refusing");
    assert!(
        malformed
            .reason
            .unwrap()
            .contains("validation_fence_malformed")
    );
}

#[test]
fn the_request_is_closed_read_only_and_carries_nothing() {
    let request: PrivilegedWorkerRequest =
        serde_json::from_str(r#"{"request":"fence_status"}"#).unwrap();
    assert!(matches!(request, PrivilegedWorkerRequest::FenceStatus));
    assert_eq!(request.request_name(), "fence_status");
    assert!(request.descriptor().is_none() && request.policy_intent().is_none());
    // The bounded service accepts it as it accepts settle.
    assert!(super::bounded::validate_request(&request).is_ok());
}

#[test]
fn a_service_that_keeps_no_fence_refuses_to_report_one() {
    let service = InProcessPrivilegedActionService::default();
    let error = service.fence_status().unwrap_err().to_string();
    assert!(error.starts_with("fence_status_unsupported"), "{error}");
}

#[test]
fn the_response_round_trips() {
    let report = FenceReport {
        protocol: 1,
        state: "active".to_owned(),
        reason: None,
        root_pid: Some(500),
        start_ticks: Some(777),
        uid: UID,
        boot_id: BOOT.to_owned(),
        fence_sha256: Some("ab".repeat(32)),
    };
    let json = serde_json::to_string(&PrivilegedWorkerResponse::FenceStatus {
        report: report.clone(),
    })
    .unwrap();
    match serde_json::from_str::<PrivilegedWorkerResponse>(&json).unwrap() {
        PrivilegedWorkerResponse::FenceStatus { report: back } => assert_eq!(back, report),
        other => panic!("{other:?}"),
    }
}
