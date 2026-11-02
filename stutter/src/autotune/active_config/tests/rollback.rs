use super::support::*;
use crate::autotune::{
    active_config::verify_rollback_restored_baseline, observation::ActiveConfigSnapshot,
    planning::candidate::CandidateAction,
};

#[test]
fn rollback_verification_succeeds_when_active_config_returns_to_baseline() {
    let candidate = nice_candidate_for_rollback();
    let baseline = active_nice_snapshot(42, 0);
    let post_rollback = active_nice_snapshot(42, 0);

    let verification =
        verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

    assert!(verification.verified);
    assert_eq!(verification.reason_code, "rollback_verified");
}

#[test]
fn rollback_verification_faults_when_active_config_still_differs() {
    let candidate = nice_candidate_for_rollback();
    let baseline = active_nice_snapshot(42, 0);
    let post_rollback = active_nice_snapshot(42, 5);

    let verification =
        verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

    assert!(!verification.verified);
    assert_eq!(verification.reason_code, "rollback_state_mismatch");
    assert!(verification.expected.contains("tid=42 nice=0"));
    assert!(verification.actual.contains("tid=42 nice=5"));
}

#[test]
fn rollback_verification_faults_when_target_missing_after_rollback() {
    let candidate = nice_candidate_for_rollback();
    let baseline = active_nice_snapshot(42, 0);
    let post_rollback = ActiveConfigSnapshot::default();

    let verification =
        verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

    assert!(!verification.verified);
    assert_eq!(verification.reason_code, "rollback_target_missing");
    assert!(verification.actual.contains("missing"));
}

#[test]
fn rollback_verification_faults_when_verifier_is_unavailable() {
    let candidate = CandidateAction::fake(
        crate::actions::ActionId::new("fake-rollback".to_owned()),
        crate::actions::SafetyClass::ReversibleLowRisk,
    );
    let baseline = ActiveConfigSnapshot::default();
    let post_rollback = ActiveConfigSnapshot::default();

    let verification =
        verify_rollback_restored_baseline(&candidate, &baseline, &post_rollback, &[]);

    assert!(!verification.verified);
    assert_eq!(
        verification.reason_code,
        "rollback_verification_unavailable"
    );
}

fn cgroup_candidate(tid: u32) -> CandidateAction {
    use crate::{
        actions::{
            TaskIdentity,
            cgroup::{CgroupPlacementAction, CgroupPlacementTarget},
        },
        autotune::{
            objective::ObjectiveKind, planning::executable_plan::CgroupPlacementActionPlan,
        },
        process_tree::TaskClass,
    };
    CandidateAction::CgroupPlacement {
        plan: CgroupPlacementActionPlan {
            target_root_starttime_ticks: None,
            name: "exited-target".to_owned(),
            action: CgroupPlacementAction {
                cgroup_root: "/sys/fs/cgroup".into(),
                target_cgroup: "/sys/fs/cgroup/game.slice".into(),
                targets: vec![CgroupPlacementTarget {
                    identity: TaskIdentity {
                        tid: tid.into(),
                        process_pid: Some(tid.into()),
                        comm: Some("game".to_owned()),
                        starttime_ticks: None,
                    },
                    class: TaskClass::Game,
                }],
                cpuset_cpus: None,
                cpuset_mems: None,
            },
            target_root_pid: Some(tid),
            evidence: Vec::new(),
            objective: ObjectiveKind::GameRunnableLatency,
        },
    }
}

fn cgroup_snapshot(tid: u32, path: &str) -> ActiveConfigSnapshot {
    let mut snapshot = ActiveConfigSnapshot::default();
    snapshot.cgroup.per_tid.insert(tid, path.to_owned());
    snapshot
}

#[test]
fn a_cgroup_target_that_exited_during_the_action_needs_no_restoration() {
    // Beyond the kernel's pid_max, so it can never be a live thread.
    let gone = u32::MAX - 1;
    let verification = verify_rollback_restored_baseline(
        &cgroup_candidate(gone),
        &cgroup_snapshot(gone, "/suite"),
        &ActiveConfigSnapshot::default(),
        &[],
    );
    assert!(verification.verified, "{verification:?}");
}

#[test]
fn a_live_cgroup_target_missing_from_the_snapshot_still_faults() {
    let alive = std::process::id();
    let verification = verify_rollback_restored_baseline(
        &cgroup_candidate(alive),
        &cgroup_snapshot(alive, "/suite"),
        &ActiveConfigSnapshot::default(),
        &[],
    );
    assert!(!verification.verified);
    assert_eq!(verification.reason_code, "rollback_target_missing");
}

#[test]
fn an_unreaped_zombie_is_not_a_live_rollback_target() {
    use super::super::rollback::task_is_alive;
    let mut child = std::process::Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let pid = child.id();
    // Until it is waited for, the exited child keeps its /proc entry as a zombie.
    for _ in 0..100 {
        if !task_is_alive(pid) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!task_is_alive(pid));
    child.wait().unwrap();
    assert!(!task_is_alive(pid));
    assert!(task_is_alive(std::process::id()));
}
