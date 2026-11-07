//! Pressure-stall information deltas for the system or one cgroup.
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

const RESOURCES: [&str; 3] = ["cpu", "memory", "io"];

/// `some`/`full` lines of a pressure file as `(avg10, total_us)` by kind.
fn parse_pressure(text: &str) -> BTreeMap<String, (f64, u64)> {
    let mut parsed = BTreeMap::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let Some(kind) = words.next().filter(|kind| matches!(*kind, "some" | "full")) else {
            continue;
        };
        let (mut avg10, mut total) = (None, None);
        for field in words {
            if let Some(value) = field.strip_prefix("avg10=") {
                avg10 = value.parse::<f64>().ok();
            } else if let Some(value) = field.strip_prefix("total=") {
                total = value.parse::<u64>().ok();
            }
        }
        if let (Some(avg10), Some(total)) = (avg10, total) {
            parsed.insert(kind.to_owned(), (avg10, total));
        }
    }
    parsed
}

pub struct PsiSensor {
    /// `<proc>/pressure` for the system, or a cgroup directory holding `*.pressure`.
    base: PathBuf,
    cgroup: bool,
    previous: BTreeMap<String, u64>,
    health: SensorHealth,
}

impl PsiSensor {
    pub fn system(proc_root: &Path) -> Self {
        Self::new(proc_root.join("pressure"), false)
    }

    pub fn cgroup(cgroup_dir: &Path) -> Self {
        Self::new(cgroup_dir.to_path_buf(), true)
    }

    fn new(base: PathBuf, cgroup: bool) -> Self {
        Self {
            base,
            cgroup,
            previous: BTreeMap::new(),
            health: SensorHealth::Healthy,
        }
    }

    fn file(&self, resource: &str) -> PathBuf {
        if self.cgroup {
            self.base.join(format!("{resource}.pressure"))
        } else {
            self.base.join(resource)
        }
    }
}

impl Sensor for PsiSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: if self.cgroup {
                "psi_cgroup"
            } else {
                "psi_system"
            },
            cost: SensorCost::Cheap,
            scope: if self.cgroup {
                SensorScope::Cgroup
            } else {
                SensorScope::System
            },
            requires: "CONFIG_PSI and a readable pressure file",
        }
    }

    fn probe(&self) -> Availability {
        match fs::read_to_string(self.file("cpu")) {
            Ok(text) if !parse_pressure(&text).is_empty() => Availability::Available,
            Ok(_) => Availability::Unavailable("pressure file has no readable line".into()),
            Err(error) => {
                Availability::Unavailable(format!("{}: {error}", self.file("cpu").display()))
            }
        }
    }

    fn start(&mut self) -> Result<(), String> {
        match self.probe() {
            Availability::Available => {
                self.previous.clear();
                self.health = SensorHealth::Healthy;
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
        for resource in RESOURCES {
            let Ok(text) = fs::read_to_string(self.file(resource)) else {
                degraded = Some(format!("{resource} pressure unreadable"));
                continue;
            };
            for (kind, (avg10, total)) in parse_pressure(&text) {
                let key = format!("{resource}_{kind}");
                values.insert(format!("{key}_avg10"), avg10);
                match self.previous.insert(key.clone(), total) {
                    Some(before) if total >= before => {
                        values.insert(format!("{key}_stall_us"), (total - before) as f64);
                    }
                    // A counter that went backwards was reset: no delta, and the loss is counted.
                    Some(_) => {
                        lost += 1;
                        degraded = Some(format!("{key} counter went backwards"));
                    }
                    None => {}
                }
            }
        }
        self.health = match degraded {
            Some(reason) => SensorHealth::Degraded(reason),
            None => SensorHealth::Healthy,
        };
        let has_delta = values.keys().any(|key| key.ends_with("_stall_us"));
        SensorReading {
            sensor: self.descriptor().id,
            monotonic_ns,
            scope: self.descriptor().scope,
            quality: if has_delta {
                ObjectiveSignalQuality::Direct
            } else {
                ObjectiveSignalQuality::Missing
            },
            values,
            lost,
            provenance: self.base.display().to_string(),
        }
    }

    fn health(&self) -> SensorHealth {
        self.health.clone()
    }

    fn stop(&mut self) {
        self.previous.clear();
    }
}
