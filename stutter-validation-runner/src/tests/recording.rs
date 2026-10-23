use std::{fs, path::Path};

use super::{Scratch, authorization, envelope, grant, me, peer};
use crate::{
    model::{ErrorCode, RecordProbes, RegisteredWorkload, Request},
    process::{plan_recording, plan_workload_overhead, stutter_path, verify_target, xtask_path},
};

fn recording(probes: RecordProbes) -> Request {
    Request::ExecuteRecording {
        attempt: 2,
        capture: 7,
        target_pid: 4242,
        target_starttime_ticks: 777,
        seconds: 30,
        probes,
    }
}

fn authorized() -> crate::model::Authorization {
    let mut auth = authorization("/work/stutter");
    auth.grants.insert(grant(
        "corpus",
        crate::model::RunnerAction::ExecuteRecording,
        None,
    ));
    auth.grants.insert(grant(
        "overhead-real",
        crate::model::RunnerAction::ExecuteWorkloadOverhead,
        None,
    ));
    auth
}

#[test]
fn recording_arguments_derive_from_typed_fields_and_the_authorization_only() {
    let auth = authorized();
    let plan = plan_recording(
        &auth,
        &envelope("corpus", recording(RecordProbes::Corpus { irq: Some(31) })),
        Some(Path::new("/m/log.csv")),
    )
    .unwrap();
    assert_eq!(plan.program, stutter_path(&auth));
    assert_eq!(
        plan.args,
        [
            "record",
            "--tree-pid",
            "4242",
            "--duration",
            "30",
            "--summary-ms",
            "250",
            "--out-dir",
            "/work/stutter/.llm-runs/validation/core-abc1234-1/attempts/corpus/0002/captures/0007/raw",
            "--hwmon",
            "--block-io",
            "--no-cpu-freq",
            "--no-faults",
            "--no-stat-wait",
            "--irq-latency",
            "--irq",
            "31",
            "--mangohud-log",
            "/m/log.csv",
        ]
    );
    let core = plan_recording(
        &auth,
        &envelope("corpus", recording(RecordProbes::Core)),
        None,
    )
    .unwrap();
    assert!(
        core.args.contains(&"--no-hwmon".to_owned())
            && !core.args.contains(&"--mangohud-log".to_owned())
    );
    let no_irq = plan_recording(
        &auth,
        &envelope("corpus", recording(RecordProbes::Corpus { irq: None })),
        None,
    )
    .unwrap();
    assert!(!no_irq.args.contains(&"--irq-latency".to_owned()));
}

#[test]
fn recording_requests_are_authorized_per_task_and_bounded() {
    let auth = authorized();
    let ok = |task, body| auth.authorize(&peer(me()), &envelope(task, body), 8);
    ok("corpus", recording(RecordProbes::Core)).unwrap();
    assert_eq!(
        ok("live-observation", recording(RecordProbes::Core))
            .unwrap_err()
            .0,
        ErrorCode::TaskNotPermitted
    );
    for bad in [
        Request::ExecuteRecording {
            attempt: 0,
            capture: 0,
            target_pid: 4242,
            target_starttime_ticks: 1,
            seconds: 30,
            probes: RecordProbes::Core,
        },
        Request::ExecuteRecording {
            attempt: 1,
            capture: 0,
            target_pid: 1,
            target_starttime_ticks: 1,
            seconds: 30,
            probes: RecordProbes::Core,
        },
        Request::ExecuteRecording {
            attempt: 1,
            capture: 0,
            target_pid: 9,
            target_starttime_ticks: 0,
            seconds: 30,
            probes: RecordProbes::Core,
        },
        Request::ExecuteRecording {
            attempt: 1,
            capture: 0,
            target_pid: 9,
            target_starttime_ticks: 1,
            seconds: 3,
            probes: RecordProbes::Core,
        },
        Request::ExecuteRecording {
            attempt: 1,
            capture: 0,
            target_pid: 9,
            target_starttime_ticks: 1,
            seconds: 30,
            probes: RecordProbes::Corpus { irq: Some(99_999) },
        },
    ] {
        assert_eq!(ok("corpus", bad).unwrap_err().0, ErrorCode::InvalidArgument);
    }
}

fn fake_proc(
    scratch: &Scratch,
    pid: u32,
    comm: &str,
    starttime: u64,
    uid: u32,
    euid: u32,
) -> std::path::PathBuf {
    let root = scratch.path().join("proc");
    let dir = root.join(pid.to_string());
    fs::create_dir_all(&dir).unwrap();
    // Fields 4..21 are filler; field 22 is the start time.
    let filler = (4..22).map(|_| "0").collect::<Vec<_>>().join(" ");
    fs::write(
        dir.join("stat"),
        format!("{pid} ({comm}) S {filler} {starttime} 0 0\n"),
    )
    .unwrap();
    fs::write(
        dir.join("status"),
        format!("Name:\tx\nUid:\t{uid}\t{euid}\t{uid}\t{uid}\n"),
    )
    .unwrap();
    root
}

