use std::fs;

use super::support::*;
use crate::{
    affinity::CpuMask,
    autotune::{
        active_config::{
            ActiveConfigCollectorInput, ActiveConfigMatch, ActiveConfigMatchInput,
            candidate_is_noop, candidate_is_noop_with_tasks, collect_active_config,
        },
        candidate::CandidateAction,
        observation::ActiveConfigSnapshot,
    },
    daemon::capabilities::DaemonCapabilities,
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
    system_inventory::{SystemInventory, SystemInventoryRoot},
    test_support::TestRoot,
};

#[test]
fn candidate_active_config_match_reports_difference_for_task_actions() {
    let root = TestRoot::new("active-config-diff");
    let proc_root = root.join("proc");
    let sys_root = root.join("sys");
    fs::create_dir_all(&proc_root).unwrap();
    fs::create_dir_all(&sys_root).unwrap();

    write_task_fixture(
        &proc_root,
        TaskFixture {
            tid: 77,
            comm: "worker",
            cpus_allowed_list: "2",
            nice: 0,
            ioprio: "idle",
            uclamp_min: 64,
            uclamp_max: 512,
            cgroup: "/compile.slice",
        },
    );

    let inventory = SystemInventory::probe_root(&SystemInventoryRoot {
        proc_root: proc_root.clone(),
        sys_root: sys_root.clone(),
    });
    let capabilities = DaemonCapabilities {
        ionice_available: true,
        uclamp_available: true,
        ..DaemonCapabilities::default()
    };
    let active_tasks = vec![active_task(77)];
    let snapshot = collect_active_config(ActiveConfigCollectorInput {
        proc_root: &proc_root,
        sys_root: &sys_root,
        active_tasks: &active_tasks,
        capabilities: &capabilities,
        inventory: &inventory,
    });

    let candidate = CandidateAction::Nice {
        plan: crate::autotune::candidate::NiceActionPlan {
            name: "nice-diff".to_owned(),
            action: crate::actions::nice::NiceAction {
                targets: vec![crate::actions::TaskIdentity {
                    tid: 77,
                    process_pid: Some(77),
                    comm: Some("worker".to_owned()),
                    starttime_ticks: Some(1),
                }],
                nice: 10,
                policy: crate::actions::nice::NicePolicy::default(),
            },
            target_root_pid: Some(77),
            evidence: Vec::new(),
            objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
        },
    };

    let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
        snapshot: &snapshot,
        active_tasks: &active_tasks,
    });

    assert!(active_match.is_differs());
    assert!(matches!(active_match, ActiveConfigMatch::Differs { .. }));
    assert!(!candidate_is_noop(&candidate, &snapshot));
}

#[test]
fn candidate_noop_helper_matches_typed_snapshot_for_task_actions() {
    let root = TestRoot::new("active-config-noop");
    let proc_root = root.join("proc");
    let sys_root = root.join("sys");
    fs::create_dir_all(&proc_root).unwrap();
    fs::create_dir_all(&sys_root).unwrap();

    write_task_fixture(
        &proc_root,
        TaskFixture {
            tid: 77,
            comm: "worker",
            cpus_allowed_list: "2",
            nice: 10,
            ioprio: "idle",
            uclamp_min: 64,
            uclamp_max: 512,
            cgroup: "/compile.slice",
        },
    );

    let inventory = SystemInventory::probe_root(&SystemInventoryRoot {
        proc_root: proc_root.clone(),
        sys_root: sys_root.clone(),
    });
    let capabilities = DaemonCapabilities {
        ionice_available: true,
        uclamp_available: true,
        ..DaemonCapabilities::default()
    };
    let active_tasks = vec![active_task(77)];
    let snapshot = collect_active_config(ActiveConfigCollectorInput {
        proc_root: &proc_root,
        sys_root: &sys_root,
        active_tasks: &active_tasks,
        capabilities: &capabilities,
        inventory: &inventory,
    });

    let candidate = CandidateAction::Nice {
        plan: crate::autotune::candidate::NiceActionPlan {
            name: "nice-noop".to_owned(),
            action: crate::actions::nice::NiceAction {
                targets: vec![crate::actions::TaskIdentity {
                    tid: 77,
                    process_pid: Some(77),
                    comm: Some("worker".to_owned()),
                    starttime_ticks: Some(1),
                }],
                nice: 10,
                policy: crate::actions::nice::NicePolicy::default(),
            },
            target_root_pid: Some(77),
            evidence: Vec::new(),
            objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
        },
    };

    assert!(candidate_is_noop(&candidate, &snapshot));
}

