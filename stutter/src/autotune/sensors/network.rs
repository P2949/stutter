//! System network counters: TCP retransmits and timeouts, listen-queue overflows and per-interface
//! drops, as deltas between readings.
//!
//! These describe the host's network health. They are never an application's request latency and
//! never a tuning input: a retransmit burst during an experiment is a reason to distrust it. The
//! loopback interface is excluded so local test traffic does not read as network load.
use std::{collections::BTreeMap, fs, path::PathBuf};

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::autotune::objective::ObjectiveSignalQuality;

/// Counters in `/proc/net/snmp` (`Tcp:`) and `/proc/net/netstat` (`TcpExt:`) that are watched.
const SNMP_TCP: [(&str, &str); 3] = [
    ("RetransSegs", "tcp_retrans_segs"),
    ("OutSegs", "tcp_out_segs"),
    ("InErrs", "tcp_in_errs"),
];
const NETSTAT_TCPEXT: [(&str, &str); 3] = [
    ("ListenOverflows", "tcp_listen_overflows"),
    ("ListenDrops", "tcp_listen_drops"),
    ("TCPTimeouts", "tcp_timeouts"),
];

/// A two-line `Proto: names...` / `Proto: values...` table, as in `/proc/net/snmp`.
fn proto_table(text: &str, proto: &str, wanted: &[(&str, &str)]) -> BTreeMap<String, u64> {
    let prefix = format!("{proto}:");
    let mut rows = text.lines().filter(|line| line.starts_with(&prefix));
    let (Some(names), Some(values)) = (rows.next(), rows.next()) else {
        return BTreeMap::new();
    };
    let names: Vec<&str> = names.split_whitespace().skip(1).collect();
    let values: Vec<&str> = values.split_whitespace().skip(1).collect();
    wanted
        .iter()
        .filter_map(|(name, key)| {
            let index = names.iter().position(|n| n == name)?;
            Some(((*key).to_owned(), values.get(index)?.parse().ok()?))
        })
        .collect()
}

/// Sums non-loopback interface counters from `/proc/net/dev`.
fn device_totals(text: &str) -> BTreeMap<String, u64> {
    let mut totals: BTreeMap<String, u64> = BTreeMap::new();
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        if name.trim() == "lo" {
            continue;
        }
        let fields: Vec<u64> = rest
            .split_whitespace()
            .filter_map(|field| field.parse().ok())
            .collect();
        if fields.len() < 16 {
            continue;
        }
        for (key, index) in [
            ("net_rx_bytes", 0),
            ("net_rx_errors", 2),
            ("net_rx_drops", 3),
            ("net_tx_bytes", 8),
            ("net_tx_errors", 10),
            ("net_tx_drops", 11),
        ] {
            *totals.entry(key.to_owned()).or_default() += fields[index];
        }
    }
    totals
}

pub struct NetworkSensor {
    proc_root: PathBuf,
    previous: Option<BTreeMap<String, u64>>,
    health: SensorHealth,
}

impl NetworkSensor {
    pub fn new(proc_root: PathBuf) -> Self {
        Self {
            proc_root,
            previous: None,
            health: SensorHealth::Healthy,
        }
    }

    fn counters(&self) -> Option<BTreeMap<String, u64>> {
        let snmp = fs::read_to_string(self.proc_root.join("net/snmp")).ok()?;
        let mut counters = proto_table(&snmp, "Tcp", &SNMP_TCP);
        if let Ok(netstat) = fs::read_to_string(self.proc_root.join("net/netstat")) {
            counters.extend(proto_table(&netstat, "TcpExt", &NETSTAT_TCPEXT));
        }
        if let Ok(dev) = fs::read_to_string(self.proc_root.join("net/dev")) {
            counters.extend(device_totals(&dev));
        }
        (!counters.is_empty()).then_some(counters)
    }
}

impl Sensor for NetworkSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "network",
            cost: SensorCost::Cheap,
            scope: SensorScope::System,
            requires: "/proc/net/snmp",
        }
    }

    fn probe(&self) -> Availability {
        if self.counters().is_some() {
            Availability::Available
        } else {
            Availability::Unavailable(format!(
                "{}/net/snmp has no TCP counters",
                self.proc_root.display()
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
            None => degraded = Some("network counters became unreadable".to_owned()),
            Some(now) => {
                if let Some(before) = &self.previous {
                    for (key, after) in &now {
                        let Some(was) = before.get(key) else { continue };
                        if after >= was {
                            values.insert(format!("{key}_delta"), (after - was) as f64);
                        } else {
                            // An interface went away or a counter reset: no delta is invented.
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
            sensor: "network".to_owned(),
            monotonic_ns,
            scope: SensorScope::System,
            quality: if values.is_empty() {
                ObjectiveSignalQuality::Missing
            } else {
                ObjectiveSignalQuality::Direct
            },
            values,
            lost,
            provenance: format!("{}/net/{{snmp,netstat,dev}}", self.proc_root.display()),
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
#[path = "network_tests.rs"]
mod tests;
