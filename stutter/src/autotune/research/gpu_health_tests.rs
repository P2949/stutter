use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, Mutex},
};

use super::*;

fn script(body: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journalctl");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    (dir, path)
}

#[test]
fn matching_kernel_lines_are_counted_with_the_newest_event_time() {
    let (_dir, path) = script(
        "echo '1700000100.5 host kernel: amdgpu: GPU reset begin!'; echo '1700000200.1 host kernel: amdgpu: MODE1 reset'",
    );
    let scan = JournalGpuResetProbe::new(path)
        .unwrap()
        .resets_since(0)
        .unwrap();
    assert_eq!(scan.count, 2);
    assert_eq!(scan.latest_unix, Some(1_700_000_200));
}

#[test]
fn no_match_is_zero_and_an_unreadable_journal_is_unknown() {
    let (_dir, none) = script("exit 1");
    assert_eq!(
        JournalGpuResetProbe::new(none)
            .unwrap()
            .resets_since(0)
            .unwrap(),
        ResetScan::quiet()
    );
    let (_dir2, broken) = script("exit 2");
    assert!(
        JournalGpuResetProbe::new(broken)
            .unwrap()
            .resets_since(0)
            .is_err()
    );
}

#[test]
fn a_hung_query_times_out_as_unknown_and_a_relative_path_is_refused() {
    let (_dir, hung) = script("sleep 30");
    let started = Instant::now();
    assert!(
        JournalGpuResetProbe::new(hung)
            .unwrap()
            .resets_since(0)
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(8));
    assert!(JournalGpuResetProbe::new(PathBuf::from("journalctl")).is_err());
}

#[test]
fn the_production_probe_refuses_a_user_owned_or_writable_journalctl() {
    // A script in a temporary directory is owned by this user, not root.
    let (_dir, path) = script("exit 1");
    assert!(JournalGpuResetProbe::trusted(path).is_err());
    assert!(JournalGpuResetProbe::trusted(PathBuf::from("/nonexistent/journalctl")).is_err());
}

struct Fake(Arc<Mutex<anyhow::Result<ResetScan>>>);

impl GpuResetProbe for Fake {
    fn resets_since(&mut self, _: u64) -> anyhow::Result<ResetScan> {
        match &*self.0.lock().unwrap() {
            Ok(scan) => Ok(*scan),
            Err(error) => Err(anyhow::anyhow!("{error}")),
        }
    }
}

type Answer = Arc<Mutex<anyhow::Result<ResetScan>>>;

fn guard(dir: &Path, answer: &Answer) -> GpuGuard {
    GpuGuard::new(Box::new(Fake(answer.clone())), dir.to_path_buf())
}

fn answer(count: u32) -> Answer {
    Arc::new(Mutex::new(Ok(ResetScan {
        count,
        latest_unix: None,
    })))
}

fn incidents(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| incident_name(&path.file_name().unwrap().to_string_lossy()))
        .collect();
    found.sort();
    found
}

