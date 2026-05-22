use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use crate::{
    actions::uclamp::UclampValues,
    autotune::observation::{
        ActiveAffinitySnapshot, ActiveCgroupSnapshot, ActiveConfigSnapshot, ActiveCpuPowerSnapshot,
        ActiveGpuPowerSnapshot, ActiveIoPrioSnapshot, ActiveIrqSnapshot, ActiveNiceSnapshot,
        ActiveTaskSnapshot, ActiveUclampSnapshot, ActiveVmSnapshot, CpuPolicyRuntimeState,
        GpuPowerRuntimeState,
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
    fields.get(16)?.parse::<i32>().ok()
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
