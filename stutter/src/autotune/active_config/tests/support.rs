use std::{fs, path::Path};

use crate::{
    affinity::CpuMask,
    autotune::{
        observation::{
            ActiveAffinitySnapshot, ActiveConfigSnapshot, ActiveNiceSnapshot, ActiveTaskSnapshot,
        },
        planning::candidate::CandidateAction,
    },
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
};

pub(super) fn active_task(tid: u32) -> ActiveTaskSnapshot {
    active_task_with_class(tid, TaskClass::Unknown)
}

pub(super) fn active_task_with_class(tid: u32, class: TaskClass) -> ActiveTaskSnapshot {
    ActiveTaskSnapshot {
        tid: tid.into(),
        process_pid: (tid).into(),
        comm: format!("task-{tid}"),
        class,
        process_starttime_ticks: Some(1),
        task_starttime_ticks: Some(1),
        cgroup_path: Some("/game.slice".to_owned()),
    }
}

pub(super) fn profile_for_class(class: TaskClass, mask: &str) -> Profile {
    Profile {
        name: format!("profile-{class:?}"),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse(mask).unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![class],
            match_comm: Vec::new(),
        }],
    }
}

pub(super) fn affinity_snapshot<const N: usize>(entries: [(u32, &str); N]) -> ActiveConfigSnapshot {
    ActiveConfigSnapshot {
        affinity: ActiveAffinitySnapshot {
            per_tid: entries
                .into_iter()
                .map(|(tid, mask)| (tid, mask.to_owned()))
                .collect(),
        },
        ..ActiveConfigSnapshot::default()
    }
}

pub(super) struct TaskFixture<'a> {
    pub(super) tid: u32,
    pub(super) comm: &'a str,
    pub(super) cpus_allowed_list: &'a str,
    pub(super) nice: i32,
    pub(super) ioprio: &'a str,
    pub(super) uclamp_min: u32,
    pub(super) uclamp_max: u32,
    pub(super) cgroup: &'a str,
}

pub(super) fn write_task_fixture(proc_root: &Path, fixture: TaskFixture<'_>) {
    let task_root = proc_root.join(fixture.tid.to_string());
    fs::create_dir_all(&task_root).unwrap();

    fs::write(
        task_root.join("status"),
        format!(
            "Name:\t{}\nCpus_allowed_list:\t{}\n",
            fixture.comm, fixture.cpus_allowed_list
        ),
    )
    .unwrap();
    fs::write(
        task_root.join("stat"),
        fake_stat_line(fixture.tid, fixture.comm, fixture.nice),
    )
    .unwrap();
    fs::write(task_root.join("ioprio"), format!("{}\n", fixture.ioprio)).unwrap();
    fs::write(
            task_root.join("sched"),
            format!(
                "uclamp.min                                   : {}\nuclamp.max                                   : {}\n",
                fixture.uclamp_min, fixture.uclamp_max
            ),
        )
        .unwrap();
    fs::write(task_root.join("cgroup"), format!("0::{}\n", fixture.cgroup)).unwrap();
}

pub(super) fn fake_stat_line(tid: u32, comm: &str, nice: i32) -> String {
    let mut fields = vec!["0".to_owned(); 40];
    fields[0] = "S".to_owned();
    fields[16] = nice.to_string();
    format!("{tid} ({comm}) {}\n", fields.join(" "))
}

pub(super) fn active_nice_snapshot(tid: u32, nice: i32) -> ActiveConfigSnapshot {
    ActiveConfigSnapshot {
        nice: ActiveNiceSnapshot {
            per_tid: std::collections::BTreeMap::from([(tid, nice)]),
        },
        ..ActiveConfigSnapshot::default()
    }
}

pub(super) fn nice_candidate_for_rollback() -> CandidateAction {
    CandidateAction::Nice {
        plan: crate::autotune::planning::executable_plan::NiceActionPlan {
            name: "nice-rollback-verification".to_owned(),
            action: crate::actions::nice::NiceAction {
                targets: vec![crate::actions::TaskIdentity {
                    tid: 42,
                    process_pid: Some(42),
                    comm: Some("game".to_owned()),
                    starttime_ticks: Some(1),
                }],
                nice: 5,
                policy: crate::actions::nice::NicePolicy::default(),
            },
            target_root_pid: Some(42),
            evidence: Vec::new(),
            objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
        },
    }
}

pub(super) fn write_irq_fixture(proc_root: &Path, irq: u32, smp_affinity: &str) {
    let irq_root = proc_root.join("irq").join(irq.to_string());
    fs::create_dir_all(&irq_root).unwrap();
    fs::write(irq_root.join("smp_affinity"), format!("{smp_affinity}\n")).unwrap();
}

pub(super) fn write_cpu_policy_fixture(
    sys_root: &Path,
    policy: &str,
    governor: &str,
    epp: &str,
    related_cpus: &str,
) {
    let policy_root = sys_root.join("devices/system/cpu/cpufreq").join(policy);
    fs::create_dir_all(&policy_root).unwrap();
    fs::write(
        policy_root.join("scaling_governor"),
        format!("{governor}\n"),
    )
    .unwrap();
    fs::write(
        policy_root.join("energy_performance_preference"),
        format!("{epp}\n"),
    )
    .unwrap();
    fs::write(
        policy_root.join("related_cpus"),
        format!("{related_cpus}\n"),
    )
    .unwrap();
}

pub(super) fn write_gpu_fixture(sys_root: &Path, card: &str, dpm: &str, profile: &str) {
    let device_root = sys_root.join("class/drm").join(card).join("device");
    fs::create_dir_all(device_root.join("drm/renderD128")).unwrap();
    fs::write(
        device_root.join("power_dpm_force_performance_level"),
        format!("{dpm}\n"),
    )
    .unwrap();
    fs::write(
        device_root.join("pp_power_profile_mode"),
        format!("{profile}\n"),
    )
    .unwrap();
}

pub(super) fn write_vm_fixture(
    proc_root: &Path,
    swappiness: &str,
    dirty_ratio: &str,
    dirty_bg: &str,
) {
    let vm_root = proc_root.join("sys/vm");
    fs::create_dir_all(&vm_root).unwrap();
    fs::write(vm_root.join("swappiness"), format!("{swappiness}\n")).unwrap();
    fs::write(vm_root.join("dirty_ratio"), format!("{dirty_ratio}\n")).unwrap();
    fs::write(
        vm_root.join("dirty_background_ratio"),
        format!("{dirty_bg}\n"),
    )
    .unwrap();
}
