//! Whole-disk block I/O counters from `/proc/diskstats`: completed I/Os, sectors, busy time and
//! queue time as deltas, plus the instantaneous in-flight count.
//!
//! Only real block devices count (a name that exists under `/sys/block`, so partitions, loop and
//! ram devices are not double counted). These describe host storage load; they are never a tuning
//! input and say nothing about one application's I/O latency.
use std::{collections::BTreeMap, fs, path::PathBuf};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

/// Field offsets after `major minor name` in `/proc/diskstats`.
const FIELDS: [(&str, usize); 7] = [
    ("io_read_ios", 0),
    ("io_read_sectors", 2),
    ("io_write_ios", 4),
    ("io_write_sectors", 6),
    ("io_inflight", 8),
    ("io_busy_ms", 9),
    ("io_queue_ms", 10),
];

pub struct BlockIoSensor {
    diskstats: PathBuf,
    sys_block: PathBuf,
    previous: Option<BTreeMap<String, u64>>,
    health: SensorHealth,
}

impl BlockIoSensor {
    pub fn new(proc_root: PathBuf, sys_root: PathBuf) -> Self {
        Self {
            diskstats: proc_root.join("diskstats"),
            sys_block: sys_root.join("block"),
            previous: None,
            health: SensorHealth::Healthy,
        }
    }

    fn totals(&self) -> Option<BTreeMap<String, u64>> {
        let text = fs::read_to_string(&self.diskstats).ok()?;
        let mut totals: BTreeMap<String, u64> = BTreeMap::new();
        let mut disks = 0;
        for line in text.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let Some(name) = fields.get(2) else { continue };
            if name.starts_with("loop") || name.starts_with("ram") || name.contains('/') {
                continue;
            }
            if !self.sys_block.join(name).exists() {
                continue;
            }
            let numbers: Vec<u64> = fields[3..].iter().filter_map(|f| f.parse().ok()).collect();
            if numbers.len() < 11 {
                continue;
            }
            disks += 1;
            for (key, index) in FIELDS {
                *totals.entry(key.to_owned()).or_default() += numbers[index];
            }
        }
        (disks > 0).then_some(totals)
    }
}

impl Sensor for BlockIoSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "block_io",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "/proc/diskstats and /sys/block",
        }
    }

    fn probe(&self) -> Availability {
        if self.totals().is_some() {
            Availability::Available
        } else {
            Availability::Unavailable(format!(
                "no whole-disk entry in {}",
                self.diskstats.display()
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
            None => degraded = Some("diskstats became unreadable".to_owned()),
            Some(now) => {
                if let Some(before) = &self.previous {
                    for (key, after) in &now {
                        if key == "io_inflight" {
                            // An instantaneous gauge, not a counter.
                            values.insert(key.clone(), *after as f64);
                            continue;
                        }
                        let Some(was) = before.get(key) else { continue };
                        if after >= was {
                            values.insert(format!("{key}_delta"), (after - was) as f64);
                        } else {
                            // A disk was removed or a counter reset: no delta is invented.
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
            sensor: "block_io".to_owned(),
            monotonic_ns,
            scope: SensorScope::System,
            quality: if values.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Direct
            },
            values,
            lost,
            provenance: self.diskstats.display().to_string(),
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
#[path = "block_io_tests.rs"]
mod tests;
