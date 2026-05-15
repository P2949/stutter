use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use crate::{
    actions::uclamp::UclampValues,
    autotune::{
        candidate::CandidateAction,
        observation::{
            ActiveAffinitySnapshot, ActiveCgroupSnapshot, ActiveConfigSnapshot,
            ActiveCpuPowerSnapshot, ActiveGpuPowerSnapshot, ActiveIoPrioSnapshot,
            ActiveIrqSnapshot, ActiveNiceSnapshot, ActiveTaskSnapshot, ActiveUclampSnapshot,
            ActiveVmSnapshot, CpuPolicyRuntimeState, GpuPowerRuntimeState,
        },
    },
    daemon::capabilities::DaemonCapabilities,
    system_inventory::SystemInventory,
};

#[derive(Clone, Copy, Debug)]
pub struct ActiveConfigCollectorInput<'a> {
    pub proc_root: &'a Path,
    pub sys_root: &'a Path,
    pub active_tasks: &'a [ActiveTaskSnapshot],
    pub capabilities: &'a DaemonCapabilities,
    pub inventory: &'a SystemInventory,
}

#[derive(Clone, Debug, Default)]
pub struct ActiveConfigCollector;

impl ActiveConfigCollector {
    pub fn collect(&self, input: ActiveConfigCollectorInput<'_>) -> ActiveConfigSnapshot {
        let tids = sorted_active_tids(input.active_tasks);

        ActiveConfigSnapshot {
            affinity: collect_affinity(input.proc_root, &tids),
            nice: collect_nice(input.proc_root, &tids),
            ionice: if input.capabilities.ionice_available {
                collect_ioprio(input.proc_root, &tids)
            } else {
                ActiveIoPrioSnapshot::default()
            },
            uclamp: if input.capabilities.uclamp_available {
                collect_uclamp(input.proc_root, &tids)
            } else {
                ActiveUclampSnapshot::default()
            },
            cgroup: collect_cgroups(input.proc_root, &tids),
            irq: if input.capabilities.irq_affinity_available {
                collect_irq(input.proc_root)
            } else {
                ActiveIrqSnapshot::default()
            },
            cpu_power: collect_cpu_power(input.inventory),
            gpu_power: if input.capabilities.gpu_sysfs_available {
                collect_gpu_power(input.sys_root, input.inventory)
            } else {
                ActiveGpuPowerSnapshot::default()
            },
            vm: collect_vm(input.inventory),
        }
    }
}

pub fn collect_active_config(input: ActiveConfigCollectorInput<'_>) -> ActiveConfigSnapshot {
    ActiveConfigCollector.collect(input)
}

