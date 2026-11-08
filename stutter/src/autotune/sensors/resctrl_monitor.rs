//! Cache occupancy and memory bandwidth from resctrl monitoring (Intel RDT, AMD QoS).
//!
//! `mon_data/mon_L3_<id>/` holds `llc_occupancy` (bytes now, a gauge) and `mbm_total_bytes` /
//! `mbm_local_bytes` (counters). Hardware that cannot report a value writes `Unavailable` or
//! `Error`; those are counted as lost for that read and never turned into a zero. Allocation (CAT,
//! MBA) is a separate, riskier thing this does not do: it only reads.
use std::{collections::BTreeMap, fs, path::PathBuf};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

/// Most L3 monitoring domains read.
const MAX_DOMAINS: usize = 256;
const COUNTERS: [(&str, &str); 2] = [
    ("mbm_total_bytes", "mbm_total_bytes"),
    ("mbm_local_bytes", "mbm_local_bytes"),
];

pub struct ResctrlMonitorSensor {
    mon_data: PathBuf,
    previous: Option<BTreeMap<String, u64>>,
    health: SensorHealth,
}

impl ResctrlMonitorSensor {
    pub fn new(fs_root: &std::path::Path) -> Self {
        Self {
            mon_data: fs_root.join("resctrl/mon_data"),
            previous: None,
            health: SensorHealth::Healthy,
        }
    }

    /// Summed occupancy gauge and bandwidth counters over the monitored domains, plus how many
    /// reads the hardware refused.
    fn totals(&self) -> Option<(BTreeMap<String, u64>, u64)> {
        let domains = fs::read_dir(&self.mon_data).ok()?;
        let mut totals: BTreeMap<String, u64> = BTreeMap::new();
        let (mut refused, mut seen) = (0, false);
        for domain in domains.flatten().take(MAX_DOMAINS) {
            if !domain.file_name().to_string_lossy().starts_with("mon_L3_") {
                continue;
            }
            let files = std::iter::once(("llc_occupancy", "llc_occupancy_bytes")).chain(COUNTERS);
            for (file, key) in files {
                let Ok(text) = fs::read_to_string(domain.path().join(file)) else {
                    continue;
                };
                seen = true;
                match text.trim().parse::<u64>() {
                    Ok(value) => *totals.entry(key.to_owned()).or_default() += value,
                    // `Unavailable` / `Error`: the hardware has no value for this interval.
                    Err(_) => refused += 1,
                }
            }
        }
        seen.then_some((totals, refused))
    }
}

impl Sensor for ResctrlMonitorSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "resctrl_monitor",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "resctrl mounted with L3 monitoring (mon_data)",
        }
    }

    fn probe(&self) -> Availability {
        if self.totals().is_some() {
            Availability::Available
        } else {
            Availability::Unavailable(format!(
                "no L3 monitoring data under {}",
                self.mon_data.display()
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
            None => degraded = Some("resctrl monitoring became unreadable".to_owned()),
            Some((now, refused)) => {
                lost += refused;
                if refused > 0 {
                    degraded = Some(format!("{refused} monitoring read(s) were unavailable"));
                }
                if let Some(occupancy) = now.get("llc_occupancy_bytes") {
                    values.insert("llc_occupancy_bytes".to_owned(), *occupancy as f64);
                }
                if let Some(before) = &self.previous {
                    for (key, after) in now.iter().filter(|(key, _)| key.starts_with("mbm_")) {
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
            sensor: "resctrl_monitor".to_owned(),
            monotonic_ns,
            scope: SensorScope::System,
            quality: if values.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Direct
            },
            values,
            lost,
            provenance: self.mon_data.display().to_string(),
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
#[path = "resctrl_monitor_tests.rs"]
mod tests;
