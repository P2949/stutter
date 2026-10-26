//! Candidates for the simulation, built like the providers build them.
use crate::{
    actions::{
        TaskIdentity,
        cgroup::{CgroupPlacementAction, CgroupPlacementTarget},
        nice::{NiceAction, NicePolicy},
    },
    affinity::CpuMask,
    autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::CandidateAction,
            executable_plan::{CgroupPlacementActionPlan, NiceActionPlan},
        },
    },
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
};

const OBJECTIVE: ObjectiveKind = ObjectiveKind::GameRunnableLatency;

fn game() -> TaskIdentity {
    TaskIdentity {
        tid: 1234_u32.into(),
        process_pid: Some(1234_u32.into()),
        comm: Some("game".to_owned()),
        starttime_ticks: None,
    }
}

pub(super) fn affinity() -> CandidateAction {
    CandidateAction::cpu_affinity_profile_for(
        Profile {
            name: "sim-affinity".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").expect("valid mask")),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        },
        1234,
        OBJECTIVE,
    )
}

pub(super) fn cgroup() -> CandidateAction {
    CandidateAction::CgroupPlacement {
        plan: CgroupPlacementActionPlan {
            name: "sim-cgroup".to_owned(),
            action: CgroupPlacementAction {
                cgroup_root: "/sys/fs/cgroup".into(),
                target_cgroup: "/sys/fs/cgroup/game.slice".into(),
                targets: vec![CgroupPlacementTarget {
                    identity: game(),
                    class: TaskClass::Game,
                }],
                cpuset_cpus: None,
                cpuset_mems: None,
            },
            target_root_pid: Some(1234),
            evidence: Vec::new(),
            objective: OBJECTIVE,
        },
    }
}

pub(super) fn nice() -> CandidateAction {
    CandidateAction::Nice {
        plan: NiceActionPlan {
            name: "sim-nice".to_owned(),
            action: NiceAction {
                targets: vec![game()],
                nice: 5,
                policy: NicePolicy::default(),
            },
            target_root_pid: Some(1234),
            evidence: Vec::new(),
            objective: OBJECTIVE,
        },
    }
}