pub fn candidate_is_noop(candidate: &CandidateAction, snapshot: &ActiveConfigSnapshot) -> bool {
    match candidate {
        CandidateAction::CpuAffinityProfile { .. } => false,
        CandidateAction::Nice { plan } => plan.action.targets.iter().all(|target| {
            snapshot
                .nice
                .per_tid
                .get(&target.tid)
                .is_some_and(|current| *current == plan.action.nice)
        }),
        CandidateAction::IoPrio { plan } => {
            let requested = plan.action.ioprio.label();
            plan.action.targets.iter().all(|target| {
                snapshot
                    .ionice
                    .per_tid
                    .get(&target.tid)
                    .is_some_and(|current| current == &requested)
            })
        }
        CandidateAction::Uclamp { plan } => plan.action.targets.iter().all(|target| {
            snapshot
                .uclamp
                .per_tid
                .get(&target.tid)
                .is_some_and(|current| uclamp_matches_request(*current, plan.action.values))
        }),
        CandidateAction::CgroupPlacement { plan } => {
            let requested = normalize_cgroup_path(&plan.action.target_cgroup);
            plan.action.targets.iter().all(|target| {
                snapshot
                    .cgroup
                    .per_tid
                    .get(&target.identity.tid)
                    .is_some_and(|current| normalize_cgroup_str(current) == requested)
            })
        }
        CandidateAction::IrqAffinity { plan } => snapshot
            .irq
            .per_irq
            .get(&plan.action.irq)
            .is_some_and(|current| current.trim() == plan.action.smp_affinity.trim()),
        CandidateAction::CpuPower { plan } => plan.action.cpus.iter().all(|cpu| {
            let Some(policy) = cpu_policy_for_cpu(&snapshot.cpu_power.policies, *cpu) else {
                return false;
            };

            plan.action
                .scaling_governor
                .as_ref()
                .is_none_or(|requested| policy.scaling_governor.as_ref() == Some(requested))
                && plan
                    .action
                    .energy_performance_preference
                    .as_ref()
                    .is_none_or(|requested| {
                        policy.energy_performance_preference.as_ref() == Some(requested)
                    })
        }),
        CandidateAction::GpuPower { plan } => snapshot
            .gpu_power
            .devices
            .iter()
            .find(|device| device.device == plan.action.drm_card)
            .is_some_and(|device| {
                plan.action
                    .power_dpm_force_performance_level
                    .as_ref()
                    .is_none_or(|requested| {
                        device.power_dpm_force_performance_level.as_ref() == Some(requested)
                    })
                    && plan
                        .action
                        .pp_power_profile_mode
                        .as_ref()
                        .is_none_or(|requested| {
                            device.pp_power_profile_mode.as_ref() == Some(requested)
                        })
            }),
        CandidateAction::VmKnob { plan } => plan.action.changes.iter().all(|change| {
            vm_knob_keys_for_change(&plan.action.root, &change.path)
                .into_iter()
                .any(|key| snapshot.vm.knobs.get(&key) == Some(&change.value))
        }),
        CandidateAction::Fake { .. } => false,
    }
}

