//! Versioned experimental KPI contract templates for workloads whose direct signal is the
//! application's own report: low-latency audio (xruns and deadline misses) and browser or desktop
//! interactivity (input-to-response time).
//!
//! A template is a typed starting point, not a calibration: every template is `Experimental`, its
//! effect-size thresholds are declared not fitted, and `authorizes_mutation` stays false. A direct
//! xrun or response measurement still has to come from the application over the local KPI socket,
//! bound to the one verified process; nothing here derives either from CPU use or from a generic
//! foreground heuristic.
use serde::{Deserialize, Serialize};

use super::{Aggregation, Direction, KpiContract, MetricClass, MetricSpec, SourceBinding};

pub const TEMPLATE_VERSION: &str = "experimental-1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentalTemplate {
    pub id: &'static str,
    pub version: &'static str,
    pub contract: KpiContract,
}

fn spec(
    id: &str,
    unit: &str,
    class: MetricClass,
    aggregation: Aggregation,
    max_age_ms: u64,
    min_population: u64,
) -> MetricSpec {
    MetricSpec {
        id: id.to_owned(),
        unit: unit.to_owned(),
        class,
        direction: Direction::LowerIsBetter,
        aggregation,
        max_age_ms,
        min_population,
        desired_value: None,
        meaningful_effect_percent: 20.0,
        equivalent_within_percent: 5.0,
    }
}

/// Audio: the primary is the audio callback's deadline misses per window; xruns reported by the
/// audio server are a hard guardrail, so a candidate that trades a few misses for dropouts fails.
pub fn audio_xrun(binding: SourceBinding) -> ExperimentalTemplate {
    ExperimentalTemplate {
        id: "audio_xrun",
        version: TEMPLATE_VERSION,
        contract: KpiContract {
            binding,
            primary: spec(
                "deadline_miss_count",
                "count",
                MetricClass::AudioDeadline,
                Aggregation::Sum,
                5_000,
                1_000,
            ),
            hard_guardrails: vec![spec(
                "xrun_count",
                "count",
                MetricClass::AudioDeadline,
                Aggregation::Sum,
                5_000,
                1_000,
            )],
            advisory_guardrails: vec![spec(
                "callback_max_ms",
                "ms",
                MetricClass::Latency,
                Aggregation::Max,
                5_000,
                1_000,
            )],
        },
    }
}

/// Browser or desktop interactivity: the primary is the input-to-response time's tail; long main
/// thread tasks are a hard guardrail.
pub fn browser_response(binding: SourceBinding) -> ExperimentalTemplate {
    ExperimentalTemplate {
        id: "browser_response",
        version: TEMPLATE_VERSION,
        contract: KpiContract {
            binding,
            primary: spec(
                "input_to_response_p95_ms",
                "ms",
                MetricClass::Interactivity,
                Aggregation::P95,
                10_000,
                50,
            ),
            hard_guardrails: vec![spec(
                "long_task_count",
                "count",
                MetricClass::Interactivity,
                Aggregation::Sum,
                10_000,
                50,
            )],
            advisory_guardrails: vec![spec(
                "frame_drop_count",
                "count",
                MetricClass::Interactivity,
                Aggregation::Sum,
                10_000,
                50,
            )],
        },
    }
}

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;
