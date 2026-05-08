use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::affinity::CpuMask;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyModel {
    pub online_cpus: CpuMask,
    pub cpus: Vec<CpuInfo>,
    pub cores: Vec<CoreInfo>,
    pub smt_siblings: BTreeMap<u32, Vec<u32>>,
    pub numa_nodes: BTreeMap<u32, Vec<u32>>,
    pub packages: BTreeMap<u32, Vec<u32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CpuInfo {
    pub cpu: u32,
    pub core_id: Option<u32>,
    pub package_id: Option<u32>,
    pub numa_node: Option<u32>,
    pub max_mhz: Option<u64>,
    pub is_online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreInfo {
    pub core_id: Option<u32>,
    pub package_id: Option<u32>,
    pub numa_node: Option<u32>,
    pub cpus: Vec<u32>,
    pub max_mhz: Option<u64>,
    pub is_online: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CoreKey {
    package_id: Option<u32>,
    core_id: Option<u32>,
    numa_node: Option<u32>,
}

impl TopologyModel {
    pub fn read() -> anyhow::Result<Self> {
        Self::read_at(Path::new("/sys/devices/system"))
    }

    pub fn read_at(sys_root: &Path) -> anyhow::Result<Self> {
        let cpu_root = sys_root.join("cpu");
        let online_cpus = read_online_cpus_at(&cpu_root)?;
        let online_set = cpu_mask_to_set(&online_cpus);

        let mut cpus = Vec::new();
        let mut smt_siblings: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let mut numa_nodes: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        let mut packages: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

        for cpu in discover_cpu_ids_at(&cpu_root)? {
            let cpu_dir = cpu_root.join(format!("cpu{cpu}"));
            let topology_dir = cpu_dir.join("topology");

            let core_id = read_optional_u32(&topology_dir.join("core_id"));
            let package_id = read_optional_u32(&topology_dir.join("physical_package_id"));
            let numa_node = read_cpu_numa_node(&cpu_dir);
            let max_mhz = read_cpu_max_mhz(&cpu_dir);
            let is_online = online_set.contains(&cpu);

            if let Some(package_id) = package_id {
                packages.entry(package_id).or_default().push(cpu);
            }

            if let Some(numa_node) = numa_node {
                numa_nodes.entry(numa_node).or_default().push(cpu);
            }

            let siblings = read_cpu_list(&topology_dir.join("thread_siblings_list"))
                .unwrap_or_else(|| vec![cpu])
                .into_iter()
                .filter(|sibling| online_set.contains(sibling))
                .collect::<Vec<_>>();
            smt_siblings.insert(cpu, sorted_unique(siblings));

            cpus.push(CpuInfo {
                cpu,
                core_id,
                package_id,
                numa_node,
                max_mhz,
                is_online,
            });
        }

        cpus.sort_by_key(|cpu| cpu.cpu);

        for cpus in packages.values_mut() {
            *cpus = sorted_unique(std::mem::take(cpus));
        }

        for cpus in numa_nodes.values_mut() {
            *cpus = sorted_unique(std::mem::take(cpus));
        }

        let cores = build_core_infos(&cpus);

        Ok(Self {
            online_cpus,
            cpus,
            cores,
            smt_siblings,
            numa_nodes,
            packages,
        })
    }

    pub fn online_cpu_ids(&self) -> Vec<u32> {
        self.cpus
            .iter()
            .filter(|cpu| cpu.is_online)
            .map(|cpu| cpu.cpu)
            .collect()
    }

    pub fn online_core_count(&self) -> usize {
        self.cores.iter().filter(|core| core.is_online).count()
    }

    pub fn cpu_info(&self, cpu: u32) -> Option<&CpuInfo> {
        self.cpus.iter().find(|info| info.cpu == cpu)
    }
}

fn build_core_infos(cpus: &[CpuInfo]) -> Vec<CoreInfo> {
    let mut grouped: BTreeMap<CoreKey, Vec<&CpuInfo>> = BTreeMap::new();

    for cpu in cpus {
        grouped
            .entry(CoreKey {
                package_id: cpu.package_id,
                core_id: cpu.core_id,
                numa_node: cpu.numa_node,
            })
            .or_default()
            .push(cpu);
    }

    grouped
        .into_iter()
        .map(|(key, members)| {
            let cpus = sorted_unique(members.iter().map(|cpu| cpu.cpu).collect());
            let max_mhz = members.iter().filter_map(|cpu| cpu.max_mhz).max();
            let is_online = members.iter().any(|cpu| cpu.is_online);

            CoreInfo {
                core_id: key.core_id,
                package_id: key.package_id,
                numa_node: key.numa_node,
                cpus,
                max_mhz,
                is_online,
            }
        })
        .collect()
}

fn read_online_cpus_at(cpu_root: &Path) -> anyhow::Result<CpuMask> {
    let online_path = cpu_root.join("online");
    let data = fs::read_to_string(&online_path)
        .with_context(|| format!("failed to read online CPU list {}", online_path.display()))?;
    CpuMask::parse(data.trim())
        .with_context(|| format!("failed to parse online CPU list {}", online_path.display()))
}

fn discover_cpu_ids_at(cpu_root: &Path) -> anyhow::Result<Vec<u32>> {
    let mut cpus = Vec::new();

    for entry in fs::read_dir(cpu_root)
        .with_context(|| format!("failed to read CPU sysfs directory {}", cpu_root.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry under {}", cpu_root.display()))?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(cpu_suffix) = name.strip_prefix("cpu") else {
            continue;
        };
        let Ok(cpu) = cpu_suffix.parse::<u32>() else {
            continue;
        };

        if entry.path().is_dir() {
            cpus.push(cpu);
        }
    }

    cpus.sort_unstable();
    cpus.dedup();

    Ok(cpus)
}

fn read_optional_u32(path: &Path) -> Option<u32> {
    fs::read_to_string(path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
}

fn read_cpu_max_mhz(cpu_dir: &Path) -> Option<u64> {
    let cpufreq_dir = cpu_dir.join("cpufreq");

    read_optional_mhz_as_u64(&cpufreq_dir.join("cpuinfo_max_freq"))
        .or_else(|| read_optional_mhz_as_u64(&cpufreq_dir.join("scaling_max_freq")))
}

fn read_optional_mhz_as_u64(path: &Path) -> Option<u64> {
    let raw = fs::read_to_string(path).ok()?;
    let value = raw.trim();

    if value.is_empty() {
        return None;
    }

    if let Ok(khz) = value.parse::<u64>() {
        return Some(khz / 1000);
    }

    let mhz = value.parse::<f64>().ok()?;
    if !mhz.is_finite() || mhz < 0.0 {
        return None;
    }

    Some(mhz.round() as u64)
}

fn read_cpu_numa_node(cpu_dir: &Path) -> Option<u32> {
    let entries = fs::read_dir(cpu_dir).ok()?;

    entries.flatten().find_map(|entry| {
        let name = entry.file_name();
        let name = name.to_str()?;
        let node_suffix = name.strip_prefix("node")?;
        node_suffix.parse::<u32>().ok()
    })
}

fn read_cpu_list(path: &Path) -> Option<Vec<u32>> {
    let data = fs::read_to_string(path).ok()?;
    parse_cpu_list(data.trim()).ok()
}

pub(crate) fn parse_cpu_list(value: &str) -> anyhow::Result<Vec<u32>> {
    let mut cpus = Vec::new();

    for part in value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (start, end) = match part.split_once('-') {
            Some((start, end)) => (parse_cpu_id(start)?, parse_cpu_id(end)?),
            None => {
                let cpu = parse_cpu_id(part)?;
                (cpu, cpu)
            }
        };

        if start > end {
            anyhow::bail!("invalid CPU range {part}: start is greater than end");
        }

        cpus.extend(start..=end);
    }

    Ok(sorted_unique(cpus))
}

fn parse_cpu_id(value: &str) -> anyhow::Result<u32> {
    value
        .trim()
        .parse::<u32>()
        .with_context(|| format!("invalid CPU id {value:?}"))
}

pub(crate) fn cpu_mask_to_vec(mask: &CpuMask) -> Vec<u32> {
    parse_cpu_list(&mask.to_range_string()).unwrap_or_default()
}

fn cpu_mask_to_set(mask: &CpuMask) -> BTreeSet<u32> {
    cpu_mask_to_vec(mask).into_iter().collect()
}

pub(crate) fn cpus_to_range_string(cpus: &[u32]) -> String {
    let cpus = sorted_unique(cpus.to_vec());
    let mut ranges = Vec::new();
    let mut idx = 0;

    while idx < cpus.len() {
        let start = cpus[idx];
        let mut end = start;
        idx += 1;

        while idx < cpus.len() && cpus[idx] == end + 1 {
            end = cpus[idx];
            idx += 1;
        }

        if start == end {
            ranges.push(start.to_string());
        } else {
            ranges.push(format!("{start}-{end}"));
        }
    }

    ranges.join(",")
}

pub(crate) fn cpus_to_mask(cpus: &[u32]) -> Option<CpuMask> {
    let cpus = sorted_unique(cpus.to_vec());
    if cpus.is_empty() {
        return None;
    }

    CpuMask::parse(&cpus_to_range_string(&cpus)).ok()
}

pub(crate) fn sorted_unique(mut cpus: Vec<u32>) -> Vec<u32> {
    cpus.sort_unstable();
    cpus.dedup();
    cpus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_lists_with_ranges_and_singletons() {
        assert_eq!(parse_cpu_list("0-2,4,6-7").unwrap(), vec![0, 1, 2, 4, 6, 7]);
    }

    #[test]
    fn rejects_reversed_cpu_range() {
        assert!(parse_cpu_list("4-2").is_err());
    }

    #[test]
    fn renders_cpu_ranges() {
        assert_eq!(cpus_to_range_string(&[0, 1, 2, 4, 6, 7]), "0-2,4,6-7");
    }

    #[test]
    fn reads_topology_from_fake_sysfs() {
        let sys_root = temp_dir("topology-fake-sysfs");
        let cpu_root = sys_root.join("cpu");
        fs::create_dir_all(&cpu_root).unwrap();
        fs::write(cpu_root.join("online"), "0-3\n").unwrap();

        write_fake_cpu(
            &cpu_root,
            FakeCpu {
                cpu: 0,
                core_id: 0,
                package_id: 0,
                node: 0,
                siblings: "0,2",
                max_khz: 4_900_000,
            },
        );
        write_fake_cpu(
            &cpu_root,
            FakeCpu {
                cpu: 1,
                core_id: 1,
                package_id: 0,
                node: 0,
                siblings: "1,3",
                max_khz: 4_900_000,
            },
        );
        write_fake_cpu(
            &cpu_root,
            FakeCpu {
                cpu: 2,
                core_id: 0,
                package_id: 0,
                node: 0,
                siblings: "0,2",
                max_khz: 4_900_000,
            },
        );
        write_fake_cpu(
            &cpu_root,
            FakeCpu {
                cpu: 3,
                core_id: 1,
                package_id: 0,
                node: 0,
                siblings: "1,3",
                max_khz: 4_900_000,
            },
        );

        let topology = TopologyModel::read_at(&sys_root).unwrap();

        assert_eq!(topology.online_cpus.to_range_string(), "0-3");
        assert_eq!(topology.online_cpu_ids(), vec![0, 1, 2, 3]);
        assert_eq!(topology.cpus.len(), 4);
        assert_eq!(topology.cores.len(), 2);
        assert_eq!(topology.online_core_count(), 2);
        assert_eq!(topology.smt_siblings.get(&0), Some(&vec![0, 2]));
        assert_eq!(topology.smt_siblings.get(&1), Some(&vec![1, 3]));
        assert_eq!(topology.numa_nodes.get(&0), Some(&vec![0, 1, 2, 3]));
        assert_eq!(topology.packages.get(&0), Some(&vec![0, 1, 2, 3]));

        let cpu0 = topology.cpu_info(0).unwrap();
        assert_eq!(cpu0.core_id, Some(0));
        assert_eq!(cpu0.package_id, Some(0));
        assert_eq!(cpu0.numa_node, Some(0));
        assert_eq!(cpu0.max_mhz, Some(4900));
        assert!(cpu0.is_online);

        fs::remove_dir_all(sys_root).ok();
    }

    #[test]
    fn marks_discovered_offline_cpus() {
        let sys_root = temp_dir("topology-offline-cpu");
        let cpu_root = sys_root.join("cpu");
        fs::create_dir_all(&cpu_root).unwrap();
        fs::write(cpu_root.join("online"), "0\n").unwrap();

        write_fake_cpu(
            &cpu_root,
            FakeCpu {
                cpu: 0,
                core_id: 0,
                package_id: 0,
                node: 0,
                siblings: "0",
                max_khz: 4_000_000,
            },
        );
        write_fake_cpu(
            &cpu_root,
            FakeCpu {
                cpu: 1,
                core_id: 1,
                package_id: 0,
                node: 0,
                siblings: "1",
                max_khz: 4_000_000,
            },
        );

        let topology = TopologyModel::read_at(&sys_root).unwrap();

        assert_eq!(topology.online_cpu_ids(), vec![0]);
        assert!(topology.cpu_info(0).unwrap().is_online);
        assert!(!topology.cpu_info(1).unwrap().is_online);

        fs::remove_dir_all(sys_root).ok();
    }

    struct FakeCpu<'a> {
        cpu: u32,
        core_id: u32,
        package_id: u32,
        node: u32,
        siblings: &'a str,
        max_khz: u64,
    }

    fn write_fake_cpu(cpu_root: &Path, fake: FakeCpu<'_>) {
        let cpu_dir = cpu_root.join(format!("cpu{}", fake.cpu));
        let topology_dir = cpu_dir.join("topology");
        let cpufreq_dir = cpu_dir.join("cpufreq");
        fs::create_dir_all(&topology_dir).unwrap();
        fs::create_dir_all(&cpufreq_dir).unwrap();
        fs::create_dir_all(cpu_dir.join(format!("node{}", fake.node))).unwrap();

        fs::write(topology_dir.join("core_id"), format!("{}\n", fake.core_id)).unwrap();
        fs::write(
            topology_dir.join("physical_package_id"),
            format!("{}\n", fake.package_id),
        )
        .unwrap();
        fs::write(topology_dir.join("thread_siblings_list"), fake.siblings).unwrap();
        fs::write(
            cpufreq_dir.join("cpuinfo_max_freq"),
            format!("{}\n", fake.max_khz),
        )
        .unwrap();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }
}
