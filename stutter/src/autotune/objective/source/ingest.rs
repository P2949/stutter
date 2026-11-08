use std::collections::{BTreeMap, VecDeque};

use super::model::{Aggregation, KpiContract, MetricSpec, Sample, SampleKind};
use crate::telemetry::registration::PeerCredentials;

/// Samples kept per metric; older ones are dropped, never grown without bound, and a window that
/// reaches into the dropped range is reported as incomplete.
const MAX_SAMPLES_PER_METRIC: usize = 4096;
/// Longest accepted sample line, in bytes.
pub const MAX_SAMPLE_LINE_BYTES: usize = 512;
/// How far ahead of the local clock a reporter's timestamp may be before it is a forgery.
pub const MAX_FUTURE_SKEW_NS: u64 = 1_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngestDenial {
    LineTooLong,
    BadJson,
    WrongUser,
    NotTheBoundProcess,
    /// The peer's live start time differs from the bound one: a reused pid.
    PeerStartTimeMismatch,
    UnknownMetric,
    NonFinite,
    NegativeValue,
    /// A timestamp equal to or before the previous one for this metric (replay or duplicate).
    NotAfterPrevious,
    FutureTimestamp,
    TooOld,
    EmptyPopulation,
    /// A raw event must represent exactly one event.
    RawEventPopulation,
}

/// What an aggregate truthfully is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Semantics {
    /// Computed over raw events (or a population-weighted mean of summaries): a direct figure.
    Direct,
    /// A quantile of per-batch summaries: not a percentile of the underlying events, so it can
    /// never back a direct request or frame percentile claim.
    QuantileOfSummaries,
}

/// What a window of samples says, with its size so the caller can judge it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowAggregate {
    pub value: f64,
    pub samples: usize,
    pub population: u64,
    pub semantics: Semantics,
    /// False when the window starts inside the range the bounded store already dropped.
    pub complete: bool,
}

#[derive(Default)]
struct Series {
    samples: VecDeque<Sample>,
    /// Newest timestamp that was evicted by capacity, if any.
    evicted_through_ns: Option<u64>,
}

pub struct SampleStore {
    contract: KpiContract,
    series: BTreeMap<String, Series>,
}

impl SampleStore {
    pub fn new(contract: KpiContract) -> Self {
        Self {
            contract,
            series: BTreeMap::new(),
        }
    }

    /// Accepts one sample line from `peer` at the local monotonic time `now_ns`. `peer_start_ticks`
    /// is the peer's start time read live; it must equal the bound start time, so a reused pid
    /// never matches. A stale, future, duplicate or unknown sample is refused with its reason,
    /// never coerced.
    pub fn ingest_line(
        &mut self,
        peer: PeerCredentials,
        peer_start_ticks: u64,
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
        if peer_start_ticks != binding.start_ticks {
            return Err(IngestDenial::PeerStartTimeMismatch);
        }
        let sample: Sample = serde_json::from_str(line).map_err(|_| IngestDenial::BadJson)?;
        let spec = self
            .contract
            .spec(&sample.metric)
            .ok_or(IngestDenial::UnknownMetric)?;
        if !sample.value.is_finite() {
            return Err(IngestDenial::NonFinite);
        }
        if sample.value < 0.0 {
            return Err(IngestDenial::NegativeValue);
        }
        if sample.population == 0 {
            return Err(IngestDenial::EmptyPopulation);
        }
        if sample.kind == SampleKind::RawEvent && sample.population != 1 {
            return Err(IngestDenial::RawEventPopulation);
        }
        if sample.monotonic_ns > now_ns.saturating_add(MAX_FUTURE_SKEW_NS) {
            return Err(IngestDenial::FutureTimestamp);
        }
        if now_ns.saturating_sub(sample.monotonic_ns) > spec.max_age_ms.saturating_mul(1_000_000) {
            return Err(IngestDenial::TooOld);
        }
        let series = self.series.entry(sample.metric.clone()).or_default();
        if series
            .samples
            .back()
            .is_some_and(|last| sample.monotonic_ns <= last.monotonic_ns)
        {
            return Err(IngestDenial::NotAfterPrevious);
        }
        if series.samples.len() == MAX_SAMPLES_PER_METRIC
            && let Some(dropped) = series.samples.pop_front()
        {
            series.evicted_through_ns = Some(dropped.monotonic_ns);
        }
        series.samples.push_back(sample);
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
        let series = self.series.get(metric)?;
        if now_ns.saturating_sub(series.samples.back()?.monotonic_ns)
            > spec.max_age_ms.saturating_mul(1_000_000)
        {
            return None;
        }
        let inside: Vec<&Sample> = series
            .samples
            .iter()
            .filter(|sample| (from_ns..=to_ns).contains(&sample.monotonic_ns))
            .collect();
        let population: u64 = inside.iter().map(|sample| sample.population).sum();
        if inside.is_empty() || population < spec.min_population {
            return None;
        }
        let (value, semantics) = aggregate(spec, &inside);
        let complete = series
            .evicted_through_ns
            .is_none_or(|evicted| evicted < from_ns);
        Some(WindowAggregate {
            value,
            samples: inside.len(),
            population,
            semantics,
            complete,
        })
    }
}

fn aggregate(spec: &MetricSpec, samples: &[&Sample]) -> (f64, Semantics) {
    let all_raw = samples
        .iter()
        .all(|sample| sample.kind == SampleKind::RawEvent);
    let rank = |q: f64| {
        let mut values: Vec<f64> = samples.iter().map(|sample| sample.value).collect();
        values.sort_by(f64::total_cmp);
        let index = ((q * values.len() as f64).ceil() as usize).clamp(1, values.len());
        values[index - 1]
    };
    let quantile = |q: f64| {
        (
            rank(q),
            if all_raw {
                Semantics::Direct
            } else {
                Semantics::QuantileOfSummaries
            },
        )
    };
    let weight: f64 = samples.iter().map(|sample| sample.population as f64).sum();
    match spec.aggregation {
        // A population-weighted mean of summaries equals the mean of the underlying events.
        Aggregation::Mean => (
            samples
                .iter()
                .map(|sample| sample.value * sample.population as f64)
                .sum::<f64>()
                / weight,
            Semantics::Direct,
        ),
        Aggregation::Sum => (
            samples.iter().map(|sample| sample.value).sum(),
            Semantics::Direct,
        ),
        Aggregation::Max => (
            samples
                .iter()
                .map(|sample| sample.value)
                .fold(f64::MIN, f64::max),
            if all_raw {
                Semantics::Direct
            } else {
                Semantics::QuantileOfSummaries
            },
        ),
        Aggregation::Last => (
            samples.last().map_or(0.0, |sample| sample.value),
            Semantics::Direct,
        ),
        Aggregation::P50 => quantile(0.50),
        Aggregation::P95 => quantile(0.95),
        Aggregation::P99 => quantile(0.99),
    }
}
