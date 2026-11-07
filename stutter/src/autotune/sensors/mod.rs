//! The sensor framework (package H): a stable id, an actual availability probe, a cost class,
//! monotonic timestamps with provenance, source quality and loss, and no mutation authority.
//!
//! A sensor that fails suppresses only the candidates that depend on it. Expensive sensors are
//! admitted one at a time with a duration cap and a cooling period, and their own cost is recorded
//! so the daemon's incremental overhead stays visible.
mod psi;
mod registry;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

pub use psi::PsiSensor;
pub use registry::{AdmissionDenied, ExpensiveBudget, SensorRegistry};
use serde::{Deserialize, Serialize};

use crate::autotune::objective::ObjectiveSignalQuality;

/// How much a sensor costs to run, cheapest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SensorCost {
    /// Always-on, negligible.
    Cheap,
    /// Bounded and periodic.
    Moderate,
    /// On demand, short, one at a time.
    Escalation,
    /// Never at runtime; offline analysis only.
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SensorScope {
    System,
    Cgroup,
    Process,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SensorDescriptor {
    /// Stable identifier, never reused for a different measurement.
    pub id: &'static str,
    pub cost: SensorCost,
    pub scope: SensorScope,
    /// The permission or kernel feature the sensor needs, in words, for diagnostics.
    pub requires: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Availability {
    Available,
    /// The feature or permission is absent, with the actual reason.
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SensorHealth {
    Healthy,
    Degraded(String),
    Failed(String),
}

/// One reading. `monotonic_ns` is the caller's monotonic clock so readings order and age correctly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SensorReading {
    pub sensor: &'static str,
    pub monotonic_ns: u64,
    pub scope: SensorScope,
    pub quality: ObjectiveSignalQuality,
    pub values: BTreeMap<String, f64>,
    /// Events or samples the source reported losing since the previous reading.
    pub lost: u64,
    /// What the values were read from (a path or interface), for provenance.
    pub provenance: String,
}

pub trait Sensor {
    fn descriptor(&self) -> SensorDescriptor;
    /// Probes the real feature and permission now.
    fn probe(&self) -> Availability;
    fn start(&mut self) -> Result<(), String>;
    fn sample(&mut self, monotonic_ns: u64) -> SensorReading;
    fn health(&self) -> SensorHealth;
    fn stop(&mut self);
}
