//! Per-thread PMU counters as a budgeted sensor.
//!
//! This wraps the existing grouped `perf_event_open` sampler; it does not replace it. The raw
//! counters are mechanism evidence, never an outcome: a high cache-miss rate is not proof of a
//! memory-bound workload, and a multiplexed or barely-running counter is reported as approximate or
//! missing instead of being scaled into a conclusion.
use std::collections::BTreeMap;

use stutter_core::ids::Tid;

use super::{
    Availability, Sensor, SensorCost, SensorDescriptor, SensorHealth, SensorReading, SensorScope,
};
use crate::{
    autotune::objective::ObjectiveSignalQuality,
    metrics::TaskStatsMap,
    perf_counters::{
        CpuPerfConfig, CpuPerfDelta, CpuPerfSampler, try_open_disabled_cycles_current_thread,
    },
    process_tree::TaskMap,
};

/// Below this share of enabled time actually counting, a scaled value is only approximate.
pub const MIN_RUNNING_RATIO: f64 = 0.5;

/// Turns one interval's per-thread deltas into a reading. Pure, so every rule is testable.
pub fn reading_from_deltas(
    deltas: &BTreeMap<Tid, CpuPerfDelta>,
    monotonic_ns: u64,
) -> SensorReading {
    let usable: Vec<&CpuPerfDelta> = deltas
        .values()
        .filter(|delta| {
            delta.unavailable_reason.is_none()
                && matches!((delta.time_enabled_ns, delta.time_running_ns),
                    (Some(enabled), Some(running)) if running > 0 && running <= enabled)
        })
        .collect();
    let mut lost = (deltas.len() - usable.len()) as u64;
    let mut overflow = false;
    let mut sum = |field: fn(&CpuPerfDelta) -> Option<u64>| -> Option<u64> {
        usable.iter().try_fold(0u64, |total, delta| {
            let next = total.checked_add(field(delta)?);
            overflow |= next.is_none();
            next
        })
    };
    let enabled: u128 = usable
        .iter()
        .filter_map(|d| d.time_enabled_ns)
        .map(u128::from)
        .sum();
    let running: u128 = usable
        .iter()
        .filter_map(|d| d.time_running_ns)
        .map(u128::from)
        .sum();
    let multiplexed = usable
        .iter()
        .filter(|delta| {
            delta.multiplexed || delta.scaled || delta.time_enabled_ns != delta.time_running_ns
        })
        .count();

    let mut values = BTreeMap::new();
    values.insert("threads".to_owned(), usable.len() as f64);
    values.insert("threads_multiplexed".to_owned(), multiplexed as f64);
    let ratio = (enabled > 0).then(|| running as f64 / enabled as f64);
    if let Some(ratio) = ratio {
        values.insert("running_ratio".to_owned(), ratio);
    }
    let cycles = sum(|d| d.cycles);
    let instructions = sum(|d| d.instructions);
    let references = sum(|d| d.cache_references);
    let misses = sum(|d| d.cache_misses);
    let branches = sum(|d| d.branches);
    let branch_misses = sum(|d| d.branch_misses);
    // A ratio needs a positive denominator; a zero or absent one is no conclusion at all.
    if let (Some(instructions), Some(cycles)) = (instructions, cycles)
        && cycles > 0
    {
        values.insert("ipc".to_owned(), instructions as f64 / cycles as f64);
    }
    if let (Some(misses), Some(instructions)) = (misses, instructions)
        && instructions > 0
    {
        values.insert(
            "cache_mpki".to_owned(),
            misses as f64 * 1000.0 / instructions as f64,
        );
    }
    if let (Some(misses), Some(references)) = (misses, references)
        && references > 0
    {
        values.insert(
            "cache_miss_rate".to_owned(),
            misses as f64 / references as f64,
        );
    }
    if let (Some(misses), Some(branches)) = (branch_misses, branches)
        && branches > 0
        && misses <= branches
    {
        values.insert("branch_miss_rate".into(), misses as f64 / branches as f64);
    }
    if overflow {
        values.retain(|key, _| {
            matches!(
                key.as_str(),
                "threads" | "threads_multiplexed" | "running_ratio"
            )
        });
        lost += 1;
    }
    let quality = if overflow
        || usable.is_empty()
        || !values.contains_key("ipc") && !values.contains_key("cache_mpki")
    {
        ObjectiveSignalQuality::Missing
    } else if lost > 0 || multiplexed > 0 || ratio.is_some_and(|ratio| ratio < MIN_RUNNING_RATIO) {
        ObjectiveSignalQuality::Approximate
    } else {
        ObjectiveSignalQuality::Direct
    };
    SensorReading {
        sensor: "pmu_threads".to_owned(),
        monotonic_ns,
        scope: SensorScope::Process,
        quality,
        values,
        lost,
        provenance: "perf_event_open grouped counters per thread".to_owned(),
    }
}

