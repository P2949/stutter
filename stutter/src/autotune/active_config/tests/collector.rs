use std::fs;

use super::support::*;
use crate::{
    actions::uclamp::UclampValues,
    autotune::active_config::{ActiveConfigCollectorInput, collect_active_config},
    daemon::capabilities::DaemonCapabilities,
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
        TaskFixture {
            tid: 42,
            comm: "game",
            cpus_allowed_list: "0-3",
            nice: 5,
            ioprio: "best-effort:4",
            uclamp_min: 128,
            uclamp_max: 1024,
            cgroup: "/game.slice",
        },
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
