use std::fs;

use super::{Scratch, authorization, envelope};
use crate::{
    model::{ErrorCode, LiveSuite, Request},
    process::{plan_live_suite, plan_scheduler_burst, stutter_path, verify_pinned, xtask_path},
};

fn suite(suite: LiveSuite, seconds: Option<u64>, rounds: Option<u32>) -> Request {
    Request::ExecuteLiveSuite {
        suite,
        attempt: 3,
        seconds,
        rounds,
        benefit: None,
        mutation: None,
    }
}

#[test]
fn paths_derive_from_the_authorization_never_from_the_request() {
    let auth = authorization("/work/stutter");
    let plan = plan_live_suite(
        &auth,
        &envelope(
            "live-observation",
            suite(LiveSuite::Observation, Some(20), None),
        ),
    )
    .unwrap();
    assert_eq!(plan.program, xtask_path(&auth));
    assert_eq!(
        plan.program,
        std::path::Path::new("/work/stutter/target/debug/xtask")
    );
    assert_eq!(
        plan.args,
        [
            "live-acceptance",
            "observation",
            "--stutter-bin",
            "/work/stutter/target/debug/stutter",
            "--out",
            "/work/stutter/.llm-runs/validation/core-abc1234-1/attempts/live-observation/0003/raw",
            "--seconds",
            "20",
        ]
    );
    assert_eq!(plan.cwd, std::path::Path::new("/work/stutter"));
    assert_eq!(plan.path_env, "/usr/bin:/bin");
    assert_eq!(
        stutter_path(&auth),
        std::path::Path::new("/work/stutter/target/debug/stutter")
    );
}

#[test]
fn each_suite_gets_only_the_arguments_it_takes() {
    let auth = authorization("/r");
    let args = |s, seconds, rounds| {
        plan_live_suite(&auth, &envelope("t", suite(s, seconds, rounds)))
            .unwrap()
            .args
    };
    assert!(
        args(LiveSuite::Overhead, Some(20), Some(12))
            .windows(2)
            .any(|w| w == ["--rounds", "12"])
    );
    assert!(
        !args(LiveSuite::KernelCompat, None, None)
            .iter()
            .any(|a| a == "--seconds")
    );
    assert!(args(LiveSuite::KernelCompat, None, None).contains(&"kernel-compat".to_owned()));
    assert!(
        LiveSuite::GameRunnable.needs_scheduler_priority()
            && LiveSuite::ScoreDegradation.needs_scheduler_priority()
    );
    assert!(!LiveSuite::Observation.needs_scheduler_priority());
}

#[test]
fn the_burst_plan_is_a_typed_pinned_helper_invocation() {
    let auth = authorization("/r");
    let body = Request::ExecuteSchedulerBurst {
        cpu: 2,
        burst_ms: 4,
        period_ms: 20,
        seconds: 30,
    };
    let plan = plan_scheduler_burst(&auth, &envelope("score-runnable", body)).unwrap();
    assert_eq!(plan.program, xtask_path(&auth));
    assert_eq!(
        plan.args,
        [
            "live-burst",
            "--burst-ms",
            "4",
            "--period-ms",
            "20",
            "--seconds",
            "30"
        ]
    );
    assert!(plan_scheduler_burst(&auth, &envelope("score-runnable", Request::Status)).is_err());
    assert!(plan_live_suite(&auth, &envelope("score-runnable", Request::Status)).is_err());
}

#[test]
fn a_modified_binary_is_never_executed() {
    let scratch = Scratch::new("pinned");
    let path = scratch.path().join("xtask");
    fs::write(&path, b"original").unwrap();
    let digest = stutter_core::hash::sha256_hex(b"original");
    verify_pinned(&path, &digest).unwrap();
    fs::write(&path, b"tampered").unwrap();
    let (code, message) = verify_pinned(&path, &digest).unwrap_err();
    assert_eq!(code, ErrorCode::ExecutionFailed);
    assert!(message.contains("pinned digest"), "{message}");
    assert!(verify_pinned(&scratch.path().join("absent"), &digest).is_err());
}

#[test]
fn ending_a_run_ends_its_whole_process_group() {
    use std::{os::unix::process::CommandExt, process::Command, time::Duration};

    use crate::{process::reap_group, sys};

    // A child that ignores TERM and a grandchild that does too, in the child's own group.
    let mut child = Command::new("sh")
        .args(["-c", "trap '' TERM; (trap '' TERM; sleep 600) & sleep 600"])
        .process_group(0)
        .spawn()
        .unwrap();
    let group = child.id();
    let mut sentinel = Command::new("sleep").arg("600").spawn().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(
        sys::signal_group(group, 0).is_ok(),
        "the group should exist"
    );
    reap_group(&mut child, Duration::from_millis(300));
    assert!(
        sys::signal_group(group, 0).is_err(),
        "a group member survived"
    );
    assert!(
        sentinel.try_wait().unwrap().is_none(),
        "an unrelated process was killed"
    );
    sentinel.kill().unwrap();
    sentinel.wait().unwrap();
    assert!(
        sys::signal_group(1, 0).is_err(),
        "pid 1 must never be addressed"
    );
}

#[test]
#[ignore = "live disposable kernel cgroup; requires caller-owned delegation"]
fn live_action_end_kills_a_detached_descendant_and_spares_an_unrelated_process() {
    use std::{process::Command, time::Duration};

    let mut cgroup = crate::containment::OwnedCgroup::prepare()
        .expect("BlockedResource: caller-owned cgroup required");
    // The child starts a new session, ignores TERM and clears nothing it needs: a process group
    // cannot reach it, the owned cgroup can.
    let mut command = Command::new("sh");
    command.args([
        "-c",
        "setsid sh -c 'trap \"\" TERM; exec sleep 600' & sleep 600",
    ]);
    cgroup.attach(&mut command).unwrap();
    let mut child = command.spawn().unwrap();
    let mut sentinel = Command::new("sleep").arg("600").spawn().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(cgroup.populated().unwrap());
    let started = std::time::Instant::now();
    crate::process::contain_end_for_test(&mut cgroup, &mut child).unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    // `finish` kills, verifies the group empty and removes it: the group being gone is the proof.
    assert!(
        cgroup.populated().is_err(),
        "the owned cgroup was not removed"
    );
    assert!(
        sentinel.try_wait().unwrap().is_none(),
        "an unrelated process was killed"
    );
    sentinel.kill().unwrap();
    sentinel.wait().unwrap();
}