fn sorted_active_tids(active_tasks: &[ActiveTaskSnapshot]) -> Vec<u32> {
    active_tasks
        .iter()
        .map(|task| task.tid)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn collect_affinity(proc_root: &Path, tids: &[u32]) -> ActiveAffinitySnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_cpus_allowed_list(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveAffinitySnapshot { per_tid }
}

fn collect_nice(proc_root: &Path, tids: &[u32]) -> ActiveNiceSnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_nice(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveNiceSnapshot { per_tid }
}

fn collect_ioprio(proc_root: &Path, tids: &[u32]) -> ActiveIoPrioSnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_ioprio(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveIoPrioSnapshot { per_tid }
}

fn collect_uclamp(proc_root: &Path, tids: &[u32]) -> ActiveUclampSnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_uclamp(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveUclampSnapshot { per_tid }
}

fn collect_cgroups(proc_root: &Path, tids: &[u32]) -> ActiveCgroupSnapshot {
    let mut per_tid = BTreeMap::new();

    for tid in tids {
        if let Some(value) = read_cgroup(proc_root, *tid) {
            per_tid.insert(*tid, value);
        }
    }

    ActiveCgroupSnapshot { per_tid }
}

fn collect_irq(proc_root: &Path) -> ActiveIrqSnapshot {
    let mut per_irq = BTreeMap::new();
    let irq_root = proc_root.join("irq");
    let Ok(entries) = fs::read_dir(irq_root) else {
        return ActiveIrqSnapshot { per_irq };
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Ok(irq) = file_name.parse::<u32>() else {
            continue;
        };
        if let Some(value) = read_trimmed(entry.path().join("smp_affinity")) {
            per_irq.insert(irq, value);
        }
    }

    ActiveIrqSnapshot { per_irq }
}

fn collect_cpu_power(inventory: &SystemInventory) -> ActiveCpuPowerSnapshot {
    ActiveCpuPowerSnapshot {
        policies: inventory
            .cpu_policies
            .iter()
            .map(|policy| CpuPolicyRuntimeState {
                policy: policy.policy.clone(),
                scaling_governor: policy.scaling_governor.clone(),
                energy_performance_preference: policy.energy_performance_preference.clone(),
                related_cpus: policy.related_cpus.clone(),
            })
            .collect(),
    }
}

fn collect_gpu_power(sys_root: &Path, inventory: &SystemInventory) -> ActiveGpuPowerSnapshot {
    ActiveGpuPowerSnapshot {
        devices: inventory
            .drm_devices
            .iter()
            .map(|device| {
                let device_dir = sys_root.join("class/drm").join(&device.name).join("device");
                GpuPowerRuntimeState {
                    device: device.name.clone(),
                    power_dpm_force_performance_level: read_trimmed(
                        device_dir.join("power_dpm_force_performance_level"),
                    ),
                    pp_power_profile_mode: read_trimmed(device_dir.join("pp_power_profile_mode")),
                }
            })
            .collect(),
    }
}

fn collect_vm(inventory: &SystemInventory) -> ActiveVmSnapshot {
    ActiveVmSnapshot {
        knobs: inventory.vm_knobs.clone(),
    }
}

fn read_cpus_allowed_list(proc_root: &Path, tid: u32) -> Option<String> {
    read_status_value(
        proc_root.join(tid.to_string()).join("status"),
        "Cpus_allowed_list",
    )
}

fn read_status_value(path: PathBuf, key: &str) -> Option<String> {
    let contents = fs::read_to_string(path).ok()?;
    contents.lines().find_map(|line| {
        let (left, right) = line.split_once(':')?;
        (left.trim() == key).then(|| right.trim().to_owned())
    })
}

fn read_nice(proc_root: &Path, tid: u32) -> Option<i32> {
    let contents = fs::read_to_string(proc_root.join(tid.to_string()).join("stat")).ok()?;
    let fields = stat_fields_after_comm(&contents)?;
    fields.get(17)?.parse::<i32>().ok()
}

fn read_ioprio(proc_root: &Path, tid: u32) -> Option<String> {
    let fake_file = proc_root.join(tid.to_string()).join("ioprio");
    if let Some(value) = read_trimmed(fake_file) {
        return Some(value);
    }

    if proc_root != Path::new("/proc") {
        return None;
    }

    read_live_ioprio(tid)
}

#[cfg(target_os = "linux")]
fn read_live_ioprio(tid: u32) -> Option<String> {
    const IOPRIO_WHO_PROCESS: libc::c_int = 1;
    let raw = unsafe { libc::syscall(libc::SYS_ioprio_get, IOPRIO_WHO_PROCESS, tid) };
    if raw < 0 {
        return None;
    }

    crate::actions::ioprio::IoPrioValue::decode(raw as i32)
        .ok()
        .map(|value| value.label())
}

#[cfg(not(target_os = "linux"))]
fn read_live_ioprio(_tid: u32) -> Option<String> {
    None
}

fn read_uclamp(proc_root: &Path, tid: u32) -> Option<UclampValues> {
    let contents = fs::read_to_string(proc_root.join(tid.to_string()).join("sched")).ok()?;
    let mut sched_util_min = None;
    let mut sched_util_max = None;

    for line in contents.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let parsed = value.trim().parse::<u32>().ok();
        match key.trim() {
            "uclamp.min" => sched_util_min = parsed,
            "uclamp.max" => sched_util_max = parsed,
            _ => {}
        }
    }

    (sched_util_min.is_some() || sched_util_max.is_some()).then_some(UclampValues {
        sched_util_min,
        sched_util_max,
    })
}

fn read_cgroup(proc_root: &Path, tid: u32) -> Option<String> {
    let contents = fs::read_to_string(proc_root.join(tid.to_string()).join("cgroup")).ok()?;
    contents.lines().find_map(|line| {
        let (_, path) = line.rsplit_once(':')?;
        Some(path.trim().to_owned())
    })
}

fn stat_fields_after_comm(contents: &str) -> Option<Vec<&str>> {
    let end = contents.rfind(") ")?;
    Some(contents[end + 2..].split_whitespace().collect())
}