/// Acknowledge an incident the way a human does: its name and the hash of its bytes.
fn acknowledge(dir: &Path, incident: &Path) {
    let stem = incident.file_stem().unwrap().to_string_lossy().into_owned();
    let hash = stutter_core::hash::sha256_hex(&std::fs::read(incident).unwrap())[..16].to_string();
    let ack = dir.join(ACK_FILE);
    let mut text = std::fs::read_to_string(&ack).unwrap_or_default();
    text.push_str(&format!("{stem} {hash}\n"));
    std::fs::write(&ack, text).unwrap();
    std::fs::set_permissions(&ack, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn a_reset_is_a_durable_incident_that_time_and_restarts_never_clear() {
    let dir = tempfile::tempdir().unwrap();
    let ans = answer(2);
    let mut first = guard(dir.path(), &ans);
    assert!(!first.quiet_for_admission());
    assert_eq!(incidents(dir.path()).len(), 1);
    *ans.lock().unwrap() = Ok(ResetScan::quiet());
    first.admission = None;
    assert!(!first.quiet_for_admission());
    // A restarted controller over the same directory is blocked too.
    assert!(!guard(dir.path(), &ans).quiet_for_admission());
    assert!(guard(dir.path(), &ans).blocked());
}

#[test]
fn a_later_reset_after_an_acknowledgement_is_a_new_blocking_incident() {
    let dir = tempfile::tempdir().unwrap();
    let ans = Arc::new(Mutex::new(Ok(ResetScan {
        count: 1,
        latest_unix: Some(100),
    })));
    let mut guard1 = guard(dir.path(), &ans);
    assert!(!guard1.quiet_for_admission());
    acknowledge(dir.path(), &incidents(dir.path())[0]);
    // Reviewed: the journal still holds the old event but it is not counted again.
    *ans.lock().unwrap() = Ok(ResetScan::quiet());
    let mut guard2 = guard(dir.path(), &ans);
    assert!(!guard2.blocked());
    assert!(guard2.quiet_for_admission());
    // A new reset is persisted as its own incident despite the earlier record and ack.
    *ans.lock().unwrap() = Ok(ResetScan {
        count: 1,
        latest_unix: Some(5_000_000_000),
    });
    guard2.admission = None;
    assert!(!guard2.quiet_for_admission());
    assert_eq!(incidents(dir.path()).len(), 2);
    // Restart: still blocked; the first incident's ack does not cover the second.
    assert!(guard(dir.path(), &ans).blocked());
    // The newest incident's own acknowledgement clears exactly it.
    acknowledge(dir.path(), &incidents(dir.path())[1]);
    assert!(!guard(dir.path(), &ans).blocked());
}

#[test]
fn only_an_exact_owned_private_acknowledgement_of_that_incident_counts() {
    let dir = tempfile::tempdir().unwrap();
    let ans = answer(1);
    let mut g = guard(dir.path(), &ans);
    assert!(!g.quiet_for_admission());
    let incident = incidents(dir.path())[0].clone();
    // The old plain timestamp acknowledgement, whatever its value, proves nothing.
    std::fs::write(
        dir.path().join("gpu-stability-ack"),
        format!("{}\n", u64::MAX / 2),
    )
    .unwrap();
    std::fs::write(dir.path().join(ACK_FILE), format!("{}\n", u64::MAX / 2)).unwrap();
    assert!(guard(dir.path(), &ans).blocked());
    // The right line from a group-writable file is refused.
    let stem = incident.file_stem().unwrap().to_string_lossy().into_owned();
    let hash = stutter_core::hash::sha256_hex(&std::fs::read(&incident).unwrap())[..16].to_string();
    let ack = dir.path().join(ACK_FILE);
    std::fs::write(&ack, format!("{stem} {hash}\n")).unwrap();
    std::fs::set_permissions(&ack, std::fs::Permissions::from_mode(0o664)).unwrap();
    assert!(guard(dir.path(), &ans).blocked());
    // A wrong hash (an edited record) or another incident's name does not match.
    std::fs::set_permissions(&ack, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&ack, format!("{stem} {}\n", "0".repeat(16))).unwrap();
    assert!(guard(dir.path(), &ans).blocked());
    std::fs::write(&ack, format!("other {hash}\n")).unwrap();
    assert!(guard(dir.path(), &ans).blocked());
    // A symlink to a correct file is not an acknowledgement.
    let real = dir.path().join("real");
    std::fs::write(&real, format!("{stem} {hash}\n")).unwrap();
    std::fs::remove_file(&ack).unwrap();
    std::os::unix::fs::symlink(&real, &ack).unwrap();
    assert!(guard(dir.path(), &ans).blocked());
    std::fs::remove_file(&ack).unwrap();
    std::fs::write(&ack, format!("{stem} {hash}\n")).unwrap();
    std::fs::set_permissions(&ack, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!guard(dir.path(), &ans).blocked());
}

#[test]
fn a_torn_or_edited_incident_and_an_unreadable_directory_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let ans = answer(0);
    std::fs::write(dir.path().join(LEGACY_BLOCKED_FILE), b"{\"detected_u").unwrap();
    assert!(guard(dir.path(), &ans).blocked());
    // Even a human line for a torn record is refused only if hashes disagree; the intact legacy
    // record format is honoured and acknowledged by its content hash.
    std::fs::write(
        dir.path().join(LEGACY_BLOCKED_FILE),
        b"{\"detected_unix\":100}",
    )
    .unwrap();
    assert!(guard(dir.path(), &ans).blocked());
    acknowledge(dir.path(), &dir.path().join(LEGACY_BLOCKED_FILE));
    assert!(!guard(dir.path(), &ans).blocked());
    // Editing the acknowledged record invalidates its acknowledgement.
    std::fs::write(
        dir.path().join(LEGACY_BLOCKED_FILE),
        b"{\"detected_unix\":101}",
    )
    .unwrap();
    assert!(guard(dir.path(), &ans).blocked());
    // A missing state directory is unknown, so blocked.
    assert!(guard(&dir.path().join("missing"), &ans).blocked());
}

#[test]
fn a_failed_durable_write_latches_the_stop_for_the_process() {
    let dir = tempfile::tempdir().unwrap();
    let ans = answer(1);
    let state = dir.path().join("state");
    std::fs::create_dir(&state).unwrap();
    let mut g = guard(&state, &ans);
    // Make the directory unwritable (the real daemon runs as a user that could be denied).
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o500)).unwrap();
    let writable = std::fs::File::create(state.join("probe")).is_ok();
    if writable {
        // Running as a user the mode does not restrain: the failure cannot be produced here.
        std::fs::remove_file(state.join("probe")).unwrap();
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
        return;
    }
    assert!(!g.quiet_for_admission());
    *ans.lock().unwrap() = Ok(ResetScan::quiet());
    g.admission = None;
    assert!(g.blocked(), "the unwritten incident still blocks");
    assert!(!g.quiet_for_admission());
    assert!(g.reset_during_attempt());
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn a_reset_during_an_attempt_is_sticky_and_recorded_and_unknown_checks_are_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let ans = answer(0);
    let mut g = guard(dir.path(), &ans);
    g.attempt_started_unix = Some(100);
    assert!(!g.reset_during_attempt());
    *ans.lock().unwrap() = Ok(ResetScan {
        count: 3,
        latest_unix: Some(150),
    });
    g.last_poll = None;
    assert!(g.reset_during_attempt());
    assert_eq!(incidents(dir.path()).len(), 1);
    *ans.lock().unwrap() = Ok(ResetScan::quiet());
    assert!(g.reset_during_attempt(), "sticky");

    let dir2 = tempfile::tempdir().unwrap();
    let broken: Answer = Arc::new(Mutex::new(Err(anyhow::anyhow!("unreadable"))));
    let mut g2 = guard(dir2.path(), &broken);
    g2.attempt_started_unix = Some(100);
    for _ in 0..2 {
        assert!(!g2.reset_during_attempt());
        g2.last_poll = None;
    }
    assert!(g2.reset_during_attempt());
    assert_eq!(incidents(dir2.path()).len(), 1);
}

#[test]
fn an_unreadable_journal_withholds_admission_without_inventing_an_incident() {
    let dir = tempfile::tempdir().unwrap();
    let broken: Answer = Arc::new(Mutex::new(Err(anyhow::anyhow!("unreadable"))));
    assert!(!guard(dir.path(), &broken).quiet_for_admission());
    assert!(incidents(dir.path()).is_empty());
}

#[test]
fn the_uncached_admission_sees_a_reset_the_cache_would_have_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let ans = answer(0);
    let mut g = guard(dir.path(), &ans);
    assert!(g.quiet_for_admission());
    *ans.lock().unwrap() = Ok(ResetScan {
        count: 1,
        latest_unix: None,
    });
    assert!(g.quiet_for_admission(), "the cached answer is stale");
    assert!(!g.admit_now(), "the final pre-launch check is not");
    assert!(g.blocked());
}
