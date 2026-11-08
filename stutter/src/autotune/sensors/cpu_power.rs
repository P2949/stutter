//! CPU power context from sysfs: driver, governor, energy preference, requested versus actual
//! frequency, boost, thermal-throttle events and AC state.
//!
//! This is context for diagnosis, never an outcome and never a trigger for a write. Missing files
//! are missing readings, not zeros: a host without RAPL or a throttle counter reports less, and
//! no joules or throttling are invented.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

fn read_u64(path: &Path) -> Option<u64> {
    read_text(path)?.parse().ok()
}

pub struct CpuPowerSensor {
    /// `/sys/devices/system`.
    system_root: PathBuf,
    /// `/sys/class`.
    class_root: PathBuf,
    previous_throttle: Option<u64>,
    health: SensorHealth,
}

impl CpuPowerSensor {
    pub fn new(system_root: &Path, class_root: &Path) -> Self {
        Self {
            system_root: system_root.to_path_buf(),
            class_root: class_root.to_path_buf(),
            previous_throttle: None,
            health: SensorHealth::Healthy,
        }
    }

    fn policies(&self) -> Vec<(u32, PathBuf)> {
        let Ok(entries) = fs::read_dir(self.system_root.join("cpu/cpufreq")) else {
            return Vec::new();
        };
        let mut policies: Vec<(u32, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let number = entry
                    .file_name()
                    .to_str()?
                    .strip_prefix("policy")?
                    .parse::<u32>()
                    .ok()?;
                Some((number, entry.path()))
            })
            .collect();
        policies.sort();
        policies
    }

    /// Sum of core and package throttle counters over every CPU that exposes them.
    fn throttle_total(&self) -> Option<u64> {
        let entries = fs::read_dir(self.system_root.join("cpu")).ok()?;
        let mut total = None;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("cpu") || !name[3..].bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            for file in ["core_throttle_count", "package_throttle_count"] {
                if let Some(count) = read_u64(&entry.path().join("thermal_throttle").join(file)) {
                    total = Some(total.unwrap_or(0u64).saturating_add(count));
                }
            }
        }
        total
    }

    fn on_ac(&self) -> Option<bool> {
        let entries = fs::read_dir(self.class_root.join("power_supply")).ok()?;
        let mut any_mains = false;
        for entry in entries.flatten() {
            if read_text(&entry.path().join("type")).as_deref() == Some("Mains") {
                any_mains = true;
                if read_u64(&entry.path().join("online")) == Some(1) {
                    return Some(true);
                }
            }
        }
        any_mains.then_some(false)
    }
}

impl Sensor for CpuPowerSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "cpu_power",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "cpufreq sysfs (a cpufreq driver)",
        }
    }

    fn probe(&self) -> Availability {
        if self.policies().is_empty() {
            Availability::Unavailable(format!(
                "no cpufreq policy under {}",
                self.system_root.join("cpu/cpufreq").display()
            ))
        } else {
            Availability::Available
        }
    }

    fn start(&mut self) -> Result<(), String> {
        match self.probe() {
            Availability::Available => {
                self.previous_throttle = None;
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
        let mut combos: BTreeMap<String, u32> = BTreeMap::new();
        let mut lost = 0;
        let mut degraded = None;
        for (number, dir) in self.policies() {
            let (cur, max, min) = (
                read_u64(&dir.join("scaling_cur_freq")),
                read_u64(&dir.join("scaling_max_freq")),
                read_u64(&dir.join("scaling_min_freq")),
            );
            for (name, value) in [("cur_khz", cur), ("max_khz", max), ("min_khz", min)] {
                if let Some(value) = value {
                    values.insert(format!("policy{number}_{name}"), value as f64);
                }
            }
            if let (Some(cur), Some(max)) = (cur, max)
                && max > 0
            {
                values.insert(
                    format!("policy{number}_cur_over_max"),
                    cur as f64 / max as f64,
                );
            }
            let describe =
                |file: &str| read_text(&dir.join(file)).unwrap_or_else(|| "unknown".to_owned());
            *combos
                .entry(format!(
                    "{}/{}/{}",
                    describe("scaling_driver"),
                    describe("scaling_governor"),
                    describe("energy_performance_preference")
                ))
                .or_default() += 1;
        }
        if let Some(no_turbo) = read_u64(&self.system_root.join("cpu/intel_pstate/no_turbo")) {
            values.insert(
                "boost_enabled".to_owned(),
                if no_turbo == 0 { 1.0 } else { 0.0 },
            );
        } else if let Some(boost) = read_u64(&self.system_root.join("cpu/cpufreq/boost")) {
            values.insert("boost_enabled".to_owned(), boost as f64);
        }
        if let Some(on_ac) = self.on_ac() {
            values.insert("on_ac".to_owned(), if on_ac { 1.0 } else { 0.0 });
        }
        match (self.throttle_total(), self.previous_throttle) {
            (Some(now), Some(before)) if now >= before => {
                values.insert("throttle_events_delta".to_owned(), (now - before) as f64);
                self.previous_throttle = Some(now);
            }
            (Some(now), Some(_)) => {
                // A counter that went backwards was reset: no delta, counted as loss.
                lost += 1;
                degraded = Some("a throttle counter went backwards".to_owned());
                self.previous_throttle = Some(now);
            }
            (Some(now), None) => self.previous_throttle = Some(now),
            (None, _) => {}
        }
        self.health = degraded.map_or(SensorHealth::Healthy, SensorHealth::Degraded);
        let provenance = combos
            .iter()
            .map(|(combo, count)| format!("{combo} x{count}"))
            .collect::<Vec<_>>()
            .join("; ");
        SensorReading {
            sensor: "cpu_power".to_owned(),
            monotonic_ns,
            scope: SensorScope::System,
            quality: if values.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Direct
            },
            values,
            lost,
            provenance: format!("driver/governor/epp: {provenance}"),
        }
    }

    fn health(&self) -> SensorHealth {
        self.health.clone()
    }

    fn stop(&mut self) {
        self.previous_throttle = None;
    }
}

/// Whether the CPU throttled for thermal reasons in `readings`' interval. A comparison that ran
/// through throttling measured the heat, not the candidate, so the runtime invalidates it.
pub fn thermal_guard_event(readings: &[SensorReading]) -> Option<String> {
    let reading = readings
        .iter()
        .find(|reading| reading.sensor == "cpu_power")?;
    let events = reading
        .values
        .get("throttle_events_delta")
        .copied()
        .filter(|events| *events > 0.0)?;
    Some(format!(
        "{events} thermal throttle event(s) in the interval"
    ))
}
