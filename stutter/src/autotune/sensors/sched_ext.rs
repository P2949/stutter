//! The kernel's sched_ext state as readings: whether a BPF scheduler is loaded, which one, and how
//! many times one was enabled, rejected or disturbed by CPU hotplug.
//!
//! The `enable_seq` counter changes every time any scheduler is enabled, so a delta during an
//! experiment means the scheduler under it changed. That is a reason to distrust the comparison and
//! never a tuning input; this reads files and loads, attaches and unloads nothing.
use std::{collections::BTreeMap, fs, path::PathBuf};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

fn number(root: &std::path::Path, name: &str) -> Option<u64> {
    fs::read_to_string(root.join(name))
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub struct SchedExtSensor {
    root: PathBuf,
    previous: Option<BTreeMap<String, u64>>,
    ops: Option<String>,
    health: SensorHealth,
}

impl SchedExtSensor {
    pub fn new(sys_root: PathBuf) -> Self {
        Self {
            root: sys_root.join("kernel/sched_ext"),
            previous: None,
            ops: None,
            health: SensorHealth::Healthy,
        }
    }

    fn state(&self) -> Option<String> {
        let text = fs::read_to_string(self.root.join("state")).ok()?;
        Some(text.trim().to_owned())
    }

    fn counters(&self) -> BTreeMap<String, u64> {
        ["enable_seq", "nr_rejected", "hotplug_seq"]
            .into_iter()
            .filter_map(|name| Some((name.to_owned(), number(&self.root, name)?)))
            .collect()
    }
}

impl Sensor for SchedExtSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "sched_ext",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "/sys/kernel/sched_ext (CONFIG_SCHED_CLASS_EXT)",
        }
    }

    fn probe(&self) -> Availability {
        if self.state().is_some() {
            Availability::Available
        } else {
            Availability::Unavailable(format!("{}/state is not readable", self.root.display()))
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
        match self.state() {
            None => degraded = Some("sched_ext state became unreadable".to_owned()),
            Some(state) => {
                let enabled = state == "enabled";
                values.insert("enabled".to_owned(), if enabled { 1.0 } else { 0.0 });
                let ops = enabled
                    .then(|| fs::read_to_string(self.root.join("root/ops")).ok())
                    .flatten()
                    .map(|text| text.trim().to_owned());
                // The scheduler's name changing is a change even if the counter did not move.
                if self.previous.is_some() && ops != self.ops {
                    values.insert("scheduler_changed".to_owned(), 1.0);
                }
                self.ops = ops;
                let now = self.counters();
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
                }
                self.previous = Some(now);
            }
        }
        self.health = degraded.map_or(SensorHealth::Healthy, SensorHealth::Degraded);
        SensorReading {
            sensor: "sched_ext".to_owned(),
            monotonic_ns,
            scope: SensorScope::System,
            quality: if values.contains_key("enabled") {
                ObjectiveSignalQuality::Direct
            } else {
                ObjectiveSignalQuality::Missing
            },
            values,
            lost,
            provenance: format!(
                "{} ops={}",
                self.root.display(),
                self.ops.as_deref().unwrap_or("none")
            ),
        }
    }

    fn health(&self) -> SensorHealth {
        self.health.clone()
    }

    fn stop(&mut self) {
        self.previous = None;
        self.ops = None;
    }
}

/// Why the scheduler under a running experiment changed in `readings`, if it did. A comparison
/// across a scheduler switch is not trustworthy; this is a safety signal, never a tuning input.
pub fn scheduler_change_event(readings: &[SensorReading]) -> Option<String> {
    let reading = readings
        .iter()
        .find(|reading| reading.sensor == "sched_ext")?;
    if reading.values.get("scheduler_changed").copied() == Some(1.0) {
        return Some("the sched_ext scheduler changed during the interval".to_owned());
    }
    let enabled = reading
        .values
        .get("enable_seq_delta")
        .copied()
        .filter(|d| *d > 0.0)?;
    Some(format!(
        "sched_ext was enabled {enabled} time(s) during the interval"
    ))
}

#[cfg(test)]
#[path = "sched_ext_tests.rs"]
mod tests;
