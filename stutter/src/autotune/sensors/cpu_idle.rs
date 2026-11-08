//! C-state residency and package energy as deltas.
//!
//! `cpuidle` residency (microseconds spent in each idle state, summed over CPUs) shows whether the
//! CPUs are sleeping deeply or kept shallow; RAPL package energy (microjoules) shows power draw
//! when the platform exposes it to this user. A missing or unreadable file is simply absent from the
//! reading: RAPL is commonly root-only and is never assumed. Observation only, never a decision input.
use std::{collections::BTreeMap, fs, path::PathBuf};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

/// Most CPUs and idle states read per sample.
const MAX_CPUS: usize = 4096;
const MAX_STATES: usize = 16;

pub struct CpuIdleSensor {
    cpu_root: PathBuf,
    powercap_root: PathBuf,
    previous: Option<BTreeMap<String, u64>>,
    health: SensorHealth,
}

impl CpuIdleSensor {
    pub fn new(system_root: &std::path::Path, class_root: &std::path::Path) -> Self {
        Self {
            cpu_root: system_root.join("cpu"),
            powercap_root: class_root.join("powercap"),
            previous: None,
            health: SensorHealth::Healthy,
        }
    }

    /// Residency per idle-state name summed over CPUs, and package energy when readable.
    fn totals(&self) -> Option<BTreeMap<String, u64>> {
        let mut totals: BTreeMap<String, u64> = BTreeMap::new();
        let mut any = false;
        for cpu in fs::read_dir(&self.cpu_root).ok()?.flatten().take(MAX_CPUS) {
            let name = cpu.file_name();
            let is_cpu = name
                .to_str()
                .and_then(|n| n.strip_prefix("cpu"))
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
            if !is_cpu {
                continue;
            }
            let Ok(states) = fs::read_dir(cpu.path().join("cpuidle")) else {
                continue;
            };
            for state in states.flatten().take(MAX_STATES) {
                let (Ok(label), Ok(time)) = (
                    fs::read_to_string(state.path().join("name")),
                    fs::read_to_string(state.path().join("time")),
                ) else {
                    continue;
                };
                let Ok(time) = time.trim().parse::<u64>() else {
                    continue;
                };
                any = true;
                let key = format!("idle_us_{}", label.trim().to_ascii_lowercase());
                *totals.entry(key).or_default() += time;
            }
        }
        if let Ok(zones) = fs::read_dir(&self.powercap_root) {
            // Top-level package zones only (`intel-rapl:0`), not sub-zones (`intel-rapl:0:0`).
            for zone in zones.flatten().take(MAX_STATES) {
                let name = zone.file_name().to_string_lossy().into_owned();
                if name.matches(':').count() != 1 {
                    continue;
                }
                if let Some(energy) = fs::read_to_string(zone.path().join("energy_uj"))
                    .ok()
                    .and_then(|t| t.trim().parse::<u64>().ok())
                {
                    any = true;
                    *totals.entry("package_energy_uj".to_owned()).or_default() += energy;
                }
            }
        }
        any.then_some(totals)
    }
}

impl Sensor for CpuIdleSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "cpu_idle",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "cpuidle residency (and optionally RAPL energy_uj)",
        }
    }

    fn probe(&self) -> Availability {
        if self.totals().is_some() {
            Availability::Available
        } else {
            Availability::Unavailable(format!(
                "no cpuidle residency or readable energy under {}",
                self.cpu_root.display()
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
        match self.totals() {
            None => degraded = Some("idle and energy counters became unreadable".to_owned()),
            Some(now) => {
                if let Some(before) = &self.previous {
                    for (key, after) in &now {
                        let Some(was) = before.get(key) else { continue };
                        if after >= was {
                            values.insert(format!("{key}_delta"), (after - was) as f64);
                        } else {
                            // A CPU went offline, or the RAPL counter wrapped: no delta is invented.
                            lost += 1;
                            degraded = Some(format!("{key} went backwards"));
                        }
                    }
                }
                self.previous = Some(now);
            }
        }
        self.health = degraded.map_or(SensorHealth::Healthy, SensorHealth::Degraded);
        SensorReading {
            sensor: "cpu_idle".to_owned(),
            monotonic_ns,
            scope: SensorScope::System,
            quality: if values.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Direct
            },
            values,
            lost,
            provenance: format!(
                "{}/cpu*/cpuidle and {}",
                self.cpu_root.display(),
                self.powercap_root.display()
            ),
        }
    }

    fn health(&self) -> SensorHealth {
        self.health.clone()
    }

    fn stop(&mut self) {
        self.previous = None;
    }
}

#[cfg(test)]
#[path = "cpu_idle_tests.rs"]
mod tests;
