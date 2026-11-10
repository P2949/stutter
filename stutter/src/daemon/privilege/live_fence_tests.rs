//! Live qualification of the validation fence against a real, isolated, transient root worker.
//!
//! Ignored by default: it needs a worker started by the qualification procedure (see
//! `docs/validation/worker-fence-qualification-2026-11-09/RESULTS.md`) and two disposable processes
//! of the same account: the fixture the fence names and a decoy it does not. It only sends the
//! worker read-only requests and dry runs, so nothing is mutated.
use super::*;
use crate::autotune::{objective::ObjectiveKind, planning::candidate::CandidateAction};

fn sleeper() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("600")
        .spawn()
        .expect("a disposable sleeper")
}

fn allowed(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .expect("status")
        .lines()
        .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
        .expect("Cpus_allowed_list")
        .trim()
        .to_owned()
}

/// Writes the fence into the transient worker's root-owned state directory, as root, the way the
/// smoke does.
fn fence(home: &str, pid: u32, seconds_from_now: i64) {
    use std::io::Write;
    let ticks = crate::watch::process_root_starttime(pid).expect("start ticks");
    let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").expect("boot id");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let expires = now.saturating_add_signed(seconds_from_now);
    let body = format!(
        r#"{{"version":1,"uid":{},"root_pid":{pid},"start_ticks":{ticks},"boot_id":"{}","expires_unix":{expires}}}"#,
        crate::syscall::geteuid(),
        boot.trim()
    );
    let path = format!("{home}/validation-fence.json");
    let mut tee = std::process::Command::new("sudo")
        .args(["-n", "tee", &path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("sudo tee");
    tee.stdin
        .take()
        .expect("stdin")
        .write_all(body.as_bytes())
        .expect("write");
    assert!(tee.wait().expect("tee").success());
    assert!(
        std::process::Command::new("sudo")
            .args(["-n", "chmod", "0644", &path])
            .status()
            .expect("chmod")
            .success()
    );
}

fn affinity_candidate(pid: u32) -> CandidateAction {
    CandidateAction::cpu_affinity_profile_for(
        crate::profiles::Profile {
            name: "live-fence".into(),
            rules: vec![crate::profiles::ProfileRule {
                affinity: Some(crate::affinity::CpuMask::parse("0").expect("mask")),
                nice: None,
                ionice: None,
                match_class: vec![],
                match_comm: vec![
                    crate::process::model::CompiledPattern::new("sleep".into()).expect("pattern"),
                ],
            }],
        },
        pid,
        ObjectiveKind::GameRunnableLatency,
    )
}

fn apply_request(candidate: CandidateAction) -> CandidateApplyRequest {
    CandidateApplyRequest {
        plan: CandidatePlanRequest::from_candidate(candidate, crate::audit::unix_nanos_now()),
        policy: crate::daemon_policy::DaemonPolicy::apply_medium_risk(
            crate::daemon_policy::ActionSource::Test,
        ),
        context: Default::default(),
        max_plan_age_nanos: 60_000_000_000,
    }
}

/// The full two-phase exchange against a real root worker: a decoy is refused at prepare and never
/// touched, the fenced fixture is applied and restored, and a fence that moves between prepare and
/// commit stops the first mutable write.
#[test]
#[ignore = "live: needs STUTTER_QUALIFY_SOCKET and STUTTER_QUALIFY_HOME of a transient root worker"]
fn a_real_worker_applies_restores_and_refuses_at_precommit_under_the_fence() {
    let socket = std::env::var("STUTTER_QUALIFY_SOCKET").expect("STUTTER_QUALIFY_SOCKET");
    let home = std::env::var("STUTTER_QUALIFY_HOME").expect("STUTTER_QUALIFY_HOME");
    let client = UnixSocketPrivilegedActionService::new(socket);
    let (mut fixture, mut decoy) = (sleeper(), sleeper());
    let (fixture_pid, decoy_pid) = (fixture.id(), decoy.id());
    let (fixture_before, decoy_before) = (allowed(fixture_pid), allowed(decoy_pid));
    assert_ne!(
        fixture_before, "0",
        "the fixture must have somewhere to move from"
    );
    fence(&home, fixture_pid, 300);

    // Prepare: the decoy is refused, its hook never runs, nothing about it changes.
    let mut hooked = false;
    let refused = client
        .apply_candidate_prepared(apply_request(affinity_candidate(decoy_pid)), &mut |_| {
            hooked = true;
            Ok(())
        })
        .expect_err("the decoy must be refused at prepare");
    assert!(
        format!("{refused:#}").contains("validation_fence_target_not_fenced"),
        "{refused:#}"
    );
    assert!(!hooked && allowed(decoy_pid) == decoy_before);

    // The fenced fixture is prepared, committed and really changed, then restored.
    let candidate = affinity_candidate(fixture_pid);
    let applied = client
        .apply_candidate_prepared(apply_request(candidate.clone()), &mut |_| Ok(()))
        .expect("the fenced fixture applies");
    assert_eq!(
        allowed(fixture_pid),
        "0",
        "the placement was read back as changed"
    );
    client
        .rollback(RollbackRequest {
            candidate: candidate.clone(),
            token: applied.rollback,
            policy: crate::daemon_policy::DaemonPolicy::apply_medium_risk(
                crate::daemon_policy::ActionSource::Test,
            ),
            context: Default::default(),
        })
        .expect("the rollback succeeds");
    assert_eq!(allowed(fixture_pid), fixture_before, "restored exactly");
    assert_eq!(
        allowed(decoy_pid),
        decoy_before,
        "the decoy was never touched"
    );

    // Precommit: the fence moves to the decoy after the prepare. The commit must not mutate.
    let moved = client.apply_candidate_prepared(apply_request(candidate), &mut |_| {
        fence(&home, decoy_pid, 300);
        Ok(())
    });
    assert!(
        moved.is_err(),
        "a fence that moved must stop the first mutable write"
    );
    assert_eq!(allowed(fixture_pid), fixture_before, "nothing was written");
    assert_eq!(
        client.settle_issued_tokens().expect("settle"),
        0,
        "no token left outstanding"
    );
    // Expiry between prepare and commit: an expired fence refuses every mutation, so the commit stops.
    fence(&home, fixture_pid, 300);
    let expired = client.apply_candidate_prepared(
        apply_request(affinity_candidate(fixture_pid)),
        &mut |_| {
            fence(&home, fixture_pid, -5);
            Ok(())
        },
    );
    assert!(
        expired.is_err(),
        "a fence that expired mid-transaction must stop the commit"
    );
    assert_eq!(allowed(fixture_pid), fixture_before, "nothing was written");
    assert_eq!(
        client.settle_issued_tokens().expect("settle"),
        0,
        "no token left outstanding"
    );
    let _ = (fixture.kill(), decoy.kill());
    let _ = (fixture.wait(), decoy.wait());
}

fn env_pid(name: &str) -> u32 {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set"))
        .parse()
        .unwrap_or_else(|_| panic!("{name} is not a number"))
}

/// Phase one of the restart qualification: apply to the fenced fixture and leave the token
/// outstanding, as a controller that died right after the commit would.
#[test]
#[ignore = "live: phase one of the worker-restart qualification (see the RESULTS.md procedure)"]
fn live_phase_apply_and_leave_the_token_outstanding() {
    let client = UnixSocketPrivilegedActionService::new(
        std::env::var("STUTTER_QUALIFY_SOCKET").expect("STUTTER_QUALIFY_SOCKET"),
    );
    let pid = env_pid("STUTTER_QUALIFY_FIXTURE_PID");
    client
        .apply_candidate_prepared(apply_request(affinity_candidate(pid)), &mut |_| Ok(()))
        .expect("the fenced fixture applies");
    assert_eq!(
        allowed(pid),
        "0",
        "the placement is changed and its token is outstanding"
    );
}

/// Phase two: after the worker was killed with SIGKILL and restarted, its startup recovery must
/// have restored the fixture, kept the fence, and left nothing outstanding.
#[test]
#[ignore = "live: phase two of the worker-restart qualification (see the RESULTS.md procedure)"]
fn live_phase_after_restart_the_fixture_is_restored_under_the_same_fence() {
    let client = UnixSocketPrivilegedActionService::new(
        std::env::var("STUTTER_QUALIFY_SOCKET").expect("STUTTER_QUALIFY_SOCKET"),
    );
    let (fixture, decoy) = (
        env_pid("STUTTER_QUALIFY_FIXTURE_PID"),
        env_pid("STUTTER_QUALIFY_DECOY_PID"),
    );
    let before = std::env::var("STUTTER_QUALIFY_FIXTURE_BEFORE").expect("FIXTURE_BEFORE");
    assert_eq!(
        allowed(fixture),
        before,
        "startup recovery restored the fixture exactly"
    );
    let report = client.fence_status().expect("the restarted worker answers");
    assert_eq!(
        (report.state.as_str(), report.root_pid),
        ("active", Some(fixture))
    );
    let refused = client
        .apply_candidate_prepared(apply_request(affinity_candidate(decoy)), &mut |_| Ok(()))
        .expect_err("the decoy is still refused after the restart");
    assert!(
        format!("{refused:#}").contains("validation_fence_target_not_fenced"),
        "{refused:#}"
    );
    assert_eq!(
        client.settle_issued_tokens().expect("settle"),
        0,
        "nothing left outstanding"
    );
}