fn read_trimmed(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn uclamp_matches_request(current: UclampValues, requested: UclampValues) -> bool {
    requested
        .sched_util_min
        .is_none_or(|value| current.sched_util_min == Some(value))
        && requested
            .sched_util_max
            .is_none_or(|value| current.sched_util_max == Some(value))
}

fn normalize_cgroup_path(path: &Path) -> String {
    normalize_cgroup_str(&path.to_string_lossy())
}

fn normalize_cgroup_str(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "/".to_owned()
    } else if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    }
}

fn cpu_policy_for_cpu(
    policies: &[CpuPolicyRuntimeState],
    cpu: u32,
) -> Option<&CpuPolicyRuntimeState> {
    policies.iter().find(|policy| {
        policy
            .related_cpus
            .as_deref()
            .is_some_and(|related| cpu_list_contains(related, cpu))
            || policy.policy == format!("policy{cpu}")
    })
}

fn cpu_list_contains(list: &str, cpu: u32) -> bool {
    list.split(',').any(|part| {
        let part = part.trim();
        if let Some((start, end)) = part.split_once('-') {
            let Ok(start) = start.trim().parse::<u32>() else {
                return false;
            };
            let Ok(end) = end.trim().parse::<u32>() else {
                return false;
            };
            (start..=end).contains(&cpu)
        } else {
            part.parse::<u32>().is_ok_and(|value| value == cpu)
        }
    })
}