#[test]
fn cpu_affinity_profile_match_reports_exact_match() {
    let profile = profile_for_class(TaskClass::Game, "0-1");
    let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
    let snapshot = affinity_snapshot([(11, "0-1")]);
    let active_tasks = vec![active_task_with_class(11, TaskClass::Game)];

    let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
        snapshot: &snapshot,
        active_tasks: &active_tasks,
    });

    assert!(active_match.is_match());
    assert!(candidate_is_noop_with_tasks(
        &candidate,
        &snapshot,
        &active_tasks
    ));
}

#[test]
fn cpu_affinity_profile_match_reports_one_differing_tid() {
    let profile = profile_for_class(TaskClass::Game, "0-1");
    let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
    let snapshot = affinity_snapshot([(11, "2")]);
    let active_tasks = vec![active_task_with_class(11, TaskClass::Game)];

    let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
        snapshot: &snapshot,
        active_tasks: &active_tasks,
    });

    assert!(matches!(active_match, ActiveConfigMatch::Differs { .. }));
}

#[test]
fn cpu_affinity_profile_match_is_unknown_when_affinity_data_missing() {
    let profile = profile_for_class(TaskClass::Game, "0-1");
    let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
    let active_tasks = vec![active_task_with_class(11, TaskClass::Game)];

    let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
        snapshot: &ActiveConfigSnapshot::default(),
        active_tasks: &active_tasks,
    });

    assert!(matches!(active_match, ActiveConfigMatch::Unknown { .. }));
}

#[test]
fn cpu_affinity_profile_match_is_unknown_when_no_tasks_match_rules() {
    let profile = profile_for_class(TaskClass::Game, "0-1");
    let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
    let snapshot = affinity_snapshot([(11, "0-1")]);
    let active_tasks = vec![active_task_with_class(11, TaskClass::Service)];

    let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
        snapshot: &snapshot,
        active_tasks: &active_tasks,
    });

    assert!(matches!(active_match, ActiveConfigMatch::Unknown { .. }));
}

#[test]
fn cpu_affinity_profile_match_does_not_target_protected_tasks_unless_rule_matches() {
    let profile = profile_for_class(TaskClass::Game, "0-1");
    let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
    let snapshot = affinity_snapshot([(11, "0-1")]);
    let active_tasks = vec![active_task_with_class(11, TaskClass::Compositor)];

    let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
        snapshot: &snapshot,
        active_tasks: &active_tasks,
    });

    assert!(matches!(active_match, ActiveConfigMatch::Unknown { .. }));
}

#[test]
fn cpu_affinity_profile_match_supports_broad_fallback_rules() {
    let profile = Profile {
        name: "fallback".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("3").unwrap()),
            nice: None,
            ionice: None,
            match_class: Vec::new(),
            match_comm: Vec::new(),
        }],
    };
    let candidate = CandidateAction::cpu_affinity_profile(profile, 11);
    let snapshot = affinity_snapshot([(11, "3")]);
    let active_tasks = vec![active_task_with_class(11, TaskClass::Unknown)];

    let active_match = candidate.matches_active_config(ActiveConfigMatchInput {
        snapshot: &snapshot,
        active_tasks: &active_tasks,
    });

    assert!(active_match.is_match());
}
