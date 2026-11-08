//! When an expensive sensor may be started: only for a logged diagnostic hypothesis.
//!
//! The cheap always-on sensors watch for sustained pressure. After `min_consecutive` consecutive
//! breaches the escalator names a hypothesis (CPU contention, memory, I/O or thermal) and, if a
//! diagnostic sensor exists for it, asks the registry to admit that sensor under its budget. A
//! cooldown stops one hypothesis from re-triggering at once. Nothing here writes to the system:
//! a hypothesis is a reason to *look closer*, never a reason to tune.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::SensorReading;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Hypothesis {
    CpuContention,
    MemoryPressure,
    IoPressure,
    ThermalThrottling,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    /// PSI `avg10` percent above which CPU, memory or I/O pressure counts as a breach.
    pub psi_avg10_percent: f64,
    /// Throttle events in one interval that count as a thermal breach.
    pub throttle_events: f64,
    pub min_consecutive: u32,
    /// Nanoseconds before the same hypothesis may trigger again.
    pub cooldown_ns: u64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            psi_avg10_percent: 20.0,
            throttle_events: 1.0,
            min_consecutive: 3,
            cooldown_ns: 120_000_000_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub hypothesis: Hypothesis,
    /// The expensive sensor that could test the hypothesis, if one exists.
    pub diagnostic_sensor: Option<&'static str>,
    pub reason: String,
}

/// The expensive sensor that bears on a hypothesis. Memory and I/O have none yet, so they are
/// logged as hypotheses with no diagnostic available rather than silently ignored.
pub fn diagnostic_sensor(hypothesis: Hypothesis) -> Option<&'static str> {
    match hypothesis {
        Hypothesis::CpuContention | Hypothesis::ThermalThrottling => Some("pmu_threads"),
        Hypothesis::MemoryPressure | Hypothesis::IoPressure => None,
    }
}

#[derive(Debug, Default)]
pub struct Escalator {
    thresholds: Thresholds,
    streak: BTreeMap<Hypothesis, u32>,
    last_trigger_ns: BTreeMap<Hypothesis, u64>,
}

fn value(readings: &[SensorReading], sensor: &str, key: &str) -> Option<f64> {
    readings
        .iter()
        .find(|reading| reading.sensor == sensor)
        .and_then(|reading| reading.values.get(key).copied())
}

impl Escalator {
    pub fn new(thresholds: Thresholds) -> Self {
        Self {
            thresholds,
            ..Self::default()
        }
    }

    fn breaches(&self, readings: &[SensorReading]) -> Vec<(Hypothesis, String)> {
        let limit = self.thresholds.psi_avg10_percent;
        let mut found = Vec::new();
        for (hypothesis, key, label) in [
            (Hypothesis::CpuContention, "cpu_some_avg10", "CPU"),
            (Hypothesis::MemoryPressure, "memory_some_avg10", "memory"),
            (Hypothesis::IoPressure, "io_some_avg10", "I/O"),
        ] {
            if let Some(pressure) = value(readings, "psi_system", key)
                && pressure >= limit
            {
                found.push((
                    hypothesis,
                    format!("{label} pressure avg10 {pressure:.1}% >= {limit:.1}%"),
                ));
            }
        }
        let oom = value(readings, "oom_guard", "high_delta").unwrap_or(0.0)
            + value(readings, "oom_guard", "oom_kill_delta").unwrap_or(0.0);
        if oom > 0.0 {
            found.push((
                Hypothesis::MemoryPressure,
                format!("{oom} memory.high or OOM events in the interval"),
            ));
        }
        if let Some(events) = value(readings, "cpu_power", "throttle_events_delta")
            && events >= self.thresholds.throttle_events
        {
            found.push((
                Hypothesis::ThermalThrottling,
                format!("{events} thermal throttle events in the interval"),
            ));
        }
        found
    }

    /// Feeds one set of readings taken at `now_ns`; returns a trigger once a breach has held for
    /// `min_consecutive` samples and its cooldown has passed.
    pub fn observe(&mut self, readings: &[SensorReading], now_ns: u64) -> Option<Trigger> {
        let breached = self.breaches(readings);
        // A hypothesis that did not breach this time loses its streak.
        self.streak
            .retain(|hypothesis, _| breached.iter().any(|(breach, _)| breach == hypothesis));
        let mut triggered = None;
        for (hypothesis, reason) in breached {
            let count = self.streak.entry(hypothesis).or_default();
            *count += 1;
            let cooled = self
                .last_trigger_ns
                .get(&hypothesis)
                .is_none_or(|last| now_ns.saturating_sub(*last) >= self.thresholds.cooldown_ns);
            if *count >= self.thresholds.min_consecutive && cooled && triggered.is_none() {
                self.last_trigger_ns.insert(hypothesis, now_ns);
                self.streak.remove(&hypothesis);
                triggered = Some(Trigger {
                    hypothesis,
                    diagnostic_sensor: diagnostic_sensor(hypothesis),
                    reason,
                });
            }
        }
        triggered
    }
}
