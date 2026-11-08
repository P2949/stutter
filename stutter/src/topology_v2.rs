//! Topology v2 (package W8): the cache, die, capacity, NUMA-distance and PCI-locality facts that
//! the base `TopologyModel` does not carry, read from native sysfs.
//!
//! Nothing here is guessed. A last-level-cache domain is a distinct `shared_cpu_list` of the
//! highest cache level the kernel reports; a heterogeneous core class exists only when the kernel
//! exposes `cpu_capacity`; a larger-cache region exists only when LLC domains report different
//! sizes. `max_mhz` is never used to identify a core type or a V-Cache part. The fingerprint changes
//! when anything here changes, so a prior experiment is not carried across a topology change.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::{
    daemon::state::daemon_profile_stable_hash,
    topology::{TopologyModel, parse_cpu_list, sorted_unique},
};

/// A group of CPUs sharing a last-level cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlcDomain {
    pub level: u32,
    pub cpus: Vec<u32>,
    pub size_kib: Option<u64>,
}

/// CPUs of one kernel-reported capacity (1024 is the largest on a heterogeneous system).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacityClass {
    pub capacity: u64,
    pub cpus: Vec<u32>,
}

/// A PCI device's reported locality.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLocality {
    pub device: String,
    pub numa_node: Option<i32>,
    pub local_cpus: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyV2 {
    pub llc_domains: Vec<LlcDomain>,
    /// Empty on a homogeneous system: no heterogeneity is inferred.
    pub capacity_classes: Vec<CapacityClass>,
    /// CPUs of the largest-cache LLC domains, only when domains differ in verified size.
    pub larger_cache_cpus: Vec<u32>,
    pub die_groups: BTreeMap<u32, Vec<u32>>,
    /// CPUs that share one cpufreq policy.
    pub frequency_domains: Vec<Vec<u32>>,
    /// `numa_distance[from][to]` from the kernel's node distance files.
    pub numa_distance: BTreeMap<u32, Vec<u32>>,
    pub gpu_locality: Vec<DeviceLocality>,
    /// CPUs this process may actually use (a container cpuset narrows it).
    pub effective_cpus: Vec<u32>,
    pub fingerprint: String,
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

/// `"32K"`, `"32768K"`, `"96M"` as KiB.
fn parse_cache_size_kib(text: &str) -> Option<u64> {
    let text = text.trim();
    let (digits, unit) = text.split_at(text.find(|c: char| !c.is_ascii_digit())?);
    let value: u64 = digits.parse().ok()?;
    match unit.trim() {
        "K" | "k" => Some(value),
        "M" | "m" => value.checked_mul(1024),
        "G" | "g" => value.checked_mul(1024 * 1024),
        _ => None,
    }
}

fn llc_domains(cpu_root: &Path, cpus: &[u32]) -> Vec<LlcDomain> {
    // Highest level seen on any CPU that reports a data or unified cache.
    let mut per_cpu: Vec<(u32, u32, Vec<u32>, Option<u64>)> = Vec::new();
    for &cpu in cpus {
        let cache_dir = cpu_root.join(format!("cpu{cpu}/cache"));
        let Ok(entries) = fs::read_dir(&cache_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let (Some(level), Some(kind)) = (
                read_trimmed(&path.join("level")).and_then(|v| v.parse::<u32>().ok()),
                read_trimmed(&path.join("type")),
            ) else {
                continue;
            };
            if kind != "Data" && kind != "Unified" {
                continue;
            }
            let Some(shared) = read_trimmed(&path.join("shared_cpu_list"))
                .and_then(|text| parse_cpu_list(&text).ok())
            else {
                continue;
            };
            let size =
                read_trimmed(&path.join("size")).and_then(|text| parse_cache_size_kib(&text));
            per_cpu.push((cpu, level, sorted_unique(shared), size));
        }
    }
    let Some(top) = per_cpu.iter().map(|entry| entry.1).max() else {
        return Vec::new();
    };
    let mut domains: BTreeMap<Vec<u32>, Option<u64>> = BTreeMap::new();
    for (_, level, shared, size) in per_cpu {
        if level == top {
            domains.entry(shared).or_insert(size);
        }
    }
    domains
        .into_iter()
        .map(|(cpus, size_kib)| LlcDomain {
            level: top,
            cpus,
            size_kib,
        })
        .collect()
}

fn capacity_classes(cpu_root: &Path, cpus: &[u32]) -> Vec<CapacityClass> {
    let mut by_capacity: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
    for &cpu in cpus {
        if let Some(capacity) = read_trimmed(&cpu_root.join(format!("cpu{cpu}/cpu_capacity")))
            .and_then(|text| text.parse::<u64>().ok())
        {
            by_capacity.entry(capacity).or_default().push(cpu);
        }
    }
    // One capacity value everywhere is a homogeneous system, not a core class.
    if by_capacity.len() < 2 {
        return Vec::new();
    }
    by_capacity
        .into_iter()
        .map(|(capacity, cpus)| CapacityClass {
            capacity,
            cpus: sorted_unique(cpus),
        })
        .collect()
}

fn larger_cache_cpus(domains: &[LlcDomain]) -> Vec<u32> {
    let sizes: BTreeSet<u64> = domains.iter().filter_map(|d| d.size_kib).collect();
    if sizes.len() < 2 || domains.iter().any(|d| d.size_kib.is_none()) {
        return Vec::new();
    }
    let largest = sizes.iter().next_back().copied();
    sorted_unique(
        domains
            .iter()
            .filter(|domain| domain.size_kib == largest)
            .flat_map(|domain| domain.cpus.iter().copied())
            .collect(),
    )
}

fn die_groups(cpu_root: &Path, cpus: &[u32]) -> BTreeMap<u32, Vec<u32>> {
    let mut groups: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for &cpu in cpus {
        if let Some(die) = read_trimmed(&cpu_root.join(format!("cpu{cpu}/topology/die_id")))
            .and_then(|text| text.parse::<u32>().ok())
        {
            groups.entry(die).or_default().push(cpu);
        }
    }
    groups
}

fn frequency_domains(cpu_root: &Path, cpus: &[u32]) -> Vec<Vec<u32>> {
    let mut domains: BTreeSet<Vec<u32>> = BTreeSet::new();
    for &cpu in cpus {
        if let Some(related) =
            read_trimmed(&cpu_root.join(format!("cpu{cpu}/cpufreq/related_cpus"))).map(|text| {
                text.split_whitespace()
                    .filter_map(|c| c.parse().ok())
                    .collect::<Vec<u32>>()
            })
        {
            domains.insert(sorted_unique(related));
        }
    }
    domains.into_iter().collect()
}

fn numa_distance(system_root: &Path) -> BTreeMap<u32, Vec<u32>> {
    let mut distances = BTreeMap::new();
    let Ok(entries) = fs::read_dir(system_root.join("node")) else {
        return distances;
    };
    for entry in entries.flatten() {
        let Some(node) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix("node"))
            .and_then(|digits| digits.parse::<u32>().ok())
        else {
            continue;
        };
        if let Some(row) = read_trimmed(&entry.path().join("distance")) {
            distances.insert(
                node,
                row.split_whitespace()
                    .filter_map(|value| value.parse().ok())
                    .collect(),
            );
        }
    }
    distances
}