#[test]
fn a_recording_target_must_be_the_campaign_users_unreplaced_process() {
    let scratch = Scratch::new("verify-target");
    let root = fake_proc(&scratch, 4242, "vkmark", 777, 1000, 1000);
    verify_target(&root, 4242, 777, 1000).unwrap();
    assert!(
        verify_target(&root, 4242, 778, 1000)
            .unwrap_err()
            .1
            .contains("replaced")
    );
    assert!(
        verify_target(&root, 4242, 777, 1001)
            .unwrap_err()
            .1
            .contains("not owned")
    );
    assert!(
        verify_target(&root, 9999, 1, 1000)
            .unwrap_err()
            .1
            .contains("does not exist")
    );
    // A different effective uid (e.g. a setuid helper) is refused even if the real uid matches.
    let setuid_scratch = Scratch::new("verify-setuid");
    let setuid = fake_proc(&setuid_scratch, 55, "helper", 1, 1000, 0);
    assert!(verify_target(&setuid, 55, 1, 1000).is_err());
    // A comm containing spaces and parentheses cannot shift the parsed start time.
    let odd_scratch = Scratch::new("verify-comm");
    let odd = fake_proc(&odd_scratch, 56, "a) S 1 (b", 888, 1000, 1000);
    verify_target(&odd, 56, 888, 1000).unwrap();
}

#[test]
fn the_registered_workload_command_is_derived_by_the_runner() {
    let auth = authorized();
    let body = Request::ExecuteWorkloadOverhead {
        workload: RegisteredWorkload::FixedCompile,
        attempt: 1,
        seconds: 20,
        rounds: 12,
        work_units: 3,
    };
    auth.authorize(&peer(me()), &envelope("overhead-real", body.clone()), 8)
        .unwrap();
    let plan = plan_workload_overhead(&auth, &envelope("overhead-real", body)).unwrap();
    assert_eq!(plan.program, xtask_path(&auth));
    let tail = plan.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(
        plan.args[tail + 1..],
        [
            "/work/stutter/target/debug/xtask",
            "validation-campaign",
            "workload",
            "fixed-compile",
            "--cycles",
            "3",
            "--target-dir",
            "/work/stutter/.llm-runs/validation/core-abc1234-1/workloads/fixed-compile-target",
        ]
    );
    assert!(
        plan.args
            .windows(2)
            .any(|w| w == ["--workload-label", "fixed-compile-v1"])
    );
    assert!(plan.args.windows(2).any(|w| w == ["--work-units", "3"]));
    for bad in [
        Request::ExecuteWorkloadOverhead {
            workload: RegisteredWorkload::FixedCompile,
            attempt: 1,
            seconds: 5,
            rounds: 12,
            work_units: 3,
        },
        Request::ExecuteWorkloadOverhead {
            workload: RegisteredWorkload::FixedCompile,
            attempt: 1,
            seconds: 20,
            rounds: 2,
            work_units: 3,
        },
        Request::ExecuteWorkloadOverhead {
            workload: RegisteredWorkload::FixedCompile,
            attempt: 1,
            seconds: 20,
            rounds: 12,
            work_units: 0,
        },
    ] {
        assert!(
            auth.authorize(&peer(me()), &envelope("overhead-real", bad), 8)
                .is_err()
        );
    }
    // A different task may not run it.
    let other = Request::ExecuteWorkloadOverhead {
        workload: RegisteredWorkload::FixedCompile,
        attempt: 1,
        seconds: 20,
        rounds: 12,
        work_units: 3,
    };
    assert!(
        auth.authorize(&peer(me()), &envelope("corpus", other), 8)
            .is_err()
    );
}

#[test]
fn the_default_executor_refuses_actions_it_does_not_provide() {
    use crate::process::ActionExecutor;
    struct Minimal;
    impl ActionExecutor for Minimal {
        fn live_suite(
            &self,
            _: &crate::model::Authorization,
            _: &crate::model::Envelope,
        ) -> Result<serde_json::Value, (ErrorCode, String)> {
            Err((ErrorCode::Unsupported, String::new()))
        }
        fn scheduler_burst(
            &self,
            _: &crate::model::Authorization,
            _: &crate::model::Envelope,
        ) -> Result<serde_json::Value, (ErrorCode, String)> {
            Err((ErrorCode::Unsupported, String::new()))
        }
    }
    let auth = authorized();
    let result = Minimal.recording(&auth, &envelope("corpus", recording(RecordProbes::Core)));
    assert_eq!(result.unwrap_err().0, ErrorCode::Unsupported);
}
