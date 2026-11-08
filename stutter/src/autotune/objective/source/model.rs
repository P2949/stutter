use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    LowerIsBetter,
    HigherIsBetter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    Mean,
    P50,
    P95,
    P99,
    Max,
    Sum,
    Last,
}

/// What kind of quantity a metric is; latency and throughput are never interchangeable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricClass {
    Latency,
    Throughput,
    AudioDeadline,
    Interactivity,
    IoLatency,
    NetworkRequestLatency,
    EnergyPerformance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSpec {
    pub id: String,
    pub unit: String,
    pub class: MetricClass,
    pub direction: Direction,
    pub aggregation: Aggregation,
    /// A window whose newest sample is older than this is stale.
    pub max_age_ms: u64,
    /// Fewest underlying events a window must represent.
    pub min_population: u64,
    pub desired_value: Option<f64>,
    pub meaningful_effect_percent: f64,
    pub equivalent_within_percent: f64,
}

/// What a sample stands for. Only raw events support an exact percentile of the stream.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleKind {
    /// One underlying event (a request, a frame).
    RawEvent,
    /// A summary of `population` events (a batch mean or batch percentile). The default, because
    /// it is the weaker claim.
    #[default]
    Summary,
}

/// One sample as the application reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub metric: String,
    pub value: f64,
    /// The reporter's monotonic clock, in nanoseconds; must not go backwards.
    pub monotonic_ns: u64,
    pub work_unit: Option<String>,
    /// Underlying events this sample summarizes.
    pub population: u64,
    #[serde(default)]
    pub kind: SampleKind,
}

/// The one process this source is bound to, by start time so a reused pid never matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBinding {
    pub uid: u32,
    pub pid: u32,
    pub start_ticks: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KpiContract {
    pub binding: SourceBinding,
    pub primary: MetricSpec,
    /// Violating one of these invalidates a candidate.
    pub hard_guardrails: Vec<MetricSpec>,
    /// Reported, never decisive.
    pub advisory_guardrails: Vec<MetricSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractError {
    DuplicateMetric(String),
    BadSpec(String, &'static str),
    NoHardGuardrail,
}

fn check_spec(spec: &MetricSpec) -> Result<(), ContractError> {
    let bad = |why| Err(ContractError::BadSpec(spec.id.clone(), why));
    if spec.id.trim().is_empty() || spec.unit.trim().is_empty() {
        return bad("id and unit must not be empty");
    }
    if spec.max_age_ms == 0 || spec.min_population == 0 {
        return bad("max_age_ms and min_population must be positive");
    }
    let finite = [
        spec.meaningful_effect_percent,
        spec.equivalent_within_percent,
    ]
    .iter()
    .all(|value| value.is_finite() && *value > 0.0)
        && spec.desired_value.is_none_or(f64::is_finite);
    if !finite {
        return bad("effect sizes must be finite and positive");
    }
    if spec.meaningful_effect_percent <= spec.equivalent_within_percent {
        return bad("the meaningful effect must exceed the equivalence margin");
    }
    Ok(())
}

impl KpiContract {
    pub fn validate(&self) -> Result<(), ContractError> {
        let mut seen = BTreeSet::new();
        let all = std::iter::once(&self.primary)
            .chain(&self.hard_guardrails)
            .chain(&self.advisory_guardrails);
        for spec in all {
            check_spec(spec)?;
            if !seen.insert(spec.id.clone()) {
                return Err(ContractError::DuplicateMetric(spec.id.clone()));
            }
        }
        if self.hard_guardrails.is_empty() {
            return Err(ContractError::NoHardGuardrail);
        }
        Ok(())
    }

    pub fn spec(&self, metric: &str) -> Option<&MetricSpec> {
        std::iter::once(&self.primary)
            .chain(&self.hard_guardrails)
            .chain(&self.advisory_guardrails)
            .find(|spec| spec.id == metric)
    }

    /// A registered KPI never authorizes an operation: support comes only from the registry.
    pub const fn authorizes_mutation(&self) -> bool {
        false
    }
}
