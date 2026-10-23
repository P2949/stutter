use stutter_validation_runner::model::{LiveSuite, RunnerAction};

use super::{Scratch, fresh, manifest};
use crate::validation_campaign::{
    bootstrap::{CAMPAIGN_TASK, PrivilegeMode, build_authorization, detect_privilege, grants},
    graph::Graph,
};

#[test]
fn privilege_detection_prefers_root_then_noninteractive_sudo_and_never_prompts() {
    assert_eq!(
        detect_privilege(true, || panic!("sudo must not be probed as root")),
        PrivilegeMode::Root
    );
    assert_eq!(detect_privilege(false, || true), PrivilegeMode::Sudo);
    assert_eq!(detect_privilege(false, || false), PrivilegeMode::None);
}

#[test]
fn the_grant_table_is_a_small_closed_set_over_real_tasks() {
    let graph = Graph::core().unwrap();
    let grants = grants();
    for grant in &grants {
        if grant.task_id != CAMPAIGN_TASK {
            assert!(
                graph.get(&grant.task_id).is_some(),
                "{} is not a campaign task",
                grant.task_id
            );
        }
        assert_eq!(
            grant.action == RunnerAction::ExecuteLiveSuite,
            grant.suite.is_some(),
            "{grant:?}: only live-suite grants name a suite"
        );
    }
    // No grant ever reaches the release, tag or policy tasks.
    for task in [
        "tag",
        "release-package",
        "release-verify",
        "capability-policy",
        "freeze-rc",
    ] {
        assert!(
            grants.iter().all(|g| g.task_id != task),
            "{task} must have no runner grant"
        );
    }
    let has = |task: &str, action, suite| {
        grants
            .iter()
            .any(|g| g.task_id == task && g.action == action && g.suite == suite)
    };
    assert!(has(
        "live-observation",
        RunnerAction::ExecuteLiveSuite,
        Some(LiveSuite::Observation)
    ));
    assert!(!has(
        "live-observation",
        RunnerAction::ExecuteLiveSuite,
        Some(LiveSuite::Overhead)
    ));
    assert!(has("kernel-matrix", RunnerAction::Reboot, None));
    assert!(
        !has("corpus", RunnerAction::Reboot, None),
        "only the kernel stage may reboot"
    );
    assert!(has(CAMPAIGN_TASK, RunnerAction::Status, None));
}

#[test]
fn the_authorization_pins_the_campaign_and_validates() {
    let scratch = Scratch::new("bootstrap-auth");
    let (_, mut state, _) = fresh(&scratch);
    let manifest = manifest();
    let auth = build_authorization(&state, &manifest, &"d".repeat(64), "/usr/bin:/bin");
    auth.validate_shape()
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(auth.campaign_id, state.campaign_id);
    assert_eq!(auth.xtask_sha256, state.xtask_sha256);
    assert_eq!(auth.stutter_sha256, state.stutter_sha256);
    assert_eq!(auth.manifest_sha256, state.manifest_sha256);
    assert_eq!(auth.runner_sha256, "d".repeat(64));
    assert_eq!(auth.max_reboots as usize, manifest.kernels.max_reboots);
    assert!(
        auth.allowed_boot_entries.is_empty(),
        "no boot entry is authorized before discovery"
    );
    // A campaign owned by root can never be authorized.
    state.uid = 0;
    assert!(
        build_authorization(&state, &manifest, &"d".repeat(64), "/bin")
            .validate_shape()
            .is_err()
    );
}

#[test]
fn only_the_driver_tasks_may_record_burst_or_run_the_registered_workload() {
    let grants = grants();
    let tasks = |action| {
        let mut tasks = grants
            .iter()
            .filter(|g| g.action == action)
            .map(|g| g.task_id.as_str())
            .collect::<Vec<_>>();
        tasks.sort_unstable();
        tasks
    };
    assert_eq!(
        tasks(RunnerAction::ExecuteRecording),
        ["corpus", "frame-score"]
    );
    assert_eq!(
        tasks(RunnerAction::ExecuteSchedulerBurst),
        ["corpus", "frame-score"]
    );
    assert_eq!(
        tasks(RunnerAction::ExecuteWorkloadOverhead),
        ["overhead-real"]
    );
}
