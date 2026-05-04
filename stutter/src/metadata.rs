use std::fs;

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct SystemMetadata {
    pub kernel_osrelease: Option<String>,
    pub kernel_version: Option<String>,
    pub cpu_online: Option<String>,
    pub cpu_possible: Option<String>,
    pub cpu_topology: Vec<CpuTopology>,
    pub scx_state: Option<String>,
    pub scx_ops: Option<String>,
    pub scx_enable_seq: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct CpuTopology {
    pub cpu: u32,
    pub thread_siblings_list: Option<String>,
    pub core_id: Option<String>,
    pub physical_package_id: Option<String>,
}

pub fn collect_system_metadata() -> SystemMetadata {
    SystemMetadata {
        kernel_osrelease: read_trimmed("/proc/sys/kernel/osrelease"),
        kernel_version: read_trimmed("/proc/version"),
        cpu_online: read_trimmed("/sys/devices/system/cpu/online"),
        cpu_possible: read_trimmed("/sys/devices/system/cpu/possible"),
        cpu_topology: collect_cpu_topology(),
        scx_state: read_trimmed("/sys/kernel/sched_ext/state"),
        scx_ops: read_trimmed("/sys/kernel/sched_ext/root/ops"),
        scx_enable_seq: read_trimmed("/sys/kernel/sched_ext/enable_seq"),
    }
}

fn collect_cpu_topology() -> Vec<CpuTopology> {
    let mut cpus = Vec::new();

    let Ok(entries) = fs::read_dir("/sys/devices/system/cpu") else {
        return cpus;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };

        let Some(cpu_str) = name.strip_prefix("cpu") else {
            continue;
        };

        let Ok(cpu) = cpu_str.parse::<u32>() else {
            continue;
        };

        let topology = entry.path().join("topology");
        cpus.push(CpuTopology {
            cpu,
            thread_siblings_list: read_trimmed(topology.join("thread_siblings_list")),
            core_id: read_trimmed(topology.join("core_id")),
            physical_package_id: read_trimmed(topology.join("physical_package_id")),
        });
    }

    cpus.sort_by_key(|cpu| cpu.cpu);
    cpus
}

fn read_trimmed(path: impl AsRef<std::path::Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}