fn vm_knob_keys_for_change(root: &Path, path: &Path) -> Vec<String> {
    let mut keys = BTreeSet::new();
    keys.insert(path.to_string_lossy().trim_start_matches('/').to_owned());

    if let Ok(relative) = path.strip_prefix(root) {
        keys.insert(
            relative
                .to_string_lossy()
                .trim_start_matches('/')
                .to_owned(),
        );
    }

    if let Ok(relative) = path.strip_prefix("/proc") {
        keys.insert(
            relative
                .to_string_lossy()
                .trim_start_matches('/')
                .to_owned(),
        );
    }

    keys.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;
    use crate::{
        autotune::observation::ActiveTaskSnapshot,
        daemon::capabilities::DaemonCapabilities,
        process_tree::TaskClass,
        system_inventory::{SystemInventory, SystemInventoryRoot},
        test_support::TestRoot,
    };

    #[test]
    fn collector_populates_every_active_config_subsnapshot_and_serializes() {
        let root = TestRoot::new("active-config");
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        fs::create_dir_all(&proc_root).unwrap();
        fs::create_dir_all(&sys_root).unwrap();

        write_task_fixture(
            &proc_root,
            42,
            "game",
            "0-3",
            5,
            "best-effort:4",
            128,
            1024,
            "/game.slice",
        );
        write_irq_fixture(&proc_root, 44, "00000002");
        write_cpu_policy_fixture(&sys_root, "policy0", "performance", "performance", "0-3");
        write_gpu_fixture(&sys_root, "card0", "high", "3D_FULL_SCREEN");
        write_vm_fixture(&proc_root, "10", "20", "5");

        let inventory = SystemInventory::probe_root(&SystemInventoryRoot {
            proc_root: proc_root.clone(),
            sys_root: sys_root.clone(),
        });
        let capabilities = DaemonCapabilities {
            ionice_available: true,
            uclamp_available: true,
            irq_affinity_available: true,
            gpu_sysfs_available: true,
            ..DaemonCapabilities::default()
        };
        let active_tasks = vec![active_task(42)];

        let snapshot = collect_active_config(ActiveConfigCollectorInput {
            proc_root: &proc_root,
            sys_root: &sys_root,
            active_tasks: &active_tasks,
            capabilities: &capabilities,
            inventory: &inventory,
        });

        assert_eq!(snapshot.affinity.per_tid.get(&42).unwrap(), "0-3");
        assert_eq!(snapshot.nice.per_tid.get(&42), Some(&5));
        assert_eq!(snapshot.ionice.per_tid.get(&42).unwrap(), "best-effort:4");
        assert_eq!(
            snapshot.uclamp.per_tid.get(&42).unwrap(),
            &UclampValues {
                sched_util_min: Some(128),
                sched_util_max: Some(1024)
            }
        );
        assert_eq!(snapshot.cgroup.per_tid.get(&42).unwrap(), "/game.slice");
        assert_eq!(snapshot.irq.per_irq.get(&44).unwrap(), "00000002");
        assert_eq!(snapshot.cpu_power.policies.len(), 1);
        assert_eq!(
            snapshot.cpu_power.policies[0].scaling_governor.as_deref(),
            Some("performance")
        );
        assert_eq!(
            snapshot.cpu_power.policies[0]
                .energy_performance_preference
                .as_deref(),
            Some("performance")
        );
        assert_eq!(snapshot.gpu_power.devices.len(), 1);
        assert_eq!(
            snapshot.gpu_power.devices[0]
                .power_dpm_force_performance_level
                .as_deref(),
            Some("high")
        );
        assert_eq!(
            snapshot.gpu_power.devices[0]
                .pp_power_profile_mode
                .as_deref(),
            Some("3D_FULL_SCREEN")
        );
        assert_eq!(snapshot.vm.knobs.get("sys/vm/swappiness").unwrap(), "10");

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"affinity\""));
        assert!(json.contains("\"ionice\""));
        assert!(json.contains("\"cpu_power\""));
        assert!(json.contains("\"gpu_power\""));
        assert!(json.contains("\"sched_util_min\":128"));
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
            77,
            "worker",
            "2",
            10,
            "idle",
            64,
            512,
            "/compile.slice",
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

    fn active_task(tid: u32) -> ActiveTaskSnapshot {
        ActiveTaskSnapshot {
            tid,
            process_pid: tid,
            comm: format!("task-{tid}"),
            class: TaskClass::Unknown,
            process_starttime_ticks: Some(1),
            task_starttime_ticks: Some(1),
            cgroup_path: Some("/game.slice".to_owned()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_task_fixture(
        proc_root: &Path,
        tid: u32,
        comm: &str,
        cpus_allowed_list: &str,
        nice: i32,
        ioprio: &str,
        uclamp_min: u32,
        uclamp_max: u32,
        cgroup: &str,
    ) {
        let task_root = proc_root.join(tid.to_string());
        fs::create_dir_all(&task_root).unwrap();

        fs::write(
            task_root.join("status"),
            format!("Name:\t{comm}\nCpus_allowed_list:\t{cpus_allowed_list}\n"),
        )
        .unwrap();
        fs::write(task_root.join("stat"), fake_stat_line(tid, comm, nice)).unwrap();
        fs::write(task_root.join("ioprio"), format!("{ioprio}\n")).unwrap();
        fs::write(
            task_root.join("sched"),
            format!("uclamp.min                                   : {uclamp_min}\nuclamp.max                                   : {uclamp_max}\n"),
        )
        .unwrap();
        fs::write(task_root.join("cgroup"), format!("0::{cgroup}\n")).unwrap();
    }

    fn fake_stat_line(tid: u32, comm: &str, nice: i32) -> String {
        let mut fields = vec!["0".to_owned(); 40];
        fields[0] = "S".to_owned();
        fields[17] = nice.to_string();
        format!("{tid} ({comm}) {}\n", fields.join(" "))
    }

    fn write_irq_fixture(proc_root: &Path, irq: u32, smp_affinity: &str) {
        let irq_root = proc_root.join("irq").join(irq.to_string());
        fs::create_dir_all(&irq_root).unwrap();
        fs::write(irq_root.join("smp_affinity"), format!("{smp_affinity}\n")).unwrap();
    }

    fn write_cpu_policy_fixture(
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

    fn write_gpu_fixture(sys_root: &Path, card: &str, dpm: &str, profile: &str) {
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

    fn write_vm_fixture(proc_root: &Path, swappiness: &str, dirty_ratio: &str, dirty_bg: &str) {
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
}
