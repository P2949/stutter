use std::collections::{BTreeMap, VecDeque};

use super::model::{Aggregation, KpiContract, MetricSpec, Sample};
use crate::telemetry::registration::PeerCredentials;

/// Samples kept per metric; older ones are dropped, never grown without bound.
const MAX_SAMPLES_PER_METRIC: usize = 4096;
/// Longest accepted sample line, in bytes.
pub const MAX_SAMPLE_LINE_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngestDenial {
    LineTooLong,
    BadJson,
    WrongUser,
    NotTheBoundProcess,
    UnknownMetric,
    NonFinite,
    OutOfOrder,
    TooOld,
    EmptyPopulation,
}

/// What a window of samples says, with its size so the caller can judge it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowAggregate {
    pub value: f64,
    pub samples: usize,
    pub population: u64,
}

pub struct SampleStore {
    contract: KpiContract,
    samples: BTreeMap<String, VecDeque<Sample>>,
}

impl SampleStore {
    pub fn new(contract: KpiContract) -> Self {
        Self {
            contract,
            samples: BTreeMap::new(),
        }
    }

    /// Accepts one sample line from `peer` at the local monotonic time `now_ns`. The peer must be
    /// the bound process of the bound user; a stale, out-of-order or unknown sample is refused
    /// with its reason, never coerced.
    pub fn ingest_line(
        &mut self,
        peer: PeerCredentials,
        line: &str,
        now_ns: u64,
    ) -> Result<(), IngestDenial> {
        if line.len() > MAX_SAMPLE_LINE_BYTES {
            return Err(IngestDenial::LineTooLong);
        }
        let binding = self.contract.binding;
        if peer.uid != binding.uid {
            return Err(IngestDenial::WrongUser);
        }
        if peer.pid != binding.pid {
            return Err(IngestDenial::NotTheBoundProcess);
        }
        let sample: Sample = serde_json::from_str(line).map_err(|_| IngestDenial::BadJson)?;
        let spec = self
            .contract
            .spec(&sample.metric)
            .ok_or(IngestDenial::UnknownMetric)?;
        if !sample.value.is_finite() {
            return Err(IngestDenial::NonFinite);
        }
        if sample.population == 0 {
            return Err(IngestDenial::EmptyPopulation);
        }
        if now_ns.saturating_sub(sample.monotonic_ns) > spec.max_age_ms.saturating_mul(1_000_000) {
            return Err(IngestDenial::TooOld);
        }
        let queue = self.samples.entry(sample.metric.clone()).or_default();
        if queue
            .back()
            .is_some_and(|last| sample.monotonic_ns < last.monotonic_ns)
        {
            return Err(IngestDenial::OutOfOrder);
        }
        if queue.len() == MAX_SAMPLES_PER_METRIC {
            queue.pop_front();
        }
        queue.push_back(sample);
        Ok(())
    }

    /// The metric's aggregate over `[from_ns, to_ns]`, or `None` when the window is empty,
    /// stale at `now_ns`, or below the metric's minimum population.
    pub fn window(
        &self,
        metric: &str,
        from_ns: u64,
        to_ns: u64,
        now_ns: u64,
    ) -> Option<WindowAggregate> {
        let spec = self.contract.spec(metric)?;
        let queue = self.samples.get(metric)?;
        if now_ns.saturating_sub(queue.back()?.monotonic_ns)
            > spec.max_age_ms.saturating_mul(1_000_000)
        {
            return None;
        }
        let inside: Vec<&Sample> = queue
            .iter()
            .filter(|sample| (from_ns..=to_ns).contains(&sample.monotonic_ns))
            .collect();
        let population: u64 = inside.iter().map(|sample| sample.population).sum();
        if inside.is_empty() || population < spec.min_population {
            return None;
        }
        Some(WindowAggregate {
            value: aggregate(spec, &inside),
            samples: inside.len(),
            population,
        })
    }
}

fn aggregate(spec: &MetricSpec, samples: &[&Sample]) -> f64 {
    let mut values: Vec<f64> = samples.iter().map(|sample| sample.value).collect();
    let rank = |values: &mut Vec<f64>, q: f64| {
        values.sort_by(f64::total_cmp);
        let index = ((q * values.len() as f64).ceil() as usize).clamp(1, values.len());
        values[index - 1]
    };
    match spec.aggregation {
        Aggregation::Mean => values.iter().sum::<f64>() / values.len() as f64,
        Aggregation::P50 => rank(&mut values, 0.50),
        Aggregation::P95 => rank(&mut values, 0.95),
        Aggregation::P99 => rank(&mut values, 0.99),
        Aggregation::Max => values.iter().copied().fold(f64::MIN, f64::max),
        Aggregation::Sum => values.iter().sum(),
        Aggregation::Last => values.last().copied().unwrap_or(0.0),
    }
}
