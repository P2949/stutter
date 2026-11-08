//! Automatic-NUMA-balancing fault and migration accounting, and a per-process `numa_maps` reader.
//!
//! `NumaFaultSensor` reads the system counters in `/proc/vmstat` that show how hard the kernel is
//! working to move pages (hinting faults, local faults, migrated pages) as deltas. `parse_numa_maps`
//! reads one process's pages per node. Moving a thread is not moving its memory, and these only
//! observe: they never migrate, bind or change a policy, and they are never a decision input.
use std::{collections::BTreeMap, fs, path::PathBuf};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

const COUNTERS: [&str; 6] = [
    "numa_hint_faults",
    "numa_hint_faults_local",
    "numa_pages_migrated",
    "pgmigrate_success",
    "pgmigrate_fail",
    "numa_pte_updates",
];

pub struct NumaFaultSensor {
    vmstat: PathBuf,
    previous: Option<BTreeMap<String, u64>>,
    health: SensorHealth,
}

impl NumaFaultSensor {
    pub fn new(proc_root: PathBuf) -> Self {
        Self {
            vmstat: proc_root.join("vmstat"),
            previous: None,
            health: SensorHealth::Healthy,
        }
    }

    fn counters(&self) -> Option<BTreeMap<String, u64>> {
        let text = fs::read_to_string(&self.vmstat).ok()?;
        let counters: BTreeMap<String, u64> = text
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once(' ')?;
                COUNTERS
                    .contains(&key)
                    .then(|| Some((key.to_owned(), value.trim().parse().ok()?)))
                    .flatten()
            })
            .collect();
        // Without NUMA balancing built in the `numa_*` counters are absent: nothing to report.
        counters
            .contains_key("numa_hint_faults")
            .then_some(counters)
    }
}

impl Sensor for NumaFaultSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "numa_faults",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "/proc/vmstat with CONFIG_NUMA_BALANCING counters",
        }
    }

    fn probe(&self) -> Availability {
        if self.counters().is_some() {
            Availability::Available
        } else {
            Availability::Unavailable(format!(
                "{} has no numa_hint_faults counter",
                self.vmstat.display()
            ))
        }
    }

    fn start(&mut self) -> Result<(), String> {
        match self.probe() {
            Availability::Available => {
                self.previous = None;
                Ok(())
            }
            Availability::Unavailable(reason) => {
                self.health = SensorHealth::Failed(reason.clone());
                Err(reason)
            }
        }
    }

    fn sample(&mut self, monotonic_ns: u64) -> SensorReading {
        let mut values = BTreeMap::new();
        let mut lost = 0;
        let mut degraded = None;
        match self.counters() {
            None => degraded = Some("vmstat NUMA counters became unreadable".to_owned()),
            Some(now) => {
                if let Some(before) = &self.previous {
                    for (key, after) in &now {
                        let Some(was) = before.get(key) else { continue };
                        if after >= was {
                            values.insert(format!("{key}_delta"), (after - was) as f64);
                        } else {
                            lost += 1;
                            degraded = Some(format!("{key} went backwards"));
                        }
                    }
                    let faults = values.get("numa_hint_faults_delta").copied().unwrap_or(0.0);
                    let local = values
                        .get("numa_hint_faults_local_delta")
                        .copied()
                        .unwrap_or(0.0);
                    if faults > 0.0 {
                        values.insert("hint_fault_local_ratio".to_owned(), local / faults);
                    }
                }
                self.previous = Some(now);
            }
        }
        self.health = degraded.map_or(SensorHealth::Healthy, SensorHealth::Degraded);
        SensorReading {
            sensor: "numa_faults".to_owned(),
            monotonic_ns,
            scope: SensorScope::System,
            quality: if values.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Direct
            },
            values,
            lost,
            provenance: self.vmstat.display().to_string(),
        }
    }

    fn health(&self) -> SensorHealth {
        self.health.clone()
    }

    fn stop(&mut self) {
        self.previous = None;
    }
}

/// One process's resident pages per NUMA node, from `/proc/<pid>/numa_maps`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NumaMapsSummary {
    pub pages_by_node: BTreeMap<u32, u64>,
    pub anon_pages: u64,
    pub file_pages: u64,
    pub dirty_pages: u64,
    pub mappings: usize,
}

/// Parses `numa_maps` text. A token that does not parse is skipped, never counted.
pub fn parse_numa_maps(text: &str) -> NumaMapsSummary {
    let mut summary = NumaMapsSummary::default();
    for line in text.lines() {
        let mut tokens = line.split_whitespace();
        if tokens.next().is_none() {
            continue;
        }
        summary.mappings += 1;
        for token in tokens {
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            let Ok(count) = value.parse::<u64>() else {
                continue;
            };
            match (
                key.strip_prefix('N').and_then(|n| n.parse::<u32>().ok()),
                key,
            ) {
                (Some(node), _) => *summary.pages_by_node.entry(node).or_default() += count,
                (None, "anon") => summary.anon_pages += count,
                (None, "file") => summary.file_pages += count,
                (None, "dirty") => summary.dirty_pages += count,
                _ => {}
            }
        }
    }
    summary
}

impl NumaMapsSummary {
    /// Share of the process's pages on `node`, or `None` when it has no counted pages.
    pub fn share_on(&self, node: u32) -> Option<f64> {
        let total: u64 = self.pages_by_node.values().sum();
        (total > 0).then(|| *self.pages_by_node.get(&node).unwrap_or(&0) as f64 / total as f64)
    }
}

#[cfg(test)]
#[path = "numa_faults_tests.rs"]
mod tests;