pub struct PmuSensor {
    sampler: Option<CpuPerfSampler>,
    config: CpuPerfConfig,
    targets: PmuTargets,
    health: SensorHealth,
}

/// The threads to count, shared so the runtime can update them while the registry owns the sensor.
pub type PmuTargets = std::sync::Arc<std::sync::Mutex<Option<(TaskMap, TaskStatsMap)>>>;

impl PmuSensor {
    pub fn new(config: CpuPerfConfig) -> Self {
        Self {
            sampler: None,
            config,
            targets: PmuTargets::default(),
            health: SensorHealth::Healthy,
        }
    }

    /// The threads to count in the next sample, chosen by the caller from a diagnostic hypothesis.
    pub fn set_targets(&mut self, active: TaskMap, stats: TaskStatsMap) {
        if let Ok(mut targets) = self.targets.lock() {
            *targets = Some((active, stats));
        }
    }

    /// A handle the runtime keeps to choose the threads while the registry owns this sensor.
    pub fn targets_handle(&self) -> PmuTargets {
        std::sync::Arc::clone(&self.targets)
    }
}

impl Sensor for PmuSensor {
    fn descriptor(&self) -> SensorDescriptor {
        SensorDescriptor {
            id: "pmu_threads",
            cost: SensorCost::Moderate,
            scope: SensorScope::Process,
            requires: "perf_event_open (perf_event_paranoid and CAP_PERFMON) and a PMU",
        }
    }

    fn probe(&self) -> Availability {
        match try_open_disabled_cycles_current_thread(self.config.include_kernel) {
            Ok(()) => Availability::Available,
            Err(error) => Availability::Unavailable(format!("perf_event_open failed: {error}")),
        }
    }

    fn start(&mut self) -> Result<(), String> {
        if let Availability::Unavailable(reason) = self.probe() {
            self.health = SensorHealth::Failed(reason.clone());
            return Err(reason);
        }
        self.sampler = Some(CpuPerfSampler::new(self.config.clone()));
        self.health = SensorHealth::Healthy;
        Ok(())
    }

    fn sample(&mut self, monotonic_ns: u64) -> SensorReading {
        let deltas = match (self.sampler.as_mut(), self.targets.lock()) {
            (Some(sampler), Ok(targets)) => match targets.as_ref() {
                Some((active, stats)) => {
                    sampler.sync_targets(active, stats);
                    sampler.sample_interval()
                }
                None => BTreeMap::new(),
            },
            _ => BTreeMap::new(),
        };
        let reading = reading_from_deltas(&deltas, monotonic_ns);
        self.health = if reading.lost > 0 {
            SensorHealth::Degraded(format!(
                "{} thread(s) without a usable counter",
                reading.lost
            ))
        } else {
            SensorHealth::Healthy
        };
        reading
    }

    fn health(&self) -> SensorHealth {
        self.health.clone()
    }

    fn stop(&mut self) {
        self.sampler = None;
        if let Ok(mut targets) = self.targets.lock() {
            *targets = None;
        }
    }
}
