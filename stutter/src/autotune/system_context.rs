use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    autotune::{
        active_config::{ActiveConfigCollectorInput, collect_active_config},
        observation::{ActiveConfigSnapshot, ActiveTaskSnapshot, AutotuneObservation},
    },
    daemon::{
        DaemonCapabilities, SystemHealthSnapshot,
        capabilities::{CapabilityProbe, CapabilityProbeRoot},
    },
    system_inventory::{DrmDeviceInventory, SystemInventory, SystemInventoryRoot},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SystemContextSnapshot {
    pub capabilities: DaemonCapabilities,
    pub health: SystemHealthSnapshot,
    pub inventory: SystemInventory,
    pub active_config: ActiveConfigSnapshot,
    pub sampled_at_unix_nanos: u128,
}

#[derive(Clone, Debug)]
pub struct SystemContextSnapshotInput<'a> {
    pub proc_root: &'a Path,
    pub sys_root: &'a Path,
    pub active_tasks: &'a [ActiveTaskSnapshot],
    pub health: SystemHealthSnapshot,
    pub sampled_at_unix_nanos: u128,
}

impl SystemContextSnapshot {
    pub fn from_observation(observation: &AutotuneObservation) -> Self {
        Self {
            capabilities: observation.capabilities.clone(),
            health: observation.system_health.clone(),
            inventory: empty_inventory(),
            active_config: observation
                .active_config_snapshot
                .clone()
                .unwrap_or_default(),
            sampled_at_unix_nanos: observation.now_unix_nanos,
        }
    }
}

pub fn collect_system_context(input: SystemContextSnapshotInput<'_>) -> SystemContextSnapshot {
    let capabilities = CapabilityProbe::new(CapabilityProbeRoot {
        proc_root: input.proc_root.to_path_buf(),
        sys_root: input.sys_root.to_path_buf(),
    })
    .probe();

    let inventory = SystemInventory::probe_root(&SystemInventoryRoot {
        proc_root: input.proc_root.to_path_buf(),
        sys_root: input.sys_root.to_path_buf(),
    });

    let active_config = collect_active_config(ActiveConfigCollectorInput {
        proc_root: input.proc_root,
        sys_root: input.sys_root,
        active_tasks: input.active_tasks,
        capabilities: &capabilities,
        inventory: &inventory,
    });

    SystemContextSnapshot {
        capabilities,
        health: input.health,
        inventory,
        active_config,
        sampled_at_unix_nanos: input.sampled_at_unix_nanos,
    }
}

fn empty_inventory() -> SystemInventory {
    SystemInventory {
        cpu_policies: Vec::new(),
        drm_devices: Vec::<DrmDeviceInventory>::new(),
        irq_default_smp_affinity: None,
        irq_lines: Vec::new(),
        power_source: Default::default(),
        sched_ext_available: false,
        vm_knobs: BTreeMap::new(),
        inventory_hash: "empty".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        autotune::observation::ActiveTaskSnapshot,
        daemon::{SystemHealthSnapshot, SystemHealthState},
        process_tree::TaskClass,
        test_support::TestRoot,
    };

    #[test]
    fn collect_system_context_populates_capabilities_inventory_and_active_config_once() {
        let root = TestRoot::new("system-context");
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        fs::create_dir_all(proc_root.join("sys/kernel")).unwrap();
        fs::create_dir_all(proc_root.join("sys/vm")).unwrap();
        fs::create_dir_all(proc_root.join("irq")).unwrap();
        fs::create_dir_all(sys_root.join("devices/system/cpu/cpufreq/policy0")).unwrap();
        fs::create_dir_all(sys_root.join("class/drm/card7/device/drm/renderD777")).unwrap();

        fs::write(proc_root.join("sys/kernel/osrelease"), "test-kernel\n").unwrap();
        fs::write(proc_root.join("sys/kernel/perf_event_paranoid"), "1\n").unwrap();
        fs::write(proc_root.join("irq/default_smp_affinity"), "ff\n").unwrap();
        fs::write(proc_root.join("sys/vm/swappiness"), "60\n").unwrap();
        fs::write(proc_root.join("sys/vm/dirty_ratio"), "20\n").unwrap();
        fs::write(proc_root.join("sys/vm/dirty_background_ratio"), "10\n").unwrap();

        fs::write(
            sys_root.join("devices/system/cpu/cpufreq/policy0/scaling_governor"),
            "powersave\n",
        )
        .unwrap();
        fs::write(
            sys_root.join("devices/system/cpu/cpufreq/policy0/related_cpus"),
            "0 1\n",
        )
        .unwrap();

        let task_root = proc_root.join("99");
        fs::create_dir_all(&task_root).unwrap();
        fs::write(
            task_root.join("status"),
            "Name:\ttest\nCpus_allowed_list:\t0-1\n",
        )
        .unwrap();
        fs::write(task_root.join("stat"), fake_stat_line(99, "test", 5)).unwrap();
        fs::write(task_root.join("cgroup"), "0::/user.slice/test.scope\n").unwrap();

        let active_tasks = vec![ActiveTaskSnapshot {
            tid: 99,
            process_pid: 99,
            comm: "test".to_owned(),
            class: TaskClass::Unknown,
            process_starttime_ticks: Some(1),
            task_starttime_ticks: Some(1),
            cgroup_path: Some("/user.slice/test.scope".to_owned()),
        }];

        let health = SystemHealthSnapshot {
            state: SystemHealthState::Healthy,
            ok_for_apply: true,
            reason_code: Some("ok".to_owned()),
            ..SystemHealthSnapshot::default()
        };

        let snapshot = collect_system_context(SystemContextSnapshotInput {
            proc_root: &proc_root,
            sys_root: &sys_root,
            active_tasks: &active_tasks,
            health: health.clone(),
            sampled_at_unix_nanos: 123,
        });

        assert_eq!(snapshot.sampled_at_unix_nanos, 123);
        assert_eq!(snapshot.health, health);
        assert_eq!(
            snapshot.capabilities.kernel_release.as_deref(),
            Some("test-kernel")
        );
        assert_eq!(
            snapshot.inventory.irq_default_smp_affinity.as_deref(),
            Some("ff")
        );
        assert_eq!(snapshot.inventory.cpu_policies.len(), 1);
        assert_eq!(snapshot.inventory.drm_devices.len(), 1);
        assert_eq!(
            snapshot.active_config.affinity.per_tid.get(&99).unwrap(),
            "0-1"
        );
        assert_eq!(snapshot.active_config.nice.per_tid.get(&99), Some(&5));

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("\"capabilities\""));
        assert!(json.contains("\"inventory\""));
        assert!(json.contains("\"active_config\""));
        assert!(json.contains("\"sampled_at_unix_nanos\":123"));
    }

    fn fake_stat_line(tid: u32, comm: &str, nice: i32) -> String {
        let mut fields = vec!["0".to_owned(); 40];
        fields[0] = "S".to_owned();
        fields[17] = nice.to_string();
        format!("{tid} ({comm}) {}\n", fields.join(" "))
    }
}
