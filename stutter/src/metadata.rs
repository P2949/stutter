use std::fs;

use serde::{Deserialize, Serialize};

use crate::irq_inspect::{IrqLine, parse_proc_interrupts};

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct SystemMetadata {
    pub kernel_osrelease: Option<String>,
    pub kernel_version: Option<String>,
    pub cpu_online: Option<String>,
    pub cpu_possible: Option<String>,
    pub cpu_topology: Vec<CpuTopology>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub irq_lines: Vec<IrqLine>,
    pub scx_state: Option<String>,
    pub scx_ops: Option<String>,
    pub scx_enable_seq: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct CpuTopology {
    pub cpu: u32,
    pub thread_siblings_list: Option<String>,
    pub core_id: Option<String>,
    pub physical_package_id: Option<String>,
}

pub fn build_git_rev() -> &'static str {
    option_env!("STUTTER_GIT_REV").unwrap_or("unknown")
}

pub fn build_version() -> &'static str {
    option_env!("STUTTER_BUILD_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

pub fn build_feature_labels() -> Vec<&'static str> {
    let mut features = Vec::new();

    if cfg!(feature = "otel") {
        features.push("otel");
    }
    if cfg!(feature = "wayland-probe") {
        features.push("wayland-probe");
    }
    if features.is_empty() {
        features.push("default");
    }

    features
}

pub fn collect_system_metadata() -> SystemMetadata {
    SystemMetadata {
        kernel_osrelease: read_trimmed("/proc/sys/kernel/osrelease"),
        kernel_version: read_trimmed("/proc/version"),
        cpu_online: read_trimmed("/sys/devices/system/cpu/online"),
        cpu_possible: read_trimmed("/sys/devices/system/cpu/possible"),
        cpu_topology: collect_cpu_topology(),
        irq_lines: collect_irq_lines(),
        scx_state: read_trimmed("/sys/kernel/sched_ext/state"),
        scx_ops: read_trimmed("/sys/kernel/sched_ext/root/ops"),
        scx_enable_seq: read_trimmed("/sys/kernel/sched_ext/enable_seq"),
    }
}

fn collect_irq_lines() -> Vec<IrqLine> {
    let Some(contents) = fs::read_to_string("/proc/interrupts").ok() else {
        return Vec::new();
    };

    parse_proc_interrupts(&contents).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_feature_labels_do_not_report_unconditional_controller_as_optional() {
        let labels = build_feature_labels();

        assert!(
            !labels.contains(&"autotune-controller"),
            "autotune-controller is an unconditional runtime path, not an optional feature"
        );
    }

    #[test]
    fn build_feature_labels_report_only_optional_integrations_or_default() {
        let labels = build_feature_labels();

        if cfg!(feature = "otel") || cfg!(feature = "wayland-probe") {
            if cfg!(feature = "otel") {
                assert!(labels.contains(&"otel"));
            }
            if cfg!(feature = "wayland-probe") {
                assert!(labels.contains(&"wayland-probe"));
            }
        } else {
            assert_eq!(labels, vec!["default"]);
        }
    }
}
