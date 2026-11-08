//! Read-only memory-domain sensors: NUMA locality, DAMON state and resctrl inventory.
//!
//! These observe; they never configure DAMOS, move pages or write a schemata. DAMON's admin files
//! are root-only, so an unprivileged read reports `permission denied` as the reason it is
//! unavailable instead of a healthy empty reading. No address or region detail is read at all.
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

fn read_text(path: &Path) -> Result<String, String> {
    fs::read_to_string(path)
        .map(|text| text.trim().to_owned())
        .map_err(|error| match error.kind() {
            io::ErrorKind::PermissionDenied => format!("{}: permission denied", path.display()),
            io::ErrorKind::NotFound => format!("{} does not exist", path.display()),
            _ => format!("{}: {error}", path.display()),
        })
}

fn reading(
    id: &str,
    scope: SensorScope,
    monotonic_ns: u64,
    values: BTreeMap<String, f64>,
    lost: u64,
    provenance: String,
) -> SensorReading {
    SensorReading {
        sensor: id.to_owned(),
        monotonic_ns,
        scope,
        quality: if values.is_empty() {
            ObjectiveSignalQuality::Missing
        } else {
            ObjectiveSignalQuality::Direct
        },
        values,
        lost,
        provenance,
    }
}

/// NUMA hit, miss and locality counters summed over nodes, as deltas.
pub struct NumaSensor {
    node_root: PathBuf,
    previous: Option<BTreeMap<String, u64>>,
    health: SensorHealth,
}

impl NumaSensor {
    pub fn new(system_root: &Path) -> Self {
        Self {
            node_root: system_root.join("node"),
            previous: None,
            health: SensorHealth::Healthy,
        }
    }

    fn totals(&self) -> Option<BTreeMap<String, u64>> {
        let entries = fs::read_dir(&self.node_root).ok()?;
        let mut totals: BTreeMap<String, u64> = BTreeMap::new();
        let mut any = false;
        for entry in entries.flatten() {
            let Ok(text) = fs::read_to_string(entry.path().join("numastat")) else {
                continue;
            };
            any = true;
            for line in text.lines() {
                if let Some((key, value)) = line.split_once(' ')
                    && let Ok(value) = value.trim().parse::<u64>()
                {
                    *totals.entry(key.to_owned()).or_default() += value;
                }
            }
        }
        any.then_some(totals)
    }
}

impl Sensor for NumaSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "numa_stat",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "NUMA sysfs numastat (any node)",
        }
    }

    fn probe(&self) -> Availability {
        if self.totals().is_some() {
            Availability::Available
        } else {
            Availability::Unavailable(format!(
                "no node numastat under {}",
                self.node_root.display()
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
        if let Some(now) = self.totals() {
            if let Some(before) = &self.previous {
                for key in [
                    "numa_hit",
                    "numa_miss",
                    "numa_foreign",
                    "local_node",
                    "other_node",
                ] {
                    let (Some(&after), Some(&was)) = (now.get(key), before.get(key)) else {
                        continue;
                    };
                    if after >= was {
                        values.insert(format!("{key}_delta"), (after - was) as f64);
                    } else {
                        lost += 1;
                        degraded = Some(format!("numastat {key} went backwards"));
                    }
                }
                let local = values.get("local_node_delta").copied().unwrap_or(0.0);
                let other = values.get("other_node_delta").copied().unwrap_or(0.0);
                // A ratio needs events: no allocations in the interval is no ratio, not zero.
                if local + other > 0.0 {
                    values.insert("remote_ratio".to_owned(), other / (local + other));
                }
            }
            self.previous = Some(now);
        } else {
            degraded = Some("numastat became unreadable".to_owned());
        }
        self.health = degraded.map_or(SensorHealth::Healthy, SensorHealth::Degraded);
        reading(
            "numa_stat",
            SensorScope::System,
            monotonic_ns,
            values,
            lost,
            self.node_root.display().to_string(),
        )
    }

    fn health(&self) -> SensorHealth {
        self.health.clone()
    }

    fn stop(&mut self) {
        self.previous = None;
    }
}

/// How many kdamonds exist and run, and how many schemes are installed by someone. Only counts.
pub struct DamonSensor {
    root: PathBuf,
}

impl DamonSensor {
    pub fn new(kernel_mm_root: &Path) -> Self {
        Self {
            root: kernel_mm_root.join("damon/admin/kdamonds"),
        }
    }

    fn counts(&self) -> Result<(u64, u64, u64), String> {
        let total: u64 = read_text(&self.root.join("nr_kdamonds"))?
            .parse()
            .map_err(|_| "nr_kdamonds is not a number".to_owned())?;
        let (mut running, mut schemes) = (0, 0);
        for index in 0..total.min(64) {
            let dir = self.root.join(index.to_string());
            if read_text(&dir.join("state")).is_ok_and(|state| state == "on") {
                running += 1;
            }
            let contexts: u64 = read_text(&dir.join("contexts/nr_contexts"))
                .ok()
                .and_then(|text| text.parse().ok())
                .unwrap_or(0);
            for context in 0..contexts.min(16) {
                schemes += read_text(&dir.join(format!("contexts/{context}/schemes/nr_schemes")))
                    .ok()
                    .and_then(|text| text.parse::<u64>().ok())
                    .unwrap_or(0);
            }
        }
        Ok((total, running, schemes))
    }
}

impl Sensor for DamonSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "damon_state",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "DAMON sysfs admin interface (root-readable on most kernels)",
        }
    }

    fn probe(&self) -> Availability {
        match self.counts() {
            Ok(_) => Availability::Available,
            Err(reason) => Availability::Unavailable(reason),
        }
    }

    fn start(&mut self) -> Result<(), String> {
        match self.probe() {
            Availability::Available => Ok(()),
            Availability::Unavailable(reason) => Err(reason),
        }
    }

    fn sample(&mut self, monotonic_ns: u64) -> SensorReading {
        let mut values = BTreeMap::new();
        if let Ok((total, running, schemes)) = self.counts() {
            values.insert("kdamonds".to_owned(), total as f64);
            values.insert("kdamonds_running".to_owned(), running as f64);
            values.insert("schemes_installed".to_owned(), schemes as f64);
        }
        reading(
            "damon_state",
            SensorScope::System,
            monotonic_ns,
            values,
            0,
            self.root.display().to_string(),
        )
    }

    fn health(&self) -> SensorHealth {
        match self.counts() {
            Ok(_) => SensorHealth::Healthy,
            Err(reason) => SensorHealth::Failed(reason),
        }
    }

    fn stop(&mut self) {}
}