/// DRM cards and their PCI locality, from `class/drm/card*/device`.
fn gpu_locality(sys_class_root: &Path) -> Vec<DeviceLocality> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(sys_class_root.join("drm")) else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // `card0`, not `card0-HDMI-A-1` connectors or render nodes.
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let device_dir = entry.path().join("device");
        found.push(DeviceLocality {
            device: name,
            numa_node: read_trimmed(&device_dir.join("numa_node")).and_then(|t| t.parse().ok()),
            local_cpus: read_trimmed(&device_dir.join("local_cpulist"))
                .and_then(|text| parse_cpu_list(&text).ok())
                .map(sorted_unique)
                .unwrap_or_default(),
        });
    }
    found.sort_by(|a, b| a.device.cmp(&b.device));
    found
}

fn fingerprint(v2: &TopologyV2, base: &TopologyModel) -> String {
    let rendered = [
        format!("online={:?}", base.online_cpu_ids()),
        format!("llc={:?}", v2.llc_domains),
        format!("capacity={:?}", v2.capacity_classes),
        format!("dies={:?}", v2.die_groups),
        format!("freq={:?}", v2.frequency_domains),
        format!("numa={:?}", v2.numa_distance),
        format!("gpu={:?}", v2.gpu_locality),
        format!("effective={:?}", v2.effective_cpus),
    ];
    daemon_profile_stable_hash(rendered.iter().map(String::as_str))
}

impl TopologyV2 {
    /// Reads v2 facts from `system_root` (`/sys/devices/system`) and `class_root` (`/sys/class`).
    /// `effective` is the set of CPUs this process may use (its cpuset).
    pub fn read_at(
        system_root: &Path,
        class_root: &Path,
        base: &TopologyModel,
        effective: Vec<u32>,
    ) -> Self {
        let cpu_root = system_root.join("cpu");
        let online = base.online_cpu_ids();
        let llc = llc_domains(&cpu_root, &online);
        let mut v2 = Self {
            larger_cache_cpus: larger_cache_cpus(&llc),
            llc_domains: llc,
            capacity_classes: capacity_classes(&cpu_root, &online),
            die_groups: die_groups(&cpu_root, &online)
                .into_iter()
                .map(|(die, cpus)| (die, sorted_unique(cpus)))
                .collect(),
            frequency_domains: frequency_domains(&cpu_root, &online),
            numa_distance: numa_distance(system_root),
            gpu_locality: gpu_locality(class_root),
            effective_cpus: sorted_unique(effective),
            fingerprint: String::new(),
        };
        v2.fingerprint = fingerprint(&v2, base);
        v2
    }

    /// Whether `cpus` stay inside one LLC domain: a placement that does not split a cache group.
    pub fn within_one_llc(&self, cpus: &[u32]) -> bool {
        self.llc_domains
            .iter()
            .any(|domain| cpus.iter().all(|cpu| domain.cpus.contains(cpu)))
    }

    /// Whether a prior experiment recorded under `fingerprint` still applies here.
    pub fn matches(&self, fingerprint: &str) -> bool {
        self.fingerprint == fingerprint
    }
}

#[cfg(test)]
#[path = "topology_v2_tests.rs"]
mod tests;
