//! Admission of sensors: cheap ones always, expensive ones one at a time with a cap.
use std::collections::BTreeMap;

use super::{Availability, Sensor, SensorCost, SensorReading};

/// Limits on expensive sensors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpensiveBudget {
    /// How many Moderate/Escalation sensors may run at once.
    pub max_concurrent: usize,
    /// How long one may run before it must be stopped.
    pub max_duration_ns: u64,
    /// How long after one stops before another may start.
    pub cooling_ns: u64,
}

impl Default for ExpensiveBudget {
    fn default() -> Self {
        Self {
            max_concurrent: 1,
            max_duration_ns: 30_000_000_000,
            cooling_ns: 60_000_000_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionDenied {
    Unknown,
    Offline,
    Unavailable(String),
    ConcurrencyLimit,
    Cooling { remaining_ns: u64 },
    StartFailed(String),
}

struct Entry {
    sensor: Box<dyn Sensor>,
    running_since: Option<u64>,
    /// Time spent inside `sample`, as the sensor's own incremental cost.
    cost_ns: u64,
}

#[derive(Default)]
pub struct SensorRegistry {
    budget: ExpensiveBudget,
    entries: BTreeMap<&'static str, Entry>,
    last_expensive_stop_ns: Option<u64>,
}

impl SensorRegistry {
    pub fn new(budget: ExpensiveBudget) -> Self {
        Self {
            budget,
            ..Self::default()
        }
    }

    pub fn register(&mut self, sensor: Box<dyn Sensor>) {
        self.entries.insert(
            sensor.descriptor().id,
            Entry {
                sensor,
                running_since: None,
                cost_ns: 0,
            },
        );
    }

    fn expensive(cost: SensorCost) -> bool {
        matches!(cost, SensorCost::Moderate | SensorCost::Escalation)
    }

    fn running_expensive(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| {
                entry.running_since.is_some() && Self::expensive(entry.sensor.descriptor().cost)
            })
            .count()
    }

    /// Starts sensor `id` at `now_ns` if its probe passes and the budget allows. A failure here
    /// affects only the candidates that depend on this sensor.
    pub fn activate(&mut self, id: &str, now_ns: u64) -> Result<(), AdmissionDenied> {
        let cost = self
            .entries
            .get(id)
            .ok_or(AdmissionDenied::Unknown)?
            .sensor
            .descriptor()
            .cost;
        if cost == SensorCost::Offline {
            return Err(AdmissionDenied::Offline);
        }
        if Self::expensive(cost) {
            if self.running_expensive() >= self.budget.max_concurrent {
                return Err(AdmissionDenied::ConcurrencyLimit);
            }
            if let Some(stopped) = self.last_expensive_stop_ns {
                let ready = stopped.saturating_add(self.budget.cooling_ns);
                if now_ns < ready {
                    return Err(AdmissionDenied::Cooling {
                        remaining_ns: ready - now_ns,
                    });
                }
            }
        }
        let entry = self.entries.get_mut(id).ok_or(AdmissionDenied::Unknown)?;
        if let Availability::Unavailable(reason) = entry.sensor.probe() {
            return Err(AdmissionDenied::Unavailable(reason));
        }
        entry.sensor.start().map_err(AdmissionDenied::StartFailed)?;
        entry.running_since = Some(now_ns);
        Ok(())
    }

    /// Samples every running sensor, stopping any expensive one past its duration cap.
    pub fn sample_all(&mut self, now_ns: u64) -> Vec<SensorReading> {
        let max = self.budget.max_duration_ns;
        let mut readings = Vec::new();
        let mut stopped_expensive = false;
        for entry in self.entries.values_mut() {
            let Some(since) = entry.running_since else {
                continue;
            };
            let expensive = Self::expensive(entry.sensor.descriptor().cost);
            if expensive && now_ns.saturating_sub(since) > max {
                entry.sensor.stop();
                entry.running_since = None;
                stopped_expensive = true;
                continue;
            }
            let begun = std::time::Instant::now();
            readings.push(entry.sensor.sample(now_ns));
            entry.cost_ns = entry
                .cost_ns
                .saturating_add(begun.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
        }
        if stopped_expensive {
            self.last_expensive_stop_ns = Some(now_ns);
        }
        readings
    }

    pub fn deactivate(&mut self, id: &str, now_ns: u64) {
        if let Some(entry) = self.entries.get_mut(id)
            && entry.running_since.take().is_some()
        {
            entry.sensor.stop();
            if Self::expensive(entry.sensor.descriptor().cost) {
                self.last_expensive_stop_ns = Some(now_ns);
            }
        }
    }

    /// Cumulative time each sensor has spent sampling, for overhead accounting.
    pub fn overhead_ns(&self) -> BTreeMap<&'static str, u64> {
        self.entries
            .iter()
            .map(|(id, entry)| (*id, entry.cost_ns))
            .collect()
    }
}