/// resctrl inventory: closids, rmids and groups. Reports unavailable when it is not mounted.
pub struct ResctrlSensor {
    root: PathBuf,
}

impl ResctrlSensor {
    pub fn new(fs_root: &Path) -> Self {
        Self {
            root: fs_root.join("resctrl"),
        }
    }

    fn inventory(&self) -> Result<BTreeMap<String, f64>, String> {
        if !self.root.join("info").is_dir() {
            return Err(format!("{} is not mounted", self.root.display()));
        }
        let mut values = BTreeMap::new();
        for (key, path) in [
            ("l3_closids", "info/L3/num_closids"),
            ("l3_rmids", "info/L3_MON/num_rmids"),
            ("mba_closids", "info/MB/num_closids"),
        ] {
            if let Ok(text) = read_text(&self.root.join(path))
                && let Ok(value) = text.parse::<f64>()
            {
                values.insert(key.to_owned(), value);
            }
        }
        let groups = fs::read_dir(&self.root)
            .map_err(|error| error.to_string())?
            .flatten()
            .filter(|entry| {
                entry.path().is_dir()
                    && !matches!(
                        entry.file_name().to_str(),
                        Some("info" | "mon_groups" | "mon_data")
                    )
            })
            .count();
        values.insert("allocation_groups".to_owned(), groups as f64);
        Ok(values)
    }
}

impl Sensor for ResctrlSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "resctrl_inventory",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "resctrl mounted (Intel RDT or AMD QoS hardware)",
        }
    }

    fn probe(&self) -> Availability {
        match self.inventory() {
            Ok(_) => Availability::Available,
            Err(reason) => Availability::Unavailable(reason),
        }
    }

    fn start(&mut self) -> Result<(), String> {
        match self.probe() {
            Availability::Available => Ok(()),
            Availability::Unavailable(reason) => Err(reason),
        }
    }

    fn sample(&mut self, monotonic_ns: u64) -> SensorReading {
        let values = self.inventory().unwrap_or_default();
        reading(
            "resctrl_inventory",
            SensorScope::System,
            monotonic_ns,
            values,
            0,
            self.root.display().to_string(),
        )
    }

    fn health(&self) -> SensorHealth {
        match self.inventory() {
            Ok(_) => SensorHealth::Healthy,
            Err(reason) => SensorHealth::Failed(reason),
        }
    }

    fn stop(&mut self) {}
}
