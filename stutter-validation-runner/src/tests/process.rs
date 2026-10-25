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
