//! One target process's resident pages per NUMA node, from `/proc/<pid>/numa_maps`.
//!
//! Reading `numa_maps` walks the process's page tables, so this sensor reads at most once per
//! `MIN_INTERVAL_NS` and returns an empty (Missing) reading in between rather than a stale one. It
//! is bound to the target by pid *and* live start time: if the pid is reused by another process the
//! reading is dropped and says why. Observation only: moving a thread is not moving memory, and
//! nothing here migrates, binds or changes a policy.
use std::{collections::BTreeMap, fs, path::PathBuf};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
    parse_numa_maps,
};
use crate::{autotune::objective::ObjectiveSignalQuality, telemetry::proc_view::live_process};

/// Least time between two page-table walks of the same process.
pub const MIN_INTERVAL_NS: u64 = 30_000_000_000;

pub struct NumaMapsSensor {
    proc_root: PathBuf,
    pid: u32,
    start_ticks: u64,
    last_read_ns: Option<u64>,
    health: SensorHealth,
}

impl NumaMapsSensor {
    pub fn new(proc_root: PathBuf, pid: u32, start_ticks: u64) -> Self {
        Self {
            proc_root,
            pid,
            start_ticks,
            last_read_ns: None,
            health: SensorHealth::Healthy,
        }
    }

    fn identity_holds(&self) -> bool {
        live_process(&self.proc_root, self.pid)
            .is_some_and(|live| live.start_ticks == self.start_ticks)
    }

    fn path(&self) -> PathBuf {
        self.proc_root.join(self.pid.to_string()).join("numa_maps")
    }
}

impl Sensor for NumaMapsSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "numa_maps",
            cost: SensorCost::Cheap,
            scope: SensorScope::Process,
            requires: "/proc/<pid>/numa_maps (CONFIG_NUMA, same user as the target)",
        }
    }

    fn probe(&self) -> Availability {
        if !self.identity_holds() {
            return Availability::Unavailable(
                "the target process is gone or was replaced".to_owned(),
            );
        }
        match fs::File::open(self.path()) {
            Ok(_) => Availability::Available,
            Err(error) => Availability::Unavailable(format!("{}: {error}", self.path().display())),
        }
    }

    fn start(&mut self) -> Result<(), String> {
        match self.probe() {
            Availability::Available => {
                self.last_read_ns = None;
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
        let mut degraded = None;
        let due = self
            .last_read_ns
            .is_none_or(|last| monotonic_ns.saturating_sub(last) >= MIN_INTERVAL_NS);
        if due {
            self.last_read_ns = Some(monotonic_ns);
            if !self.identity_holds() {
                degraded = Some("the target process was replaced or exited".to_owned());
            } else {
                match fs::read_to_string(self.path()) {
                    Ok(text) => {
                        let summary = parse_numa_maps(&text);
                        let total: u64 = summary.pages_by_node.values().sum();
                        for (node, pages) in &summary.pages_by_node {
                            values.insert(format!("pages_on_node_{node}"), *pages as f64);
                        }
                        values.insert("pages_total".to_owned(), total as f64);
                        values.insert("anon_pages".to_owned(), summary.anon_pages as f64);
                    }
                    Err(error) => degraded = Some(format!("numa_maps unreadable: {error}")),
                }
            }
        }
        self.health = degraded.map_or(SensorHealth::Healthy, SensorHealth::Degraded);
        SensorReading {
            sensor: "numa_maps".to_owned(),
            monotonic_ns,
            scope: SensorScope::Process,
            quality: if values.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Direct
            },
            values,
            lost: 0,
            provenance: format!(
                "{} (pid {} start {})",
                self.path().display(),
                self.pid,
                self.start_ticks
            ),
        }
    }

    fn health(&self) -> SensorHealth {
        self.health.clone()
    }

    fn stop(&mut self) {
        self.last_read_ns = None;
    }
}

#[cfg(test)]
#[path = "numa_maps_tests.rs"]
mod tests;
